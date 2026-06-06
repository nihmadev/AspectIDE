use std::path::PathBuf;

use lux_core::WorkspaceInfo;
use lux_core::{
    ExtensionActivationPlan, ExtensionActivationReport, ExtensionCommandExecution,
    ExtensionCommandRoute, ExtensionContributionRegistry, ExtensionInfo,
};
use tauri::{AppHandle, Manager, State};

use super::{lock_error, SharedState};

#[tauri::command]
pub async fn extensions_list(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<Vec<ExtensionInfo>, String> {
    let roots = extension_discovery_roots(&app, &state)?;

    tokio::task::spawn_blocking(move || lux_extensions::discover_extensions_in_roots(roots))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
pub async fn extensions_activation_plan(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<ExtensionActivationPlan, String> {
    let roots = extension_discovery_roots(&app, &state)?;

    tokio::task::spawn_blocking(move || {
        lux_extensions::build_activation_plan(lux_extensions::discover_extensions_in_roots(roots)?)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(String::from)
}

#[tauri::command]
pub async fn extensions_activate(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<ExtensionActivationReport, String> {
    let roots = extension_discovery_roots(&app, &state)?;

    tokio::task::spawn_blocking(move || lux_extensions::activate_extensions_in_roots(roots))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
pub async fn extensions_contribution_registry(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<ExtensionContributionRegistry, String> {
    let roots = extension_discovery_roots(&app, &state)?;

    tokio::task::spawn_blocking(move || {
        lux_extensions::extension_contribution_registry_in_roots(roots)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(String::from)
}

#[tauri::command]
pub async fn extensions_command_routes(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<Vec<ExtensionCommandRoute>, String> {
    let roots = extension_discovery_roots(&app, &state)?;

    tokio::task::spawn_blocking(move || lux_extensions::extension_command_routes_in_roots(roots))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
pub async fn extensions_execute_command(
    app: AppHandle,
    state: State<'_, SharedState>,
    command_id: String,
) -> Result<ExtensionCommandExecution, String> {
    let roots = extension_discovery_roots(&app, &state)?;

    tokio::task::spawn_blocking(move || {
        lux_extensions::execute_extension_command_in_roots(roots, &command_id)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(String::from)
}

fn extension_discovery_roots(
    app: &AppHandle,
    state: &State<'_, SharedState>,
) -> Result<Vec<PathBuf>, String> {
    let mut roots = Vec::new();
    roots.push(
        app.path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("extensions"),
    );

    let workspace = state.workspace.lock().map_err(lock_error)?;
    if let Some(WorkspaceInfo { root, .. }) = workspace.as_ref() {
        let workspace_extensions = root.join(".lux").join("extensions");
        if workspace_extensions.exists() {
            roots.push(workspace_extensions);
        }
    }

    Ok(roots)
}