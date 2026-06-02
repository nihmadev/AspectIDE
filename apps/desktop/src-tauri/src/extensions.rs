use lux_core::{
    ExtensionActivationPlan, ExtensionActivationReport, ExtensionCommandExecution,
    ExtensionCommandRoute, ExtensionContributionRegistry, ExtensionInfo,
};
use tauri::{AppHandle, Manager};

#[tauri::command]
pub async fn extensions_list(app: AppHandle) -> Result<Vec<ExtensionInfo>, String> {
    let extensions_root = extensions_root(&app)?;

    tokio::task::spawn_blocking(move || lux_extensions::discover_extensions(extensions_root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
pub async fn extensions_activation_plan(app: AppHandle) -> Result<ExtensionActivationPlan, String> {
    let extensions_root = extensions_root(&app)?;

    tokio::task::spawn_blocking(move || lux_extensions::extension_activation_plan(extensions_root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
pub async fn extensions_activate(app: AppHandle) -> Result<ExtensionActivationReport, String> {
    let extensions_root = extensions_root(&app)?;

    tokio::task::spawn_blocking(move || lux_extensions::activate_extensions(extensions_root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
pub async fn extensions_contribution_registry(
    app: AppHandle,
) -> Result<ExtensionContributionRegistry, String> {
    let extensions_root = extensions_root(&app)?;

    tokio::task::spawn_blocking(move || {
        lux_extensions::extension_contribution_registry(extensions_root)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(String::from)
}

#[tauri::command]
pub async fn extensions_command_routes(
    app: AppHandle,
) -> Result<Vec<ExtensionCommandRoute>, String> {
    let extensions_root = extensions_root(&app)?;

    tokio::task::spawn_blocking(move || lux_extensions::extension_command_routes(extensions_root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
pub async fn extensions_execute_command(
    app: AppHandle,
    command_id: String,
) -> Result<ExtensionCommandExecution, String> {
    let extensions_root = extensions_root(&app)?;

    tokio::task::spawn_blocking(move || {
        lux_extensions::execute_extension_command(extensions_root, &command_id)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(String::from)
}

fn extensions_root(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|error| error.to_string())
        .map(|path| path.join("extensions"))
}
