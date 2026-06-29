#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

//! `lux-extensions` — WASM extension host for the Lux IDE.
//!
//! # Module layout
//!
//! | Module           | Responsibility                                           |
//! |------------------|----------------------------------------------------------|
//! | `manifest`       | Manifest parsing, field validation, contribution mapping |
//! | `discovery`      | Directory scanning, deterministic dedup, conflict report |
//! | `wasm_preflight` | Binary validation: size, magic, sections, ABI contract   |
//! | `plan`           | `ExtensionActivationPlan` builder                        |
//! | `activation`     | WASM init-export runner → `ExtensionActivationReport`    |
//! | `commands`       | Route building, duplicate detection, command execution   |
//! | `registry`       | Contribution registry from activation report             |
//! | `runtime`        | Wasmtime engine, fuel, wall-clock timeout, host imports  |

mod activation;
mod commands;
mod discovery;
mod manifest;
mod plan;
mod registry;
mod runtime;
mod wasm_preflight;

use std::path::Path;

use lux_core::{
    AppResult, ExtensionActivationPlan, ExtensionActivationReport, ExtensionCommandExecution,
    ExtensionCommandRoute, ExtensionContributionRegistry, ExtensionInfo, ExtensionManifest,
};

// ---------------------------------------------------------------------------
// Shared constants (used by multiple modules)
// ---------------------------------------------------------------------------

const MANIFEST_FILE: &str = "lux-extension.json";
const MAX_CONTRIBUTION_POINTS: usize = 128;
const MAX_WASM_MODULE_BYTES: u64 = 32 * 1024 * 1024;
const WASM_MAGIC_AND_VERSION: [u8; 8] = [0x00, b'a', b's', b'm', 0x01, 0x00, 0x00, 0x00];
const LUX_EXTENSION_ABI_VERSION: u32 = 1;
const LUX_EXTENSION_ENTRYPOINT: &str = "lux_extension_init";
const LUX_EXTENSION_OPTIONAL_EXPORTS: &[&str] = &["lux_extension_shutdown"];
const LUX_HOST_IMPORT_MODULE: &str = "lux:extension/host@1";
const EXTENSION_HOST_MAX_MEMORY_PAGES: u32 = 256;
/// Wall-clock deadline for extension activation (compile + instantiate + init).
const EXTENSION_HOST_ACTIVATION_TIMEOUT_MS: u64 = 5_000;
const EXTENSION_HOST_MAX_OUTPUT_BYTES: u64 = 1024 * 1024;
/// Fuel budget for the activation (init) export.
const EXTENSION_HOST_ACTIVATION_FUEL: u64 = 250_000;
/// F8: separate fuel budget for command handler calls so handler budget is
/// independent of how much fuel the activation export consumed.
const EXTENSION_HOST_COMMAND_FUEL: u64 = 250_000;
const EXTENSION_WASM_EXECUTION_EXHAUSTED_FUEL: &str = "extension WASM execution exhausted fuel";
const WASM_PAGE_BYTES: usize = 65_536;
/// F4: maximum number of table elements an extension may declare; bounds
/// host resource consumption that memory limits alone do not cover.
const EXTENSION_HOST_MAX_TABLE_ELEMENTS: u32 = 65_536;

// ---------------------------------------------------------------------------
// Host import spec (shared between wasm_preflight and runtime)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct HostImportSpec {
    name: &'static str,
    permission: Option<lux_core::ExtensionHostPermission>,
}

/// All imports the host contract allows.  Imports with a `permission` require
/// the corresponding manifest declaration.  The F9 fix in `wasm_preflight`
/// further rejects I/O imports (those with `permission.is_some()`) at
/// preflight time until real host-side implementations are provided.
const ALLOWED_HOST_IMPORTS: &[HostImportSpec] = &[
    HostImportSpec { name: "log", permission: None },
    HostImportSpec {
        name: "workspace_read",
        permission: Some(lux_core::ExtensionHostPermission::WorkspaceRead),
    },
    HostImportSpec {
        name: "workspace_write",
        permission: Some(lux_core::ExtensionHostPermission::WorkspaceWrite),
    },
    HostImportSpec {
        name: "network_fetch",
        permission: Some(lux_core::ExtensionHostPermission::NetworkAccess),
    },
    HostImportSpec {
        name: "process_spawn",
        permission: Some(lux_core::ExtensionHostPermission::ProcessSpawn),
    },
];

// ---------------------------------------------------------------------------
// Public API — single-root convenience wrappers
// ---------------------------------------------------------------------------

pub fn discover_extensions(root: impl AsRef<Path>) -> AppResult<Vec<ExtensionInfo>> {
    discovery::discover_extensions(root)
}

pub fn discover_extensions_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> AppResult<Vec<ExtensionInfo>> {
    discovery::discover_extensions_in_roots(roots)
}

pub fn extension_activation_plan(root: impl AsRef<Path>) -> AppResult<ExtensionActivationPlan> {
    Ok(build_activation_plan(discover_extensions(root)?))
}

pub fn activate_extensions(root: impl AsRef<Path>) -> AppResult<ExtensionActivationReport> {
    Ok(activate_extension_plan(extension_activation_plan(root)?))
}

pub fn extension_contribution_registry(
    root: impl AsRef<Path>,
) -> AppResult<ExtensionContributionRegistry> {
    Ok(build_contribution_registry(activate_extensions(root)?))
}

pub fn extension_command_routes(root: impl AsRef<Path>) -> AppResult<Vec<ExtensionCommandRoute>> {
    let plan = extension_activation_plan(root)?;
    let routes = command_routes_for_activation_plan(&plan);
    commands::validate_unique_command_routes(&routes)?;
    Ok(routes)
}

pub fn execute_extension_command(
    root: impl AsRef<Path>,
    command_id: &str,
) -> AppResult<ExtensionCommandExecution> {
    execute_extension_command_in_roots([root.as_ref()], command_id)
}

// ---------------------------------------------------------------------------
// Public API — multi-root variants
// ---------------------------------------------------------------------------

pub fn extension_activation_plan_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> AppResult<ExtensionActivationPlan> {
    Ok(build_activation_plan(discover_extensions_in_roots(roots)?))
}

pub fn activate_extensions_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> AppResult<ExtensionActivationReport> {
    Ok(activate_extension_plan(extension_activation_plan_in_roots(roots)?))
}

pub fn extension_contribution_registry_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> AppResult<ExtensionContributionRegistry> {
    Ok(build_contribution_registry(activate_extensions_in_roots(roots)?))
}

pub fn extension_command_routes_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> AppResult<Vec<ExtensionCommandRoute>> {
    let plan = extension_activation_plan_in_roots(roots)?;
    let routes = command_routes_for_activation_plan(&plan);
    commands::validate_unique_command_routes(&routes)?;
    Ok(routes)
}

pub fn execute_extension_command_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
    command_id: &str,
) -> AppResult<ExtensionCommandExecution> {
    let plan = extension_activation_plan_in_roots(roots)?;
    Ok(commands::execute_extension_command_from_plan(&plan, command_id))
}

// ---------------------------------------------------------------------------
// Public plan/activation/registry helpers (used by Tauri commands)
// ---------------------------------------------------------------------------

#[must_use]
pub fn activate_extension_plan(plan: ExtensionActivationPlan) -> ExtensionActivationReport {
    activation::activate_extension_plan(plan)
}

#[must_use]
pub fn build_activation_plan(extensions: Vec<ExtensionInfo>) -> ExtensionActivationPlan {
    plan::build_activation_plan(extensions)
}

#[must_use]
pub fn build_contribution_registry(
    activation: ExtensionActivationReport,
) -> ExtensionContributionRegistry {
    registry::build_contribution_registry(activation)
}

#[must_use]
pub fn command_routes_for_activation_plan(
    plan: &ExtensionActivationPlan,
) -> Vec<ExtensionCommandRoute> {
    commands::command_routes_for_activation_plan(plan)
}

pub fn validate_manifest(manifest: &ExtensionManifest, extension_root: &Path) -> AppResult<()> {
    manifest::validate_manifest(manifest, extension_root)
}

#[must_use]
pub fn contribution_points_for_manifest(
    manifest: &ExtensionManifest,
) -> Vec<lux_core::ExtensionContributionPoint> {
    manifest::contribution_points_for_manifest(manifest)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::{SystemTime, UNIX_EPOCH}};
    use lux_core::{
        ExtensionCommandExecutionPhase, ExtensionCommandExecutionStatus,
        ExtensionContributionKind, ExtensionManifest,
        ExtensionStatus,
    };
    use std::path::PathBuf;

    // -----------------------------------------------------------------------
    // Manifest unit tests
    // -----------------------------------------------------------------------

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
            lux_core::ExtensionContributionPoint {
                id: "commands".to_string(),
                kind: ExtensionContributionKind::Commands,
            }
        );
        assert_eq!(
            points[1],
            lux_core::ExtensionContributionPoint {
                id: "custom.point".to_string(),
                kind: ExtensionContributionKind::Unknown,
            }
        );
        assert_eq!(
            points[2],
            lux_core::ExtensionContributionPoint {
                id: "languages".to_string(),
                kind: ExtensionContributionKind::Languages,
            }
        );
    }

    // -----------------------------------------------------------------------
    // F7 regression: namespace boundary must be exact (dot separator)
    // -----------------------------------------------------------------------

    #[test]
    fn validate_command_id_requires_exact_namespace_boundary() {
        // "lux.foo" must NOT be allowed to register "lux.foobar.run"
        let err = manifest::validate_command_id("lux.foo", "lux.foobar.run")
            .expect_err("prefix impersonation should be rejected");
        assert!(
            err.to_string().contains("namespace"),
            "unexpected error: {err}"
        );

        // "lux.foo" CAN register "lux.foo.run"
        manifest::validate_command_id("lux.foo", "lux.foo.run")
            .expect("valid namespaced command id should be accepted");
    }

    // -----------------------------------------------------------------------
    // Discovery tests
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // F6 regression: duplicate IDs must be reported, not silently dropped
    // -----------------------------------------------------------------------

    #[test]
    fn discover_extensions_reports_duplicate_extension_ids_as_invalid() {
        let root_a = unique_temp_dir("lux-extension-dup-a");
        let root_b = unique_temp_dir("lux-extension-dup-b");
        // Both roots define "lux.shared" — second should be invalid with conflict reason.
        write_extension(
            &root_a,
            "shared-ext",
            r#"{"id":"lux.shared","name":"Shared","version":"0.1.0","wasm_module":"extension.wasm","contributes":["commands"]}"#,
            true,
        );
        write_extension(
            &root_b,
            "shared-ext",
            r#"{"id":"lux.shared","name":"Shared","version":"0.1.0","wasm_module":"extension.wasm","contributes":["commands"]}"#,
            true,
        );

        let extensions =
            discover_extensions_in_roots([root_a.as_path(), root_b.as_path()])
                .expect("discovery should not fail");

        // One winner + one conflict.
        assert_eq!(extensions.len(), 2, "both entries should appear");
        let conflict = extensions.iter().find(|e| e.status == ExtensionStatus::Invalid);
        assert!(conflict.is_some(), "duplicate should be marked Invalid");
        assert!(
            conflict
                .unwrap()
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("duplicate"),
            "error should mention duplicate"
        );

        let _ = fs::remove_dir_all(root_a);
        let _ = fs::remove_dir_all(root_b);
    }

    // -----------------------------------------------------------------------
    // Activation plan tests
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // F9 regression: declared I/O imports must be rejected at preflight
    // -----------------------------------------------------------------------

    #[test]
    fn activation_plan_blocks_unimplemented_io_imports_even_with_permission() {
        let root = unique_temp_dir("lux-extension-io-import-preflight");
        write_extension(
            &root,
            "io-tools",
            r#"{
                "id": "lux.io_tools",
                "name": "IO Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "permissions": ["workspaceRead"],
                "contributes": ["commands"]
            }"#,
            false,
        );
        fs::write(
            root.join("io-tools").join("extension.wasm"),
            lux_wasm_with_host_import("workspace_read"),
        )
        .expect("wasm module should be written");

        let plan = extension_activation_plan(&root).expect("activation plan should be built");

        // Extension declares the permission but the import is not yet
        // implemented, so the preflight must block it.
        assert!(
            plan.candidates.is_empty(),
            "IO import should be blocked at preflight"
        );
        assert_eq!(plan.blocked.len(), 1);
        assert!(
            plan.blocked[0].reason.contains("not yet implemented"),
            "block reason should mention not-yet-implemented: {}",
            plan.blocked[0].reason
        );

        let _ = fs::remove_dir_all(root);
    }

    // -----------------------------------------------------------------------
    // F3 regression: host import must match the exact zero-arg/zero-result ABI
    // signature; a mismatched signature must be rejected at preflight, not at
    // activation.
    // -----------------------------------------------------------------------

    #[test]
    fn activation_plan_blocks_host_import_with_mismatched_signature() {
        let root = unique_temp_dir("lux-extension-import-signature");
        write_extension(
            &root,
            "bad-signature",
            r#"{
                "id": "lux.bad_signature",
                "name": "Bad Signature",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands"]
            }"#,
            false,
        );
        // Imports the no-permission `log` host function but declares it with a
        // `(i32) -> ()` signature instead of the linked `() -> ()`.
        fs::write(
            root.join("bad-signature").join("extension.wasm"),
            lux_wasm_with_host_import_one_param("log"),
        )
        .expect("wasm module should be written");

        let plan = extension_activation_plan(&root).expect("activation plan should be built");

        assert!(
            plan.candidates.is_empty(),
            "mismatched host import signature should be blocked at preflight"
        );
        assert_eq!(plan.blocked.len(), 1);
        assert!(
            plan.blocked[0].reason.contains("unsupported signature"),
            "block reason should mention the signature mismatch: {}",
            plan.blocked[0].reason
        );

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

    // -----------------------------------------------------------------------
    // Activation runtime tests
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Contribution registry tests
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Command route + execution tests
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // F5 regression: duplicate command ID in one extension must not break
    // commands from other extensions
    // -----------------------------------------------------------------------

    #[test]
    fn execute_command_is_not_blocked_by_duplicate_in_unrelated_extension() {
        let root = unique_temp_dir("lux-extension-f5-dup-cmd");
        // ext-a: has "lux.ext_a.hello" — should remain executable
        write_extension(
            &root,
            "ext-a",
            r#"{
                "id": "lux.ext_a",
                "name": "Ext A",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands"],
                "commands": [{"id":"lux.ext_a.hello","title":"Hello","handler":"lux_ext_a_hello"}]
            }"#,
            false,
        );
        fs::write(
            root.join("ext-a").join("extension.wasm"),
            command_lux_wasm("lux_ext_a_hello", &[0x0b]),
        )
        .expect("ext-a wasm should be written");

        let result = execute_extension_command(&root, "lux.ext_a.hello")
            .expect("command from unique extension should execute");
        assert_eq!(
            result.status,
            ExtensionCommandExecutionStatus::Succeeded,
            "unique command should succeed regardless of unrelated duplicates"
        );

        let _ = fs::remove_dir_all(root);
    }

    // -----------------------------------------------------------------------
    // Other manifest validation tests
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Test WASM builders and helpers
    // -----------------------------------------------------------------------

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

    fn write_extension(root: &Path, name: &str, manifest_json: &str, write_wasm: bool) {
        let extension_root = root.join(name);
        fs::create_dir_all(&extension_root).expect("extension root should be created");
        if write_wasm {
            fs::write(extension_root.join("extension.wasm"), minimal_lux_wasm())
                .expect("wasm module should be written");
        }
        fs::write(extension_root.join(MANIFEST_FILE), manifest_json)
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
                &[0x23, 0x00, 0x41, 0x01, 0x46, 0x04, 0x40, 0x0f, 0x0b, 0x00, 0x0b],
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

    /// Builds a Lux host import whose function type takes one `i32` parameter,
    /// i.e. it does NOT match the linked zero-arg/zero-result host ABI.  Used to
    /// exercise the F3 signature check.
    fn lux_wasm_with_host_import_one_param(name: &str) -> Vec<u8> {
        let mut wasm = WASM_MAGIC_AND_VERSION.to_vec();
        // Type section: type 0 = (i32) -> (), type 1 = () -> ().
        let mut type_payload = Vec::new();
        type_payload.push(0x02); // two types
        type_payload.extend_from_slice(&[0x60, 0x01, 0x7f, 0x00]); // (i32) -> ()
        type_payload.extend_from_slice(&[0x60, 0x00, 0x00]); // () -> ()
        push_section(&mut wasm, 1, &type_payload);
        // Import section: host import referencing the (i32) -> () type (index 0).
        let mut import_payload = Vec::new();
        import_payload.push(0x01); // one import
        push_name(&mut import_payload, LUX_HOST_IMPORT_MODULE);
        push_name(&mut import_payload, name);
        import_payload.push(0x00); // import kind: function
        import_payload.push(0x00); // type index 0
        push_section(&mut wasm, 2, &import_payload);
        // The single locally-defined function uses type 1 (() -> ()).
        push_function_section(&mut wasm, 1);
        // Export the entrypoint (function index 1: import 0 + local 0).
        push_export_section(&mut wasm, LUX_EXTENSION_ENTRYPOINT, 1);
        push_code_section(&mut wasm, &[&[0x0b]]);
        wasm
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
