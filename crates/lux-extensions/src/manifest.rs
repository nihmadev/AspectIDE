// Manifest parsing, validation, field checks, and contribution mapping.
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

use lux_core::{
    AppError, AppResult, ExtensionCommandContribution, ExtensionContributionKind,
    ExtensionContributionPoint, ExtensionInfo, ExtensionManifest, ExtensionStatus,
};

use crate::{LUX_EXTENSION_ENTRYPOINT, LUX_EXTENSION_OPTIONAL_EXPORTS, MAX_CONTRIBUTION_POINTS};

// ---------------------------------------------------------------------------
// Info construction
// ---------------------------------------------------------------------------

pub fn read_extension_info(extension_root: PathBuf, manifest_path: PathBuf) -> ExtensionInfo {
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
                |v| v.to_string_lossy().to_string(),
            ),
            name: extension_root.file_name().map_or_else(
                || "Invalid extension".to_string(),
                |v| v.to_string_lossy().to_string(),
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

pub fn read_manifest(path: &Path) -> AppResult<ExtensionManifest> {
    let contents = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&contents)?)
}

pub fn manifest_to_info(
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

// ---------------------------------------------------------------------------
// Contribution points
// ---------------------------------------------------------------------------

#[must_use]
pub fn contribution_points_for_manifest(
    manifest: &ExtensionManifest,
) -> Vec<ExtensionContributionPoint> {
    let mut points = manifest
        .contributes
        .iter()
        .filter_map(|v| contribution_point(v))
        .collect::<Vec<_>>();
    points.sort_by(|l, r| l.id.cmp(&r.id));
    points.dedup_by(|l, r| l.id == r.id);
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

// ---------------------------------------------------------------------------
// Manifest validation
// ---------------------------------------------------------------------------

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
    if !trimmed.bytes().all(|b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-' || b == b'_'
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

pub fn validate_non_empty_field(label: &str, value: &str) -> AppResult<()> {
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
        if !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
        {
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
    if !manifest.contributes.iter().any(|p| p == "commands") {
        return Err(AppError::Service(
            "extension commands require the commands contribution point".into(),
        ));
    }
    for command in &manifest.commands {
        validate_extension_command(manifest, command)?;
    }
    // Catch intra-manifest duplicate command IDs at manifest-read time.
    let mut seen = BTreeSet::new();
    for command in &manifest.commands {
        if !seen.insert(command.id.as_str()) {
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

/// Validate command ID ownership with an exact namespace boundary.
///
/// F7 fix: previously only `starts_with(extension_id)` was checked, allowing
/// `lux.foo` to register `lux.foobar.run` (prefix impersonation of the
/// `lux.foobar` namespace).  We now require `"{extension_id}."` (dot
/// separator) plus a non-empty suffix.
pub fn validate_command_id(extension_id: &str, command_id: &str) -> AppResult<()> {
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
    // Exact namespace boundary: must start with "{extension_id}." and have a suffix.
    let required_prefix = format!("{extension_id}.");
    if !trimmed.starts_with(required_prefix.as_str()) || trimmed.len() <= required_prefix.len() {
        return Err(AppError::Service(format!(
            "extension command id must start with extension id namespace followed by '.': \
             expected prefix '{required_prefix}'"
        )));
    }
    if !trimmed.bytes().all(|b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-' || b == b'_'
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
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
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
