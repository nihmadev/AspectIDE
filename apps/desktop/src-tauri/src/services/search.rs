use std::path::PathBuf;

use aspect_core::{SearchOptions, SearchResponse};
use tauri::State;

use crate::{lock_error, SharedState};

#[tauri::command]
pub async fn search_query(
    state: State<'_, SharedState>,
    query: String,
    options: SearchOptions,
) -> Result<SearchResponse, String> {
    let root = workspace_root(&state)?;
    tokio::task::spawn_blocking(move || aspect_search::query(root, query, &options))
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
