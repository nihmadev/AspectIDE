#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{
    collections::{BTreeSet, HashSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    time::Instant,
};

use lux_core::{
    AppError, AppResult, ExtensionActivated, ExtensionActivationBlocked,
    ExtensionActivationCandidate, ExtensionActivationFailed, ExtensionActivationPlan,
    ExtensionActivationReport, ExtensionCommandContribution, ExtensionCommandExecution,
    ExtensionCommandExecutionPhase, ExtensionCommandExecutionStatus, ExtensionCommandRoute,
    ExtensionContributionKind, ExtensionContributionPoint, ExtensionContributionRegistration,
    ExtensionContributionRegistry, ExtensionContributionUnavailable,
    ExtensionHostActivationContract, ExtensionHostLimits, ExtensionHostPermission, ExtensionInfo,
    ExtensionManifest, ExtensionStatus, ExtensionWasmAbi, ExtensionWasmImport,
    ExtensionWasmImportKind, ExtensionWasmPreflight,
};
use wasmparser::{Encoding, ExternalKind, Parser, Payload, TypeRef, Validator};
use wasmtime::{
    Config, Engine, Instance, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, Trap,
};

const MANIFEST_FILE: &str = "lux-extension.json";
const MAX_CONTRIBUTION_POINTS: usize = 128;
const MAX_WASM_MODULE_BYTES: u64 = 32 * 1024 * 1024;
const WASM_MAGIC_AND_VERSION: [u8; 8] = [0x00, b'a', b's', b'm', 0x01, 0x00, 0x00, 0x00];
const LUX_EXTENSION_ABI_VERSION: u32 = 1;
const LUX_EXTENSION_ENTRYPOINT: &str = "lux_extension_init";
const LUX_EXTENSION_OPTIONAL_EXPORTS: &[&str] = &["lux_extension_shutdown"];
const LUX_HOST_IMPORT_MODULE: &str = "lux:extension/host@1";
const EXTENSION_HOST_MAX_MEMORY_PAGES: u32 = 256;
const EXTENSION_HOST_ACTIVATION_TIMEOUT_MS: u64 = 5_000;
const EXTENSION_HOST_MAX_OUTPUT_BYTES: u64 = 1024 * 1024;
const EXTENSION_HOST_ACTIVATION_FUEL: u64 = 250_000;
const EXTENSION_WASM_EXECUTION_EXHAUSTED_FUEL: &str = "extension WASM execution exhausted fuel";
const WASM_PAGE_BYTES: usize = 65_536;

#[derive(Debug, Clone, Copy)]
struct HostImportSpec {
    name: &'static str,
    permission: Option<ExtensionHostPermission>,
}

#[derive(Debug, Clone, Copy)]
struct ExtensionExportExecution {
    fuel_consumed: u64,
    fuel_remaining: u64,
}

#[derive(Debug)]
struct ExtensionExportFailure {
    error: AppError,
    execution: Option<ExtensionExportExecution>,
}

struct ExtensionRuntime {
    store: Store<StoreLimits>,
    instance: Instance,
}

impl ExtensionExportFailure {
    const fn without_execution(error: AppError) -> Self {
        Self {
            error,
            execution: None,
        }
    }

    const fn with_execution(error: AppError, execution: ExtensionExportExecution) -> Self {
        Self {
            error,
            execution: Some(execution),
        }
    }
}

impl ExtensionRuntime {
    fn instantiate(
        candidate: &ExtensionActivationCandidate,
    ) -> Result<Self, ExtensionExportFailure> {
        let bytes = fs::read(&candidate.wasm_preflight.module_path)
            .map_err(AppError::from)
            .map_err(ExtensionExportFailure::without_execution)?;
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config)
            .map_err(|error| ExtensionExportFailure::without_execution(wasmtime_error(&error)))?;
        let memory_limit = memory_limit_bytes(candidate.host_contract.limits.max_memory_pages)
            .map_err(ExtensionExportFailure::without_execution)?;
        let limits = StoreLimitsBuilder::new()
            .memory_size(memory_limit)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(&engine, limits);
        store.limiter(|limits| limits);
        store
            .set_fuel(EXTENSION_HOST_ACTIVATION_FUEL)
            .map_err(|error| ExtensionExportFailure::without_execution(wasmtime_error(&error)))?;

        let module = Module::new(&engine, bytes.as_slice())
            .map_err(|error| ExtensionExportFailure::without_execution(wasmtime_error(&error)))?;
        let mut linker = Linker::new(&engine);
        define_host_imports(&mut linker, &candidate.host_contract)
            .map_err(ExtensionExportFailure::without_execution)?;

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|error| export_failure_from_initial_fuel(wasmtime_error(&error), &store))?;

        Ok(Self { store, instance })
    }

    fn call_export(
        &mut self,
        export_name: &str,
    ) -> Result<ExtensionExportExecution, ExtensionExportFailure> {
        let fuel_before = current_fuel(&self.store)?;
        let entrypoint = match self
            .instance
            .get_typed_func::<(), ()>(&mut self.store, export_name)
        {
            Ok(entrypoint) => entrypoint,
            Err(error) => {
                return Err(export_failure_since(
                    wasmtime_error(&error),
                    &self.store,
                    fuel_before,
                ));
            }
        };
        if let Err(error) = entrypoint.call(&mut self.store, ()) {
            return Err(export_failure_since(
                wasmtime_error(&error),
                &self.store,
                fuel_before,
            ));
        }

        export_execution_since(&self.store, fuel_before).ok_or_else(|| {
            ExtensionExportFailure::without_execution(AppError::Service(
                "extension WASM fuel accounting is unavailable".into(),
            ))
        })
    }
}

const ALLOWED_HOST_IMPORTS: &[HostImportSpec] = &[
    HostImportSpec {
        name: "log",
        permission: None,
    },
    HostImportSpec {
        name: "workspace_read",
        permission: Some(ExtensionHostPermission::WorkspaceRead),
    },
    HostImportSpec {
        name: "workspace_write",
        permission: Some(ExtensionHostPermission::WorkspaceWrite),
    },
    HostImportSpec {
        name: "network_fetch",
        permission: Some(ExtensionHostPermission::NetworkAccess),
    },
    HostImportSpec {
        name: "process_spawn",
        permission: Some(ExtensionHostPermission::ProcessSpawn),
    },
];

pub fn discover_extensions(root: impl AsRef<Path>) -> AppResult<Vec<ExtensionInfo>> {
    discover_extensions_in_roots([root.as_ref()])
}

pub fn discover_extensions_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> AppResult<Vec<ExtensionInfo>> {
    let mut extensions = Vec::new();
    let mut seen_ids = HashSet::new();

    for root in roots {
        let root = root.as_ref();
        if !root.exists() {
            continue;
        }

        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if !file_type.is_dir() {
                continue;
            }

            let extension_root = entry.path();
            let manifest_path = extension_root.join(MANIFEST_FILE);
            if !manifest_path.exists() {
                continue;
            }

            let extension = read_extension_info(extension_root, manifest_path);
            if seen_ids.insert(extension.id.clone()) {
                extensions.push(extension);
            }
        }
    }

    extensions.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(extensions)
}

pub fn extension_activation_plan(root: impl AsRef<Path>) -> AppResult<ExtensionActivationPlan> {
    build_activation_plan(discover_extensions(root)?)
}

pub fn activate_extensions(root: impl AsRef<Path>) -> AppResult<ExtensionActivationReport> {
    activate_extension_plan(extension_activation_plan(root)?)
}

pub fn extension_contribution_registry(
    root: impl AsRef<Path>,
) -> AppResult<ExtensionContributionRegistry> {
    Ok(build_contribution_registry(activate_extensions(root)?))
}

pub fn extension_command_routes(root: impl AsRef<Path>) -> AppResult<Vec<ExtensionCommandRoute>> {
    let plan = extension_activation_plan(root)?;
    let routes = command_routes_for_activation_plan(&plan);
    validate_unique_command_routes(&routes)?;
    Ok(routes)
}

pub fn execute_extension_command(
    root: impl AsRef<Path>,
    command_id: &str,
) -> AppResult<ExtensionCommandExecution> {
    execute_extension_command_in_roots([root.as_ref()], command_id)
}

pub fn extension_activation_plan_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> AppResult<ExtensionActivationPlan> {
    build_activation_plan(discover_extensions_in_roots(roots)?)
}

pub fn activate_extensions_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> AppResult<ExtensionActivationReport> {
    activate_extension_plan(extension_activation_plan_in_roots(roots)?)
}

pub fn extension_contribution_registry_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> AppResult<ExtensionContributionRegistry> {
    Ok(build_contribution_registry(activate_extensions_in_roots(
        roots,
    )?))
}

pub fn extension_command_routes_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> AppResult<Vec<ExtensionCommandRoute>> {
    let plan = extension_activation_plan_in_roots(roots)?;
    let routes = command_routes_for_activation_plan(&plan);
    validate_unique_command_routes(&routes)?;
    Ok(routes)
}

pub fn execute_extension_command_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
    command_id: &str,
) -> AppResult<ExtensionCommandExecution> {
    let plan = extension_activation_plan_in_roots(roots)?;
    Ok(execute_extension_command_from_plan(&plan, command_id))
}

pub fn activate_extension_plan(
    plan: ExtensionActivationPlan,
) -> AppResult<ExtensionActivationReport> {
    let mut activated = Vec::new();
    let mut failed = Vec::new();

    for candidate in &plan.candidates {
        match activate_extension_candidate(candidate) {
            Ok(result) => activated.push(result),
            Err(error) => failed.push(ExtensionActivationFailed {
                id: candidate.id.clone(),
                name: candidate.name.clone(),
                version: candidate.version.clone(),
                root: candidate.root.clone(),
                wasm_module: candidate.wasm_module.clone(),
                reason: activation_failure_reason(&error),
            }),
        }
    }

    activated.sort_by(|left, right| left.id.cmp(&right.id));
    failed.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(ExtensionActivationReport {
        plan,
        activated,
        failed,
    })
}

pub fn build_activation_plan(extensions: Vec<ExtensionInfo>) -> AppResult<ExtensionActivationPlan> {
    let mut candidates = Vec::new();
    let mut blocked = Vec::new();

    for extension in extensions {
        if extension.status != ExtensionStatus::Discovered {
            blocked.push(blocked_extension(
                &extension,
                extension
                    .error
                    .as_deref()
                    .unwrap_or("extension manifest is not valid"),
            ));
            continue;
        }

        let unknown_contributions = extension
            .contribution_points
            .iter()
            .filter(|point| point.kind == ExtensionContributionKind::Unknown)
            .map(|point| point.id.as_str())
            .collect::<Vec<_>>();
        if !unknown_contributions.is_empty() {
            blocked.push(blocked_extension(
                &extension,
                &format!(
                    "unsupported contribution points: {}",
                    unknown_contributions.join(", ")
                ),
            ));
            continue;
        }

        let wasm_preflight = match validate_wasm_preflight(&extension) {
            Ok(preflight) => preflight,
            Err(error) => {
                blocked.push(blocked_extension(&extension, &error.to_string()));
                continue;
            }
        };
        let host_contract = match validate_wasm_host_contract(&extension, &wasm_preflight) {
            Ok(contract) => contract,
            Err(error) => {
                blocked.push(blocked_extension(&extension, &error.to_string()));
                continue;
            }
        };

        candidates.push(ExtensionActivationCandidate {
            id: extension.id,
            name: extension.name,
            version: extension.version,
            root: extension.root,
            manifest_path: extension.manifest_path,
            wasm_module: extension.wasm_module,
            contribution_points: extension.contribution_points,
            commands: extension.commands,
            wasm_preflight,
            host_contract,
        });
    }

    candidates.sort_by(|left, right| left.id.cmp(&right.id));
    blocked.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(ExtensionActivationPlan {
        candidates,
        blocked,
    })
}

pub fn validate_manifest(manifest: &ExtensionManifest, extension_root: &Path) -> AppResult<()> {
    validate_extension_id(&manifest.id)?;
    validate_non_empty_field("extension name", &manifest.name)?;
    validate_non_empty_field("extension version", &manifest.version)?;
    validate_wasm_module_path(&manifest.wasm_module)?;
    validate_contributes(&manifest.contributes)?;
    validate_commands(manifest)?;

    let wasm_path = extension_root.join(&manifest.wasm_module);
    if !wasm_path.is_file() {
        return Err(AppError::NotFound(format!(
            "WASM module does not exist: {}",
            wasm_path.display()
        )));
    }
    Ok(())
}

fn validate_extension_id(id: &str) -> AppResult<()> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(AppError::Service("extension id cannot be empty".into()));
    }
    if trimmed != id {
        return Err(AppError::Service(
            "extension id cannot contain leading or trailing whitespace".into(),
        ));
    }
    if !trimmed.contains('.') {
        return Err(AppError::Service(
            "extension id must use a reverse-DNS style namespace".into(),
        ));
    }
    if !trimmed.bytes().all(|byte| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || byte == b'.'
            || byte == b'-'
            || byte == b'_'
    }) {
        return Err(AppError::Service(
            "extension id may only contain lowercase ASCII letters, digits, '.', '-' and '_'"
                .into(),
        ));
    }
    if trimmed.split('.').any(str::is_empty) {
        return Err(AppError::Service(
            "extension id namespace segments cannot be empty".into(),
        ));
    }
    Ok(())
}

fn validate_non_empty_field(label: &str, value: &str) -> AppResult<()> {
    if value.trim().is_empty() {
        return Err(AppError::Service(format!("{label} cannot be empty")));
    }
    Ok(())
}

fn validate_wasm_module_path(path: &Path) -> AppResult<()> {
    if path.as_os_str().is_empty() {
        return Err(AppError::Service("WASM module path cannot be empty".into()));
    }
    if path.is_absolute() {
        return Err(AppError::Service(
            "WASM module path must be relative to the extension root".into(),
        ));
    }
    if path.extension().and_then(std::ffi::OsStr::to_str) != Some("wasm") {
        return Err(AppError::Service(
            "WASM module path must point to a .wasm file".into(),
        ));
    }
    if path.components().any(is_forbidden_relative_component) {
        return Err(AppError::Service(
            "WASM module path cannot escape the extension root".into(),
        ));
    }
    Ok(())
}

fn validate_contributes(contributes: &[String]) -> AppResult<()> {
    if contributes.len() > MAX_CONTRIBUTION_POINTS {
        return Err(AppError::Service(format!(
            "extension contributes too many contribution points: {} > {MAX_CONTRIBUTION_POINTS}",
            contributes.len()
        )));
    }

    for contribution in contributes {
        let id = contribution.trim();
        if id.is_empty() {
            continue;
        }
        if id != contribution {
            return Err(AppError::Service(
                "contribution point ids cannot contain leading or trailing whitespace".into(),
            ));
        }
        if !id.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-' || byte == b'_'
        }) {
            return Err(AppError::Service(format!(
                "invalid contribution point id: {id}"
            )));
        }
    }

    Ok(())
}

fn validate_commands(manifest: &ExtensionManifest) -> AppResult<()> {
    if manifest.commands.is_empty() {
        return Ok(());
    }

    if !manifest.contributes.iter().any(|point| point == "commands") {
        return Err(AppError::Service(
            "extension commands require the commands contribution point".into(),
        ));
    }

    for command in &manifest.commands {
        validate_extension_command(manifest, command)?;
    }

    let mut command_ids = BTreeSet::new();
    for command in &manifest.commands {
        if !command_ids.insert(command.id.as_str()) {
            return Err(AppError::Service(format!(
                "duplicate extension command id: {}",
                command.id
            )));
        }
    }

    Ok(())
}

fn validate_extension_command(
    manifest: &ExtensionManifest,
    command: &ExtensionCommandContribution,
) -> AppResult<()> {
    validate_command_id(&manifest.id, &command.id)?;
    validate_non_empty_field("extension command title", &command.title)?;
    if let Some(category) = &command.category {
        validate_non_empty_field("extension command category", category)?;
    }
    validate_handler_export_name(&command.handler)?;
    Ok(())
}

fn validate_command_id(extension_id: &str, command_id: &str) -> AppResult<()> {
    let trimmed = command_id.trim();
    if trimmed.is_empty() {
        return Err(AppError::Service(
            "extension command id cannot be empty".into(),
        ));
    }
    if trimmed != command_id {
        return Err(AppError::Service(
            "extension command id cannot contain leading or trailing whitespace".into(),
        ));
    }
    if !trimmed.starts_with(extension_id) {
        return Err(AppError::Service(format!(
            "extension command id must start with extension id namespace: {extension_id}"
        )));
    }
    if !trimmed.bytes().all(|byte| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || byte == b'.'
            || byte == b'-'
            || byte == b'_'
    }) {
        return Err(AppError::Service(
            "extension command id may only contain lowercase ASCII letters, digits, '.', '-' and '_'"
                .into(),
        ));
    }
    Ok(())
}

fn validate_handler_export_name(handler: &str) -> AppResult<()> {
    let trimmed = handler.trim();
    if trimmed.is_empty() {
        return Err(AppError::Service(
            "extension command handler cannot be empty".into(),
        ));
    }
    if trimmed != handler {
        return Err(AppError::Service(
            "extension command handler cannot contain leading or trailing whitespace".into(),
        ));
    }
    if handler == LUX_EXTENSION_ENTRYPOINT || LUX_EXTENSION_OPTIONAL_EXPORTS.contains(&handler) {
        return Err(AppError::Service(format!(
            "extension command handler uses reserved Lux export: {handler}"
        )));
    }
    if !handler
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.' || byte == b'-')
    {
        return Err(AppError::Service(format!(
            "invalid extension command handler export: {handler}"
        )));
    }
    Ok(())
}

const fn is_forbidden_relative_component(component: Component<'_>) -> bool {
    matches!(
        component,
        Component::ParentDir | Component::RootDir | Component::Prefix(_)
    )
}

fn read_extension_info(extension_root: PathBuf, manifest_path: PathBuf) -> ExtensionInfo {
    match read_manifest(&manifest_path) {
        Ok(manifest) => match validate_manifest(&manifest, &extension_root) {
            Ok(()) => manifest_to_info(
                extension_root,
                manifest_path,
                manifest,
                ExtensionStatus::Discovered,
                None,
            ),
            Err(error) => manifest_to_info(
                extension_root,
                manifest_path,
                manifest,
                ExtensionStatus::Invalid,
                Some(error.to_string()),
            ),
        },
        Err(error) => ExtensionInfo {
            id: extension_root.file_name().map_or_else(
                || "invalid-extension".to_string(),
                |value| value.to_string_lossy().to_string(),
            ),
            name: extension_root.file_name().map_or_else(
                || "Invalid extension".to_string(),
                |value| value.to_string_lossy().to_string(),
            ),
            version: "0.0.0".to_string(),
            wasm_module: PathBuf::new(),
            permissions: Vec::new(),
            contributes: Vec::new(),
            contribution_points: Vec::new(),
            commands: Vec::new(),
            status: ExtensionStatus::Invalid,
            error: Some(error.to_string()),
            root: extension_root,
            manifest_path,
        },
    }
}

fn read_manifest(path: &Path) -> AppResult<ExtensionManifest> {
    let contents = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&contents)?)
}

fn manifest_to_info(
    extension_root: PathBuf,
    manifest_path: PathBuf,
    manifest: ExtensionManifest,
    status: ExtensionStatus,
    error: Option<String>,
) -> ExtensionInfo {
    let contribution_points = contribution_points_for_manifest(&manifest);
    ExtensionInfo {
        id: manifest.id,
        name: manifest.name,
        version: manifest.version,
        wasm_module: extension_root.join(manifest.wasm_module),
        permissions: manifest.permissions,
        contributes: manifest.contributes,
        contribution_points,
        commands: manifest.commands,
        status,
        error,
        root: extension_root,
        manifest_path,
    }
}

fn blocked_extension(extension: &ExtensionInfo, reason: &str) -> ExtensionActivationBlocked {
    ExtensionActivationBlocked {
        id: extension.id.clone(),
        name: extension.name.clone(),
        version: extension.version.clone(),
        root: extension.root.clone(),
        manifest_path: extension.manifest_path.clone(),
        wasm_module: extension.wasm_module.clone(),
        contribution_points: extension.contribution_points.clone(),
        commands: extension.commands.clone(),
        reason: reason.to_string(),
    }
}

#[must_use]
pub fn build_contribution_registry(
    activation: ExtensionActivationReport,
) -> ExtensionContributionRegistry {
    let mut registered = Vec::new();
    let mut unavailable = Vec::new();

    for activated in &activation.activated {
        if let Some(candidate) = activation
            .plan
            .candidates
            .iter()
            .find(|candidate| candidate.id == activated.id)
        {
            registered.extend(
                candidate
                    .contribution_points
                    .iter()
                    .cloned()
                    .map(|contribution| ExtensionContributionRegistration {
                        extension_id: candidate.id.clone(),
                        extension_name: candidate.name.clone(),
                        extension_version: candidate.version.clone(),
                        contribution,
                    }),
            );
        }
    }

    for failed in &activation.failed {
        if let Some(candidate) = activation
            .plan
            .candidates
            .iter()
            .find(|candidate| candidate.id == failed.id)
        {
            unavailable.extend(
                candidate
                    .contribution_points
                    .iter()
                    .cloned()
                    .map(|contribution| ExtensionContributionUnavailable {
                        extension_id: candidate.id.clone(),
                        extension_name: candidate.name.clone(),
                        extension_version: candidate.version.clone(),
                        contribution,
                        reason: failed.reason.clone(),
                    }),
            );
        }
    }

    for blocked in &activation.plan.blocked {
        unavailable.extend(
            blocked
                .contribution_points
                .iter()
                .cloned()
                .map(|contribution| ExtensionContributionUnavailable {
                    extension_id: blocked.id.clone(),
                    extension_name: blocked.name.clone(),
                    extension_version: blocked.version.clone(),
                    contribution,
                    reason: blocked.reason.clone(),
                }),
        );
    }

    registered.sort_by(|left, right| {
        left.contribution
            .id
            .cmp(&right.contribution.id)
            .then_with(|| left.extension_id.cmp(&right.extension_id))
    });
    unavailable.sort_by(|left, right| {
        left.contribution
            .id
            .cmp(&right.contribution.id)
            .then_with(|| left.extension_id.cmp(&right.extension_id))
    });

    ExtensionContributionRegistry {
        activation,
        registered,
        unavailable,
    }
}

#[must_use]
pub fn command_routes_for_activation_plan(
    plan: &ExtensionActivationPlan,
) -> Vec<ExtensionCommandRoute> {
    let mut routes = Vec::new();
    for candidate in &plan.candidates {
        routes.extend(
            candidate
                .commands
                .iter()
                .map(|command| ExtensionCommandRoute {
                    id: command.id.clone(),
                    title: command.title.clone(),
                    category: command.category.clone(),
                    handler: command.handler.clone(),
                    extension_id: candidate.id.clone(),
                    extension_name: candidate.name.clone(),
                    extension_version: candidate.version.clone(),
                }),
        );
    }

    routes.sort_by(|left, right| left.id.cmp(&right.id));
    routes
}

fn execute_extension_command_from_plan(
    plan: &ExtensionActivationPlan,
    command_id: &str,
) -> ExtensionCommandExecution {
    let started_at = Instant::now();
    let routes = command_routes_for_activation_plan(plan);
    if let Err(error) = validate_unique_command_routes(&routes) {
        return failed_command_execution(
            command_id,
            None,
            ExtensionCommandExecutionPhase::Routing,
            activation_failure_reason(&error),
            started_at,
            None,
            None,
        );
    }
    let Some(route) = routes.into_iter().find(|route| route.id == command_id) else {
        return failed_command_execution(
            command_id,
            None,
            ExtensionCommandExecutionPhase::Routing,
            format!("extension command is not registered: {command_id}"),
            started_at,
            None,
            None,
        );
    };
    let Some(candidate) = plan
        .candidates
        .iter()
        .find(|candidate| candidate.id == route.extension_id)
    else {
        return failed_command_execution(
            command_id,
            Some(route),
            ExtensionCommandExecutionPhase::Routing,
            format!("extension command route lost activation candidate: {command_id}"),
            started_at,
            None,
            None,
        );
    };

    execute_extension_command_candidate(candidate, command_id, route, started_at)
}

fn execute_extension_command_candidate(
    candidate: &ExtensionActivationCandidate,
    command_id: &str,
    route: ExtensionCommandRoute,
    started_at: Instant,
) -> ExtensionCommandExecution {
    let mut runtime = match ExtensionRuntime::instantiate(candidate) {
        Ok(runtime) => runtime,
        Err(failure) => {
            return failed_command_execution(
                command_id,
                Some(route),
                ExtensionCommandExecutionPhase::Activation,
                execution_failure_reason(
                    &failure.error,
                    ExtensionCommandExecutionPhase::Activation,
                ),
                started_at,
                failure.execution,
                None,
            );
        }
    };

    let activation_execution = match runtime.call_export(&candidate.host_contract.abi.entrypoint) {
        Ok(execution) => execution,
        Err(failure) => {
            return failed_command_execution(
                command_id,
                Some(route),
                ExtensionCommandExecutionPhase::Activation,
                execution_failure_reason(
                    &failure.error,
                    ExtensionCommandExecutionPhase::Activation,
                ),
                started_at,
                failure.execution,
                None,
            );
        }
    };

    match runtime.call_export(&route.handler) {
        Ok(handler_execution) => succeeded_command_execution(
            command_id,
            route,
            activation_execution,
            handler_execution,
            started_at,
        ),
        Err(failure) => failed_command_execution(
            command_id,
            Some(route),
            ExtensionCommandExecutionPhase::Handler,
            execution_failure_reason(&failure.error, ExtensionCommandExecutionPhase::Handler),
            started_at,
            Some(activation_execution),
            failure.execution,
        ),
    }
}

fn succeeded_command_execution(
    command_id: &str,
    route: ExtensionCommandRoute,
    activation_execution: ExtensionExportExecution,
    handler_execution: ExtensionExportExecution,
    started_at: Instant,
) -> ExtensionCommandExecution {
    ExtensionCommandExecution {
        command_id: command_id.to_owned(),
        route: Some(route),
        status: ExtensionCommandExecutionStatus::Succeeded,
        phase: ExtensionCommandExecutionPhase::Handler,
        reason: None,
        duration_ms: elapsed_ms(started_at),
        activation_fuel_consumed: Some(activation_execution.fuel_consumed),
        activation_fuel_remaining: Some(activation_execution.fuel_remaining),
        handler_fuel_consumed: Some(handler_execution.fuel_consumed),
        handler_fuel_remaining: Some(handler_execution.fuel_remaining),
    }
}

fn failed_command_execution(
    command_id: &str,
    route: Option<ExtensionCommandRoute>,
    phase: ExtensionCommandExecutionPhase,
    reason: String,
    started_at: Instant,
    activation_execution: Option<ExtensionExportExecution>,
    handler_execution: Option<ExtensionExportExecution>,
) -> ExtensionCommandExecution {
    ExtensionCommandExecution {
        command_id: command_id.to_owned(),
        route,
        status: ExtensionCommandExecutionStatus::Failed,
        phase,
        reason: Some(reason),
        duration_ms: elapsed_ms(started_at),
        activation_fuel_consumed: activation_execution
            .as_ref()
            .map(|execution| execution.fuel_consumed),
        activation_fuel_remaining: activation_execution
            .as_ref()
            .map(|execution| execution.fuel_remaining),
        handler_fuel_consumed: handler_execution
            .as_ref()
            .map(|execution| execution.fuel_consumed),
        handler_fuel_remaining: handler_execution
            .as_ref()
            .map(|execution| execution.fuel_remaining),
    }
}

fn elapsed_ms(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn validate_unique_command_routes(routes: &[ExtensionCommandRoute]) -> AppResult<()> {
    let mut route_ids = BTreeSet::new();
    for route in routes {
        if !route_ids.insert(route.id.as_str()) {
            return Err(AppError::Service(format!(
                "duplicate registered extension command id: {}",
                route.id
            )));
        }
    }
    Ok(())
}

fn validate_wasm_preflight(extension: &ExtensionInfo) -> AppResult<ExtensionWasmPreflight> {
    let root = extension.root.canonicalize()?;
    let module_path = extension.wasm_module.canonicalize()?;
    if !module_path.starts_with(&root) {
        return Err(AppError::Service(format!(
            "WASM module escapes extension root: {}",
            module_path.display()
        )));
    }

    let metadata = fs::metadata(&module_path)?;
    if !metadata.is_file() {
        return Err(AppError::Service(format!(
            "WASM module is not a file: {}",
            module_path.display()
        )));
    }
    if metadata.len() > MAX_WASM_MODULE_BYTES {
        return Err(AppError::Service(format!(
            "WASM module is too large: {} bytes > {MAX_WASM_MODULE_BYTES}",
            metadata.len()
        )));
    }

    let mut header = [0_u8; 8];
    let mut file = fs::File::open(&module_path)?;
    file.read_exact(&mut header)?;
    if header != WASM_MAGIC_AND_VERSION {
        return Err(AppError::Service(format!(
            "WASM module has invalid magic or version: {}",
            module_path.display()
        )));
    }

    Ok(ExtensionWasmPreflight {
        module_path,
        size_bytes: metadata.len(),
    })
}

fn activate_extension_candidate(
    candidate: &ExtensionActivationCandidate,
) -> AppResult<ExtensionActivated> {
    let execution = run_extension_export(candidate, &candidate.host_contract.abi.entrypoint)
        .map_err(|failure| failure.error)?;
    Ok(ExtensionActivated {
        id: candidate.id.clone(),
        name: candidate.name.clone(),
        version: candidate.version.clone(),
        root: candidate.root.clone(),
        wasm_module: candidate.wasm_module.clone(),
        fuel_consumed: execution.fuel_consumed,
        fuel_remaining: execution.fuel_remaining,
    })
}

fn run_extension_export(
    candidate: &ExtensionActivationCandidate,
    export_name: &str,
) -> Result<ExtensionExportExecution, ExtensionExportFailure> {
    let mut runtime = ExtensionRuntime::instantiate(candidate)?;
    runtime.call_export(export_name)
}

fn export_failure_from_initial_fuel(
    error: AppError,
    store: &Store<StoreLimits>,
) -> ExtensionExportFailure {
    match export_execution_since(store, EXTENSION_HOST_ACTIVATION_FUEL) {
        Some(execution) => ExtensionExportFailure::with_execution(error, execution),
        None => ExtensionExportFailure::without_execution(error),
    }
}

fn export_failure_since(
    error: AppError,
    store: &Store<StoreLimits>,
    fuel_before: u64,
) -> ExtensionExportFailure {
    match export_execution_since(store, fuel_before) {
        Some(execution) => ExtensionExportFailure::with_execution(error, execution),
        None => ExtensionExportFailure::without_execution(error),
    }
}

fn current_fuel(store: &Store<StoreLimits>) -> Result<u64, ExtensionExportFailure> {
    store
        .get_fuel()
        .map_err(|error| ExtensionExportFailure::without_execution(wasmtime_error(&error)))
}

fn export_execution_since(
    store: &Store<StoreLimits>,
    fuel_before: u64,
) -> Option<ExtensionExportExecution> {
    store
        .get_fuel()
        .ok()
        .map(|fuel_remaining| ExtensionExportExecution {
            fuel_consumed: fuel_before.saturating_sub(fuel_remaining),
            fuel_remaining,
        })
}

fn define_host_imports(
    linker: &mut Linker<StoreLimits>,
    contract: &ExtensionHostActivationContract,
) -> AppResult<()> {
    for import in &contract.abi.imports {
        if import.module != LUX_HOST_IMPORT_MODULE
            || import.kind != ExtensionWasmImportKind::Function
        {
            return Err(AppError::Service(format!(
                "runtime refused unsupported host import: {}.{}",
                import.module, import.name
            )));
        }

        match import.name.as_str() {
            "log" => linker.func_wrap(LUX_HOST_IMPORT_MODULE, "log", || ()),
            "workspace_read" => {
                ensure_permission(
                    contract,
                    ExtensionHostPermission::WorkspaceRead,
                    "workspace_read",
                )?;
                linker.func_wrap(LUX_HOST_IMPORT_MODULE, "workspace_read", deny_host_io)
            }
            "workspace_write" => {
                ensure_permission(
                    contract,
                    ExtensionHostPermission::WorkspaceWrite,
                    "workspace_write",
                )?;
                linker.func_wrap(LUX_HOST_IMPORT_MODULE, "workspace_write", deny_host_io)
            }
            "network_fetch" => {
                ensure_permission(
                    contract,
                    ExtensionHostPermission::NetworkAccess,
                    "network_fetch",
                )?;
                linker.func_wrap(LUX_HOST_IMPORT_MODULE, "network_fetch", deny_host_io)
            }
            "process_spawn" => {
                ensure_permission(
                    contract,
                    ExtensionHostPermission::ProcessSpawn,
                    "process_spawn",
                )?;
                linker.func_wrap(LUX_HOST_IMPORT_MODULE, "process_spawn", deny_host_io)
            }
            name => {
                return Err(AppError::Service(format!(
                    "runtime refused unknown Lux host import: {name}"
                )));
            }
        }
        .map_err(|error| AppError::Service(format!("extension WASM linker error: {error}")))?;
    }

    Ok(())
}

fn deny_host_io() -> wasmtime::Result<()> {
    Err(wasmtime::format_err!(
        "Lux host IO imports are unavailable during extension activation"
    ))
}

fn ensure_permission(
    contract: &ExtensionHostActivationContract,
    permission: ExtensionHostPermission,
    import_name: &str,
) -> AppResult<()> {
    if contract.permissions.contains(&permission) {
        return Ok(());
    }

    Err(AppError::Service(format!(
        "runtime refused {import_name} without manifest permission {permission:?}"
    )))
}

fn memory_limit_bytes(max_pages: u32) -> AppResult<usize> {
    usize::try_from(max_pages)
        .ok()
        .and_then(|pages| pages.checked_mul(WASM_PAGE_BYTES))
        .ok_or_else(|| AppError::Service("extension memory limit overflows usize".into()))
}

fn activation_failure_reason(error: &AppError) -> String {
    match error {
        AppError::Service(reason) if reason == EXTENSION_WASM_EXECUTION_EXHAUSTED_FUEL => {
            "extension activation exhausted fuel".into()
        }
        AppError::Service(reason) => reason.clone(),
        error => error.to_string(),
    }
}

fn execution_failure_reason(error: &AppError, phase: ExtensionCommandExecutionPhase) -> String {
    match (phase, error) {
        (ExtensionCommandExecutionPhase::Activation, AppError::Service(reason))
            if reason == EXTENSION_WASM_EXECUTION_EXHAUSTED_FUEL =>
        {
            "extension activation exhausted fuel".into()
        }
        (ExtensionCommandExecutionPhase::Handler, AppError::Service(reason))
            if reason == EXTENSION_WASM_EXECUTION_EXHAUSTED_FUEL =>
        {
            "extension command handler exhausted fuel".into()
        }
        (_, AppError::Service(reason)) => reason.clone(),
        (_, error) => error.to_string(),
    }
}

fn wasmtime_error(error: &wasmtime::Error) -> AppError {
    if let Some(trap) = error.downcast_ref::<Trap>() {
        return match trap {
            Trap::OutOfFuel => AppError::Service(EXTENSION_WASM_EXECUTION_EXHAUSTED_FUEL.into()),
            trap => AppError::Service(format!("extension WASM runtime trap: {trap:?}")),
        };
    }

    let message = error.to_string();
    AppError::Service(format!("extension WASM runtime error: {message}"))
}

fn validate_wasm_host_contract(
    extension: &ExtensionInfo,
    preflight: &ExtensionWasmPreflight,
) -> AppResult<ExtensionHostActivationContract> {
    let bytes = fs::read(&preflight.module_path)?;
    Validator::new()
        .validate_all(&bytes)
        .map_err(|error| AppError::Service(error.to_string()))?;

    let mut exported_entrypoint = false;
    let mut exports_memory = false;
    let mut imports = Vec::new();
    let mut exported_functions = Vec::new();

    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.map_err(|error| AppError::Service(error.to_string()))? {
            Payload::Version { encoding, .. } => {
                if encoding != Encoding::Module {
                    return Err(AppError::Service(
                        "extension WASM must be a core module, not a component".into(),
                    ));
                }
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import.map_err(|error| AppError::Service(error.to_string()))?;
                    validate_host_import(extension, import.module, import.name, import.ty)?;
                    imports.push(ExtensionWasmImport {
                        module: import.module.to_string(),
                        name: import.name.to_string(),
                        kind: import_kind(import.ty),
                    });
                }
            }
            Payload::MemorySection(reader) => {
                for memory in reader {
                    let memory = memory.map_err(|error| AppError::Service(error.to_string()))?;
                    validate_memory_limit(memory.initial, memory.maximum)?;
                }
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export.map_err(|error| AppError::Service(error.to_string()))?;
                    if export.name == LUX_EXTENSION_ENTRYPOINT {
                        if export.kind != ExternalKind::Func {
                            return Err(AppError::Service(format!(
                                "required export {LUX_EXTENSION_ENTRYPOINT} must be a function"
                            )));
                        }
                        exported_entrypoint = true;
                    }
                    if export.kind == ExternalKind::Func {
                        exported_functions.push(export.name.to_string());
                    }
                    if export.name == "memory" {
                        if export.kind != ExternalKind::Memory {
                            return Err(AppError::Service(
                                "export named memory must be a WebAssembly memory".into(),
                            ));
                        }
                        exports_memory = true;
                    }
                }
            }
            _ => {}
        }
    }

    if !exported_entrypoint {
        return Err(AppError::Service(format!(
            "WASM module must export required Lux extension entrypoint: {LUX_EXTENSION_ENTRYPOINT}"
        )));
    }
    validate_command_handler_exports(extension, &exported_functions)?;

    imports.sort_by(|left, right| {
        left.module
            .cmp(&right.module)
            .then_with(|| left.name.cmp(&right.name))
    });

    Ok(ExtensionHostActivationContract {
        abi: ExtensionWasmAbi {
            version: LUX_EXTENSION_ABI_VERSION,
            entrypoint: LUX_EXTENSION_ENTRYPOINT.to_string(),
            required_exports: vec![LUX_EXTENSION_ENTRYPOINT.to_string()],
            optional_exports: LUX_EXTENSION_OPTIONAL_EXPORTS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            imports,
            exports_memory,
        },
        permissions: extension.permissions.clone(),
        limits: ExtensionHostLimits {
            max_memory_pages: EXTENSION_HOST_MAX_MEMORY_PAGES,
            activation_timeout_ms: EXTENSION_HOST_ACTIVATION_TIMEOUT_MS,
            max_output_bytes: EXTENSION_HOST_MAX_OUTPUT_BYTES,
        },
    })
}

fn validate_command_handler_exports(
    extension: &ExtensionInfo,
    exported_functions: &[String],
) -> AppResult<()> {
    for command in &extension.commands {
        if !exported_functions
            .iter()
            .any(|export| export == &command.handler)
        {
            return Err(AppError::Service(format!(
                "extension command {} references missing WASM handler export: {}",
                command.id, command.handler
            )));
        }
    }
    Ok(())
}

fn validate_host_import(
    extension: &ExtensionInfo,
    module: &str,
    name: &str,
    ty: TypeRef,
) -> AppResult<()> {
    if module != LUX_HOST_IMPORT_MODULE {
        return Err(AppError::Service(format!(
            "unsupported WASM import module: {module}.{name}"
        )));
    }
    if !matches!(ty, TypeRef::Func(_) | TypeRef::FuncExact(_)) {
        return Err(AppError::Service(format!(
            "Lux host import must be a function: {module}.{name}"
        )));
    }

    let Some(spec) = ALLOWED_HOST_IMPORTS
        .iter()
        .find(|candidate| candidate.name == name)
    else {
        return Err(AppError::Service(format!(
            "unsupported Lux host import: {module}.{name}"
        )));
    };

    if let Some(permission) = spec.permission {
        if !extension.permissions.contains(&permission) {
            return Err(AppError::Service(format!(
                "WASM import {module}.{name} requires manifest permission {permission:?}"
            )));
        }
    }

    Ok(())
}

fn validate_memory_limit(initial: u64, maximum: Option<u64>) -> AppResult<()> {
    if initial > u64::from(EXTENSION_HOST_MAX_MEMORY_PAGES) {
        return Err(AppError::Service(format!(
            "WASM memory initial pages exceed host limit: {initial} > {EXTENSION_HOST_MAX_MEMORY_PAGES}"
        )));
    }
    if let Some(maximum) = maximum {
        if maximum > u64::from(EXTENSION_HOST_MAX_MEMORY_PAGES) {
            return Err(AppError::Service(format!(
                "WASM memory maximum pages exceed host limit: {maximum} > {EXTENSION_HOST_MAX_MEMORY_PAGES}"
            )));
        }
    }
    Ok(())
}

const fn import_kind(ty: TypeRef) -> ExtensionWasmImportKind {
    match ty {
        TypeRef::Func(_) | TypeRef::FuncExact(_) => ExtensionWasmImportKind::Function,
        TypeRef::Table(_) => ExtensionWasmImportKind::Table,
        TypeRef::Memory(_) => ExtensionWasmImportKind::Memory,
        TypeRef::Global(_) => ExtensionWasmImportKind::Global,
        TypeRef::Tag(_) => ExtensionWasmImportKind::Tag,
    }
}

#[must_use]
pub fn contribution_points_for_manifest(
    manifest: &ExtensionManifest,
) -> Vec<ExtensionContributionPoint> {
    let mut points = manifest
        .contributes
        .iter()
        .filter_map(|value| contribution_point(value))
        .collect::<Vec<_>>();
    points.sort_by(|left, right| left.id.cmp(&right.id));
    points.dedup_by(|left, right| left.id == right.id);
    points
}

fn contribution_point(value: &str) -> Option<ExtensionContributionPoint> {
    let id = value.trim();
    if id.is_empty() {
        return None;
    }

    Some(ExtensionContributionPoint {
        id: id.to_string(),
        kind: contribution_kind(id),
    })
}

fn contribution_kind(id: &str) -> ExtensionContributionKind {
    match id {
        "commands" => ExtensionContributionKind::Commands,
        "themes" => ExtensionContributionKind::Themes,
        "keybindings" => ExtensionContributionKind::Keybindings,
        "languages" => ExtensionContributionKind::Languages,
        "grammars" => ExtensionContributionKind::Grammars,
        "snippets" => ExtensionContributionKind::Snippets,
        "views" => ExtensionContributionKind::Views,
        "menus" => ExtensionContributionKind::Menus,
        "settings" | "configuration" => ExtensionContributionKind::Settings,
        "debuggers" => ExtensionContributionKind::Debuggers,
        "tasks" => ExtensionContributionKind::Tasks,
        "problemMatchers" => ExtensionContributionKind::ProblemMatchers,
        _ => ExtensionContributionKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn contribution_points_are_deduplicated_and_classified() {
        let manifest = ExtensionManifest {
            id: "lux.rust".to_string(),
            name: "Rust Tools".to_string(),
            version: "0.1.0".to_string(),
            wasm_module: PathBuf::from("extension.wasm"),
            permissions: Vec::new(),
            contributes: vec![
                "commands".to_string(),
                "languages".to_string(),
                "commands".to_string(),
                "custom.point".to_string(),
                " ".to_string(),
            ],
            commands: Vec::new(),
        };

        let points = contribution_points_for_manifest(&manifest);

        assert_eq!(points.len(), 3);
        assert_eq!(
            points[0],
            ExtensionContributionPoint {
                id: "commands".to_string(),
                kind: ExtensionContributionKind::Commands,
            }
        );
        assert_eq!(
            points[1],
            ExtensionContributionPoint {
                id: "custom.point".to_string(),
                kind: ExtensionContributionKind::Unknown,
            }
        );
        assert_eq!(
            points[2],
            ExtensionContributionPoint {
                id: "languages".to_string(),
                kind: ExtensionContributionKind::Languages,
            }
        );
    }

    #[test]
    fn discover_extensions_marks_valid_manifests_as_discovered_not_active() {
        let root = unique_temp_dir("lux-extension-discovery");
        let extension_root = root.join("rust-tools");
        fs::create_dir_all(&extension_root).expect("extension root should be created");
        fs::write(extension_root.join("extension.wasm"), minimal_lux_wasm())
            .expect("wasm module should be written");
        fs::write(
            extension_root.join(MANIFEST_FILE),
            r#"{
                "id": "lux.rust",
                "name": "Rust Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands", "themes"]
            }"#,
        )
        .expect("manifest should be written");

        let extensions = discover_extensions(&root).expect("extensions should be discovered");

        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0].status, ExtensionStatus::Discovered);
        assert_eq!(extensions[0].contribution_points.len(), 2);
        assert_eq!(
            extensions[0].contribution_points[0].kind,
            ExtensionContributionKind::Commands
        );
        assert_eq!(
            extensions[0].contribution_points[1].kind,
            ExtensionContributionKind::Themes
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn discover_extensions_keeps_manifest_contributions_for_invalid_extensions() {
        let root = unique_temp_dir("lux-extension-invalid");
        let extension_root = root.join("broken-tools");
        fs::create_dir_all(&extension_root).expect("extension root should be created");
        fs::write(
            extension_root.join(MANIFEST_FILE),
            r#"{
                "id": "lux.broken",
                "name": "Broken Tools",
                "version": "0.1.0",
                "wasm_module": "missing.wasm",
                "contributes": ["debuggers"]
            }"#,
        )
        .expect("manifest should be written");

        let extensions = discover_extensions(&root).expect("extensions should be discovered");

        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0].status, ExtensionStatus::Invalid);
        assert_eq!(
            extensions[0].contribution_points[0].kind,
            ExtensionContributionKind::Debuggers
        );
        assert!(extensions[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("WASM module does not exist"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn activation_plan_includes_only_valid_extensions_with_supported_contributions() {
        let root = unique_temp_dir("lux-extension-activation-plan");
        write_extension(
            &root,
            "rust-tools",
            r#"{
                "id": "lux.rust",
                "name": "Rust Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands", "languages"]
            }"#,
            true,
        );
        write_extension(
            &root,
            "unknown-tools",
            r#"{
                "id": "lux.unknown",
                "name": "Unknown Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands", "custom.point"]
            }"#,
            true,
        );
        write_extension(
            &root,
            "broken-tools",
            r#"{
                "id": "lux.broken",
                "name": "Broken Tools",
                "version": "0.1.0",
                "wasm_module": "missing.wasm",
                "contributes": ["commands"]
            }"#,
            false,
        );

        let plan = extension_activation_plan(&root).expect("activation plan should be built");

        assert_eq!(plan.candidates.len(), 1);
        assert_eq!(plan.candidates[0].id, "lux.rust");
        assert_eq!(plan.candidates[0].contribution_points.len(), 2);
        assert_eq!(
            plan.candidates[0].host_contract.abi.entrypoint,
            LUX_EXTENSION_ENTRYPOINT
        );
        assert_eq!(plan.candidates[0].host_contract.abi.version, 1);
        assert!(plan.candidates[0].host_contract.permissions.is_empty());
        assert_eq!(
            plan.candidates[0].host_contract.limits.max_memory_pages,
            EXTENSION_HOST_MAX_MEMORY_PAGES
        );
        assert!(plan.candidates[0]
            .contribution_points
            .iter()
            .all(|point| point.kind != ExtensionContributionKind::Unknown));

        assert_eq!(plan.blocked.len(), 2);
        assert!(plan.blocked.iter().any(|blocked| {
            blocked.id == "lux.broken" && blocked.reason.contains("WASM module does not exist")
        }));
        assert!(plan.blocked.iter().any(|blocked| {
            blocked.id == "lux.unknown"
                && blocked
                    .reason
                    .contains("unsupported contribution points: custom.point")
        }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn activation_plan_blocks_wasm_without_required_lux_entrypoint() {
        let root = unique_temp_dir("lux-extension-missing-entrypoint");
        write_extension(
            &root,
            "no-entrypoint",
            r#"{
                "id": "lux.no_entrypoint",
                "name": "No Entrypoint",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands"]
            }"#,
            false,
        );
        fs::write(
            root.join("no-entrypoint").join("extension.wasm"),
            empty_wasm_module(),
        )
        .expect("wasm module should be written");

        let plan = extension_activation_plan(&root).expect("activation plan should be built");

        assert!(plan.candidates.is_empty());
        assert_eq!(plan.blocked.len(), 1);
        assert!(plan.blocked[0]
            .reason
            .contains("must export required Lux extension entrypoint"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn activation_plan_records_allowed_imports_and_manifest_permissions() {
        let root = unique_temp_dir("lux-extension-import-contract");
        write_extension(
            &root,
            "workspace-tools",
            r#"{
                "id": "lux.workspace_tools",
                "name": "Workspace Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "permissions": ["workspaceRead"],
                "contributes": ["commands"]
            }"#,
            false,
        );
        fs::write(
            root.join("workspace-tools").join("extension.wasm"),
            lux_wasm_with_host_import("workspace_read"),
        )
        .expect("wasm module should be written");

        let plan = extension_activation_plan(&root).expect("activation plan should be built");

        assert_eq!(plan.candidates.len(), 1);
        assert!(plan.blocked.is_empty());
        assert_eq!(
            plan.candidates[0].host_contract.permissions,
            vec![ExtensionHostPermission::WorkspaceRead]
        );
        assert_eq!(plan.candidates[0].host_contract.abi.imports.len(), 1);
        assert_eq!(
            plan.candidates[0].host_contract.abi.imports[0].module,
            LUX_HOST_IMPORT_MODULE
        );
        assert_eq!(
            plan.candidates[0].host_contract.abi.imports[0].name,
            "workspace_read"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn activation_plan_blocks_forbidden_or_undeclared_host_imports() {
        let root = unique_temp_dir("lux-extension-import-blocks");
        write_extension(
            &root,
            "undeclared-permission",
            r#"{
                "id": "lux.undeclared_permission",
                "name": "Undeclared Permission",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands"]
            }"#,
            false,
        );
        fs::write(
            root.join("undeclared-permission").join("extension.wasm"),
            lux_wasm_with_host_import("workspace_write"),
        )
        .expect("wasm module should be written");

        write_extension(
            &root,
            "unknown-module",
            r#"{
                "id": "lux.unknown_module",
                "name": "Unknown Module",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands"]
            }"#,
            false,
        );
        fs::write(
            root.join("unknown-module").join("extension.wasm"),
            wasm_with_import("env", "hostcall"),
        )
        .expect("wasm module should be written");

        let plan = extension_activation_plan(&root).expect("activation plan should be built");

        assert!(plan.candidates.is_empty());
        assert_eq!(plan.blocked.len(), 2);
        assert!(plan.blocked.iter().any(|blocked| {
            blocked.id == "lux.undeclared_permission"
                && blocked.reason.contains("requires manifest permission")
        }));
        assert!(plan.blocked.iter().any(|blocked| {
            blocked.id == "lux.unknown_module"
                && blocked.reason.contains("unsupported WASM import module")
        }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn activation_plan_blocks_wasm_modules_that_fail_preflight() {
        let root = unique_temp_dir("lux-extension-wasm-preflight");
        write_extension(
            &root,
            "bad-header",
            r#"{
                "id": "lux.bad_header",
                "name": "Bad Header",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands"]
            }"#,
            false,
        );
        fs::write(root.join("bad-header").join("extension.wasm"), b"not-wasm")
            .expect("bad wasm should be written");

        write_extension(
            &root,
            "too-large",
            r#"{
                "id": "lux.too_large",
                "name": "Too Large",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands"]
            }"#,
            false,
        );
        let too_large = root.join("too-large").join("extension.wasm");
        fs::write(&too_large, WASM_MAGIC_AND_VERSION).expect("wasm header should be written");
        let file = fs::OpenOptions::new()
            .write(true)
            .open(&too_large)
            .expect("wasm file should open");
        file.set_len(MAX_WASM_MODULE_BYTES + 1)
            .expect("wasm file size should be expanded");

        let plan = extension_activation_plan(&root).expect("activation plan should be built");

        assert!(plan.candidates.is_empty());
        assert_eq!(plan.blocked.len(), 2);
        assert!(plan.blocked.iter().any(|blocked| {
            blocked.id == "lux.bad_header" && blocked.reason.contains("invalid magic or version")
        }));
        assert!(plan.blocked.iter().any(|blocked| {
            blocked.id == "lux.too_large" && blocked.reason.contains("WASM module is too large")
        }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn activation_runtime_activates_valid_candidates_and_preserves_blocked_plan_entries() {
        let root = unique_temp_dir("lux-extension-runtime-success");
        write_extension(
            &root,
            "runtime-tools",
            r#"{
                "id": "lux.runtime_tools",
                "name": "Runtime Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands"]
            }"#,
            true,
        );
        write_extension(
            &root,
            "blocked-tools",
            r#"{
                "id": "lux.blocked_tools",
                "name": "Blocked Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["custom.point"]
            }"#,
            true,
        );

        let report = activate_extensions(&root).expect("extensions should activate");

        assert_eq!(report.activated.len(), 1);
        assert_eq!(report.activated[0].id, "lux.runtime_tools");
        assert!(report.activated[0].fuel_consumed > 0);
        assert!(report.failed.is_empty());
        assert_eq!(report.plan.blocked.len(), 1);
        assert_eq!(report.plan.blocked[0].id, "lux.blocked_tools");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn contribution_registry_registers_only_activated_extension_contributions() {
        let root = unique_temp_dir("lux-extension-contribution-registry");
        write_extension(
            &root,
            "runtime-tools",
            r#"{
                "id": "lux.runtime_tools",
                "name": "Runtime Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands", "languages"]
            }"#,
            true,
        );
        write_extension(
            &root,
            "blocked-tools",
            r#"{
                "id": "lux.blocked_tools",
                "name": "Blocked Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["custom.point"]
            }"#,
            true,
        );
        write_extension(
            &root,
            "loop-tools",
            r#"{
                "id": "lux.loop_tools",
                "name": "Loop Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["debuggers"]
            }"#,
            false,
        );
        fs::write(
            root.join("loop-tools").join("extension.wasm"),
            loop_lux_wasm(),
        )
        .expect("loop wasm should be written");

        let registry =
            extension_contribution_registry(&root).expect("contribution registry should be built");

        assert_eq!(registry.activation.activated.len(), 1);
        assert_eq!(registry.registered.len(), 2);
        assert_eq!(registry.registered[0].contribution.id, "commands");
        assert_eq!(registry.registered[1].contribution.id, "languages");
        assert!(registry
            .registered
            .iter()
            .all(|entry| entry.extension_id == "lux.runtime_tools"));

        assert_eq!(registry.unavailable.len(), 2);
        assert!(registry.unavailable.iter().any(|entry| {
            entry.extension_id == "lux.blocked_tools"
                && entry.contribution.id == "custom.point"
                && entry.reason.contains("unsupported contribution points")
        }));
        assert!(registry.unavailable.iter().any(|entry| {
            entry.extension_id == "lux.loop_tools"
                && entry.contribution.id == "debuggers"
                && entry.reason == "extension activation exhausted fuel"
        }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn command_routes_validate_manifest_and_execute_explicit_handlers() {
        let root = unique_temp_dir("lux-extension-command-routes");
        write_extension(
            &root,
            "command-tools",
            r#"{
                "id": "lux.command_tools",
                "name": "Command Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands"],
                "commands": [{
                    "id": "lux.command_tools.say_hello",
                    "title": "Say Hello",
                    "category": "Lux Test",
                    "handler": "lux_command_say_hello"
                }]
            }"#,
            false,
        );
        fs::write(
            root.join("command-tools").join("extension.wasm"),
            command_lux_wasm("lux_command_say_hello", &[0x0b]),
        )
        .expect("command wasm should be written");

        let routes = extension_command_routes(&root).expect("command routes should be built");

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].id, "lux.command_tools.say_hello");
        assert_eq!(routes[0].handler, "lux_command_say_hello");
        assert_eq!(routes[0].category.as_deref(), Some("Lux Test"));

        let execution = execute_extension_command(&root, "lux.command_tools.say_hello")
            .expect("extension command should execute");

        assert_eq!(execution.command_id, "lux.command_tools.say_hello");
        assert_eq!(execution.status, ExtensionCommandExecutionStatus::Succeeded);
        assert_eq!(execution.phase, ExtensionCommandExecutionPhase::Handler);
        assert!(execution.reason.is_none());
        assert_eq!(
            execution
                .route
                .as_ref()
                .expect("successful execution should keep route")
                .id,
            "lux.command_tools.say_hello"
        );
        assert!(execution
            .activation_fuel_consumed
            .is_some_and(|fuel| fuel > 0));
        assert!(execution.handler_fuel_consumed.is_some_and(|fuel| fuel > 0));

        let missing = execute_extension_command(&root, "lux.command_tools.missing")
            .expect("missing command should return typed execution report");
        assert_eq!(missing.status, ExtensionCommandExecutionStatus::Failed);
        assert_eq!(missing.phase, ExtensionCommandExecutionPhase::Routing);
        assert!(missing.route.is_none());
        assert!(missing
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("not registered")));

        fs::write(
            root.join("command-tools").join("extension.wasm"),
            command_lux_wasm("lux_command_say_hello", &[0x00, 0x0b]),
        )
        .expect("trap command wasm should be written");

        let failed = execute_extension_command(&root, "lux.command_tools.say_hello")
            .expect("handler trap should return typed execution report");
        assert_eq!(failed.status, ExtensionCommandExecutionStatus::Failed);
        assert_eq!(failed.phase, ExtensionCommandExecutionPhase::Handler);
        assert!(failed.route.is_some());
        assert!(failed.activation_fuel_consumed.is_some_and(|fuel| fuel > 0));
        assert!(failed.handler_fuel_consumed.is_some_and(|fuel| fuel > 0));
        assert!(failed
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("runtime trap")));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn command_execution_runs_activation_and_handler_in_one_instance() {
        let root = unique_temp_dir("lux-extension-command-stateful");
        write_extension(
            &root,
            "command-tools",
            r#"{
                "id": "lux.command_tools",
                "name": "Command Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands"],
                "commands": [{
                    "id": "lux.command_tools.check_state",
                    "title": "Check State",
                    "handler": "lux_command_check_state"
                }]
            }"#,
            false,
        );
        fs::write(
            root.join("command-tools").join("extension.wasm"),
            stateful_command_lux_wasm("lux_command_check_state"),
        )
        .expect("stateful command wasm should be written");

        let execution = execute_extension_command(&root, "lux.command_tools.check_state")
            .expect("stateful command should execute");

        assert_eq!(execution.status, ExtensionCommandExecutionStatus::Succeeded);
        assert_eq!(execution.phase, ExtensionCommandExecutionPhase::Handler);
        assert!(execution.reason.is_none());
        assert!(execution
            .activation_fuel_consumed
            .is_some_and(|fuel| fuel > 0));
        assert!(execution.handler_fuel_consumed.is_some_and(|fuel| fuel > 0));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn activation_plan_blocks_command_contributions_with_missing_handlers() {
        let root = unique_temp_dir("lux-extension-command-missing-handler");
        write_extension(
            &root,
            "command-tools",
            r#"{
                "id": "lux.command_tools",
                "name": "Command Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands"],
                "commands": [{
                    "id": "lux.command_tools.missing",
                    "title": "Missing Handler",
                    "handler": "lux_command_missing"
                }]
            }"#,
            true,
        );

        let plan = extension_activation_plan(&root).expect("activation plan should be built");

        assert!(plan.candidates.is_empty());
        assert_eq!(plan.blocked.len(), 1);
        assert!(plan.blocked[0]
            .reason
            .contains("references missing WASM handler export"));
        assert_eq!(plan.blocked[0].commands.len(), 1);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn activation_runtime_reports_traps_without_activating_candidate() {
        let root = unique_temp_dir("lux-extension-runtime-trap");
        write_extension(
            &root,
            "trap-tools",
            r#"{
                "id": "lux.trap_tools",
                "name": "Trap Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands"]
            }"#,
            false,
        );
        fs::write(
            root.join("trap-tools").join("extension.wasm"),
            trap_lux_wasm(),
        )
        .expect("trap wasm should be written");

        let report = activate_extensions(&root).expect("activation report should be built");

        assert!(report.activated.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].id, "lux.trap_tools");
        assert!(report.failed[0]
            .reason
            .contains("extension WASM runtime trap: UnreachableCodeReached"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn activation_runtime_enforces_fuel_limits() {
        let root = unique_temp_dir("lux-extension-runtime-fuel");
        write_extension(
            &root,
            "loop-tools",
            r#"{
                "id": "lux.loop_tools",
                "name": "Loop Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands"]
            }"#,
            false,
        );
        fs::write(
            root.join("loop-tools").join("extension.wasm"),
            loop_lux_wasm(),
        )
        .expect("loop wasm should be written");

        let report = activate_extensions(&root).expect("activation report should be built");

        assert!(report.activated.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].id, "lux.loop_tools");
        assert_eq!(
            report.failed[0].reason,
            "extension activation exhausted fuel"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn validate_manifest_rejects_wasm_paths_that_escape_extension_root() {
        let root = unique_temp_dir("lux-extension-traversal");
        fs::create_dir_all(&root).expect("extension root should be created");
        let manifest = manifest_with_wasm(PathBuf::from("../escape.wasm"));

        let error = validate_manifest(&manifest, &root).expect_err("traversal path should fail");

        assert!(error
            .to_string()
            .contains("WASM module path cannot escape the extension root"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn validate_manifest_rejects_non_wasm_modules_and_directories() {
        let root = unique_temp_dir("lux-extension-non-wasm");
        fs::create_dir_all(root.join("module.wasm")).expect("directory should be created");
        let directory_manifest = manifest_with_wasm(PathBuf::from("module.wasm"));

        let directory_error = validate_manifest(&directory_manifest, &root)
            .expect_err("directory should not satisfy wasm module validation");

        assert!(directory_error
            .to_string()
            .contains("WASM module does not exist"));

        let text_manifest = manifest_with_wasm(PathBuf::from("module.txt"));
        let extension_error = validate_manifest(&text_manifest, &root)
            .expect_err("non-wasm extension should fail before existence check");

        assert!(extension_error
            .to_string()
            .contains("WASM module path must point to a .wasm file"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn validate_manifest_rejects_unsafe_extension_ids_and_contribution_ids() {
        let root = unique_temp_dir("lux-extension-ids");
        fs::create_dir_all(&root).expect("extension root should be created");
        fs::write(root.join("extension.wasm"), minimal_lux_wasm()).expect("wasm should be written");

        let mut bad_extension_id = manifest_with_wasm(PathBuf::from("extension.wasm"));
        bad_extension_id.id = "lux.Rust".to_string();
        let id_error = validate_manifest(&bad_extension_id, &root)
            .expect_err("unsafe extension id should fail");
        assert!(id_error
            .to_string()
            .contains("extension id may only contain lowercase ASCII"));

        let mut bad_contribution_id = manifest_with_wasm(PathBuf::from("extension.wasm"));
        bad_contribution_id.contributes = vec![" commands".to_string()];
        let contribution_error = validate_manifest(&bad_contribution_id, &root)
            .expect_err("unsafe contribution id should fail");
        assert!(contribution_error
            .to_string()
            .contains("contribution point ids cannot contain"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn validate_manifest_caps_contribution_points() {
        let root = unique_temp_dir("lux-extension-many-contributions");
        fs::create_dir_all(&root).expect("extension root should be created");
        fs::write(root.join("extension.wasm"), minimal_lux_wasm()).expect("wasm should be written");
        let mut manifest = manifest_with_wasm(PathBuf::from("extension.wasm"));
        manifest.contributes = (0..=MAX_CONTRIBUTION_POINTS)
            .map(|index| format!("custom.point.{index}"))
            .collect();

        let error = validate_manifest(&manifest, &root)
            .expect_err("too many contribution points should fail");

        assert!(error
            .to_string()
            .contains("extension contributes too many contribution points"));
        let _ = fs::remove_dir_all(root);
    }

    fn manifest_with_wasm(wasm_module: PathBuf) -> ExtensionManifest {
        ExtensionManifest {
            id: "lux.test".to_string(),
            name: "Test Extension".to_string(),
            version: "0.1.0".to_string(),
            wasm_module,
            permissions: Vec::new(),
            contributes: vec!["commands".to_string()],
            commands: Vec::new(),
        }
    }

    fn write_extension(root: &Path, name: &str, manifest: &str, write_wasm: bool) {
        let extension_root = root.join(name);
        fs::create_dir_all(&extension_root).expect("extension root should be created");
        if write_wasm {
            fs::write(extension_root.join("extension.wasm"), minimal_lux_wasm())
                .expect("wasm module should be written");
        }
        fs::write(extension_root.join(MANIFEST_FILE), manifest)
            .expect("manifest should be written");
    }

    fn empty_wasm_module() -> Vec<u8> {
        WASM_MAGIC_AND_VERSION.to_vec()
    }

    fn minimal_lux_wasm() -> Vec<u8> {
        lux_wasm_with_body(&[0x0b])
    }

    fn trap_lux_wasm() -> Vec<u8> {
        lux_wasm_with_body(&[0x00, 0x0b])
    }

    fn loop_lux_wasm() -> Vec<u8> {
        lux_wasm_with_body(&[0x03, 0x40, 0x0c, 0x00, 0x0b, 0x0b])
    }

    fn command_lux_wasm(handler: &str, handler_body: &[u8]) -> Vec<u8> {
        let mut wasm = WASM_MAGIC_AND_VERSION.to_vec();
        push_type_section(&mut wasm, 1);
        push_function_section_many(&mut wasm, &[0, 0]);
        push_export_section_many(&mut wasm, &[(LUX_EXTENSION_ENTRYPOINT, 0), (handler, 1)]);
        push_code_section(&mut wasm, &[&[0x0b], handler_body]);
        wasm
    }

    fn stateful_command_lux_wasm(handler: &str) -> Vec<u8> {
        let mut wasm = WASM_MAGIC_AND_VERSION.to_vec();
        push_type_section(&mut wasm, 1);
        push_function_section_many(&mut wasm, &[0, 0]);
        push_global_section(&mut wasm);
        push_export_section_many(&mut wasm, &[(LUX_EXTENSION_ENTRYPOINT, 0), (handler, 1)]);
        push_code_section(
            &mut wasm,
            &[
                &[0x41, 0x01, 0x24, 0x00, 0x0b],
                &[
                    0x23, 0x00, 0x41, 0x01, 0x46, 0x04, 0x40, 0x0f, 0x0b, 0x00, 0x0b,
                ],
            ],
        );
        wasm
    }

    fn lux_wasm_with_body(body: &[u8]) -> Vec<u8> {
        let mut wasm = WASM_MAGIC_AND_VERSION.to_vec();
        push_type_section(&mut wasm, 1);
        push_function_section(&mut wasm, 0);
        push_export_section(&mut wasm, LUX_EXTENSION_ENTRYPOINT, 0);
        push_code_section(&mut wasm, &[body]);
        wasm
    }

    fn lux_wasm_with_host_import(name: &str) -> Vec<u8> {
        wasm_with_import(LUX_HOST_IMPORT_MODULE, name)
    }

    fn wasm_with_import(module: &str, name: &str) -> Vec<u8> {
        let mut wasm = WASM_MAGIC_AND_VERSION.to_vec();
        push_type_section(&mut wasm, 2);
        push_import_section(&mut wasm, module, name);
        push_function_section(&mut wasm, 1);
        push_export_section(&mut wasm, LUX_EXTENSION_ENTRYPOINT, 1);
        push_code_section(&mut wasm, &[&[0x0b]]);
        wasm
    }

    fn push_type_section(wasm: &mut Vec<u8>, count: u8) {
        let mut payload = Vec::new();
        payload.push(count);
        for _ in 0..count {
            payload.extend_from_slice(&[0x60, 0x00, 0x00]);
        }
        push_section(wasm, 1, &payload);
    }

    fn push_import_section(wasm: &mut Vec<u8>, module: &str, name: &str) {
        let mut payload = Vec::new();
        payload.push(0x01);
        push_name(&mut payload, module);
        push_name(&mut payload, name);
        payload.push(0x00);
        payload.push(0x00);
        push_section(wasm, 2, &payload);
    }

    fn push_function_section(wasm: &mut Vec<u8>, type_index: u8) {
        push_section(wasm, 3, &[0x01, type_index]);
    }

    fn push_function_section_many(wasm: &mut Vec<u8>, type_indices: &[u8]) {
        let mut payload = Vec::new();
        push_leb_u32(
            &mut payload,
            u32::try_from(type_indices.len()).expect("test WASM function count should fit in u32"),
        );
        payload.extend_from_slice(type_indices);
        push_section(wasm, 3, &payload);
    }

    fn push_global_section(wasm: &mut Vec<u8>) {
        push_section(wasm, 6, &[0x01, 0x7f, 0x01, 0x41, 0x00, 0x0b]);
    }

    fn push_export_section(wasm: &mut Vec<u8>, name: &str, function_index: u8) {
        push_export_section_many(wasm, &[(name, function_index)]);
    }

    fn push_export_section_many(wasm: &mut Vec<u8>, exports: &[(&str, u8)]) {
        let mut payload = Vec::new();
        push_leb_u32(
            &mut payload,
            u32::try_from(exports.len()).expect("test WASM export count should fit in u32"),
        );
        for (name, function_index) in exports {
            push_name(&mut payload, name);
            payload.push(0x00);
            payload.push(*function_index);
        }
        push_section(wasm, 7, &payload);
    }

    fn push_code_section(wasm: &mut Vec<u8>, bodies: &[&[u8]]) {
        let mut payload = Vec::new();
        push_leb_u32(
            &mut payload,
            u32::try_from(bodies.len()).expect("test WASM function count should fit in u32"),
        );
        for body in bodies {
            push_leb_u32(
                &mut payload,
                u32::try_from(body.len() + 1).expect("test WASM body size should fit in u32"),
            );
            payload.push(0x00);
            payload.extend_from_slice(body);
        }
        push_section(wasm, 10, &payload);
    }

    fn push_section(wasm: &mut Vec<u8>, section_id: u8, payload: &[u8]) {
        wasm.push(section_id);
        push_leb_u32(
            wasm,
            u32::try_from(payload.len()).expect("test WASM section payload should fit in u32"),
        );
        wasm.extend_from_slice(payload);
    }

    fn push_name(bytes: &mut Vec<u8>, value: &str) {
        push_leb_u32(
            bytes,
            u32::try_from(value.len()).expect("test WASM name length should fit in u32"),
        );
        bytes.extend_from_slice(value.as_bytes());
    }

    fn push_leb_u32(bytes: &mut Vec<u8>, mut value: u32) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            bytes.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nonce}"))
    }
}
