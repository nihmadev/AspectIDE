//! Thin Tauri command wrappers over `aspect_runtimes`.
//!
//! The core logic lives in `crates/aspect-runtimes` — these adapters bridge
//! Tauri's `AppHandle`/`Emitter` to the crate's `&Path` + callback API.

use aspect_runtimes::lsp::{self as rt_lsp, LspInstallEvent};
use aspect_runtimes::runtime::{self as rt_runtime, RuntimeProvisionEvent};
use tauri::{AppHandle, Emitter, Manager};

fn data_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path().app_data_dir().map_err(|e| e.to_string())
}

// ── Runtime provisioning ──

#[tauri::command]
pub fn runtime_catalog(app: AppHandle) -> Result<Vec<aspect_runtimes::runtime::RuntimeCatalogEntry>, String> {
    Ok(rt_runtime::runtime_catalog(&data_dir(&app)?))
}

#[tauri::command]
pub async fn runtime_provision(app: AppHandle, id: String) -> Result<String, String> {
    let dir = data_dir(&app)?;
    let sink = move |event: RuntimeProvisionEvent| {
        let _ = app.emit("aspect://runtime-provision", &event);
    };
    rt_runtime::runtime_provision(&dir, &id, &sink).await
}

// ── LSP install ──

#[tauri::command]
pub fn lsp_server_catalog(app: AppHandle) -> Result<Vec<aspect_runtimes::lsp::LspCatalogEntry>, String> {
    Ok(rt_lsp::lsp_server_catalog(&data_dir(&app)?))
}

#[tauri::command]
pub async fn lsp_install_server(app: AppHandle, language_id: String) -> Result<String, String> {
    let dir = data_dir(&app)?;
    let sink = move |event: LspInstallEvent| {
        let _ = app.emit("aspect://lsp-install", &event);
    };
    rt_lsp::lsp_install_server(&dir, &language_id, &sink).await
}

#[tauri::command]
pub async fn lsp_uninstall_server(app: AppHandle, language_id: String) -> Result<String, String> {
    let dir = data_dir(&app)?;
    let sink = move |event: LspInstallEvent| {
        let _ = app.emit("aspect://lsp-install", &event);
    };
    rt_lsp::lsp_uninstall_server(&dir, &language_id, &sink).await
}
