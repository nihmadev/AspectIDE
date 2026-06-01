use lux_core::ExtensionInfo;
use tauri::{AppHandle, Manager};

#[tauri::command]
pub async fn extensions_list(app: AppHandle) -> Result<Vec<ExtensionInfo>, String> {
    let extensions_root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("extensions");

    tokio::task::spawn_blocking(move || lux_extensions::discover_extensions(extensions_root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}
