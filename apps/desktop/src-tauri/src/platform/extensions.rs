use std::path::PathBuf;

use aspect_core::WorkspaceInfo;
use aspect_core::{
    ExtensionActivationPlan, ExtensionActivationReport, ExtensionCommandExecution,
    ExtensionCommandRoute, ExtensionContributionRegistry, ExtensionInfo,
};
use tauri::{AppHandle, Manager, State};

use crate::{lock_error, SharedState};

#[tauri::command]
pub async fn extensions_list(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<Vec<ExtensionInfo>, String> {
    let roots = extension_discovery_roots(&app, &state)?;

    tokio::task::spawn_blocking(move || {
        aspect_extensions::discover_extensions_in_roots(roots).map_err(|e| e.to_string())
    })
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
        // `build_activation_plan` is infallible (returns the plan directly); only
        // discovery can fail, so surface that error and wrap the plan in `Ok`.
        let extensions =
            aspect_extensions::discover_extensions_in_roots(roots).map_err(String::from)?;
        Ok::<ExtensionActivationPlan, String>(aspect_extensions::build_activation_plan(extensions))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn extensions_activate(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<ExtensionActivationReport, String> {
    let roots = extension_discovery_roots(&app, &state)?;

    tokio::task::spawn_blocking(move || {
        aspect_extensions::activate_extensions_in_roots(roots).map_err(|e| e.to_string())
    })
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
        aspect_extensions::extension_contribution_registry_in_roots(roots).map_err(|e| e.to_string())
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

    tokio::task::spawn_blocking(move || {
        aspect_extensions::extension_command_routes_in_roots(roots).map_err(|e| e.to_string())
    })
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
        aspect_extensions::execute_extension_command_in_roots(roots, &command_id).map_err(|e| e.to_string())
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
        let workspace_extensions = root.join(".aspect").join("extensions");
        if workspace_extensions.exists() {
            roots.push(workspace_extensions);
        }
    }

    Ok(roots)
}
