use std::path::PathBuf;

use lux_core::DebugWorkspaceInfo;
use tauri::State;

use super::{lock_error, SharedState};

#[tauri::command]
pub async fn debug_workspace_info(
    state: State<'_, SharedState>,
) -> Result<DebugWorkspaceInfo, String> {
    let root = workspace_root(&state)?;
    tokio::task::spawn_blocking(move || lux_dap::workspace_debug_info(root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

fn workspace_root(state: &State<'_, SharedState>) -> Result<PathBuf, String> {
    state
        .workspace
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .map(|workspace| workspace.root.clone())
        .ok_or_else(|| "no workspace is open".to_string())
}
