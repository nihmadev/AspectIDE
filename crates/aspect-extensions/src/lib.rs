#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

//! `aspect-extensions` РІР‚вЂќ WASM extension host for the AspectIDE.
//!
//! # Module layout
//!
//! | Module           | Responsibility                                           |
//! |------------------|----------------------------------------------------------|
//! | `manifest`       | Manifest parsing, field validation, contribution mapping |
//! | `discovery`      | Directory scanning, deterministic dedup, conflict report |
//! | `wasm_preflight` | Binary validation: size, magic, sections, ABI contract   |
//! | `plan`           | `ExtensionActivationPlan` builder                        |
//! | `activation`     | WASM init-export runner РІвЂ вЂ™ `ExtensionActivationReport`    |
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

use aspect_core::{
    AppResult, ExtensionActivationPlan, ExtensionActivationReport, ExtensionCommandExecution,
    ExtensionCommandRoute, ExtensionContributionRegistry, ExtensionInfo, ExtensionManifest,
};

// ---------------------------------------------------------------------------
// Shared constants (used by multiple modules)
// ---------------------------------------------------------------------------

const MANIFEST_FILE: &str = "aspect-extension.json";
const MAX_CONTRIBUTION_POINTS: usize = 128;
const MAX_WASM_MODULE_BYTES: u64 = 32 * 1024 * 1024;
const WASM_MAGIC_AND_VERSION: [u8; 8] = [0x00, b'a', b's', b'm', 0x01, 0x00, 0x00, 0x00];
const ASPECT_EXTENSION_ABI_VERSION: u32 = 1;
const ASPECT_EXTENSION_ENTRYPOINT: &str = "aspect_extension_init";
const ASPECT_EXTENSION_OPTIONAL_EXPORTS: &[&str] = &["aspect_extension_shutdown"];
const ASPECT_HOST_IMPORT_MODULE: &str = "aspect:extension/host@1";
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
    permission: Option<aspect_core::ExtensionHostPermission>,
}

/// All imports the host contract allows.  Imports with a `permission` require
/// the corresponding manifest declaration.  The F9 fix in `wasm_preflight`
/// further rejects I/O imports (those with `permission.is_some()`) at
/// preflight time until real host-side implementations are provided.
const ALLOWED_HOST_IMPORTS: &[HostImportSpec] = &[
    HostImportSpec {
        name: "log",
        permission: None,
    },
    HostImportSpec {
        name: "workspace_read",
        permission: Some(aspect_core::ExtensionHostPermission::WorkspaceRead),
    },
    HostImportSpec {
        name: "workspace_write",
        permission: Some(aspect_core::ExtensionHostPermission::WorkspaceWrite),
    },
    HostImportSpec {
        name: "network_fetch",
        permission: Some(aspect_core::ExtensionHostPermission::NetworkAccess),
    },
    HostImportSpec {
        name: "process_spawn",
        permission: Some(aspect_core::ExtensionHostPermission::ProcessSpawn),
    },
];

// ---------------------------------------------------------------------------
// Public API РІР‚вЂќ single-root convenience wrappers
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
// Public API РІР‚вЂќ multi-root variants
// ---------------------------------------------------------------------------

pub fn extension_activation_plan_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> AppResult<ExtensionActivationPlan> {
    Ok(build_activation_plan(discover_extensions_in_roots(roots)?))
}

pub fn activate_extensions_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> AppResult<ExtensionActivationReport> {
    Ok(activate_extension_plan(extension_activation_plan_in_roots(
        roots,
    )?))
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
    commands::validate_unique_command_routes(&routes)?;
    Ok(routes)
}

pub fn execute_extension_command_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
    command_id: &str,
) -> AppResult<ExtensionCommandExecution> {
    let plan = extension_activation_plan_in_roots(roots)?;
    Ok(commands::execute_extension_command_from_plan(
        &plan, command_id,
    ))
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
) -> Vec<aspect_core::ExtensionContributionPoint> {
    manifest::contribution_points_for_manifest(manifest)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

