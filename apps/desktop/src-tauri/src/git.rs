use std::path::PathBuf;

use lux_core::{GitDiff, GitStatus};
use tauri::State;

use super::{lock_error, SharedState};

#[tauri::command]
pub async fn git_status(state: State<'_, SharedState>) -> Result<GitStatus, String> {
    let root = workspace_root(&state)?;
    tokio::task::spawn_blocking(move || lux_git::status(root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
pub async fn git_diff(state: State<'_, SharedState>) -> Result<GitDiff, String> {
    let root = workspace_root(&state)?;
    tokio::task::spawn_blocking(move || lux_git::diff(root))
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
