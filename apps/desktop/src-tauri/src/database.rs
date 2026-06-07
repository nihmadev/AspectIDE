use std::path::PathBuf;

use lux_core::{DatabaseTablePreview, FileInspectionOptions};
use lux_file_intel::{
    database_execute, database_tables, database_update_cell as apply_database_cell_update,
    DatabaseCellUpdate, DatabaseExecuteRequest, DatabaseExecuteResult,
};
use tauri::State;

use super::{resolve_workspace_path, SharedState};

#[tauri::command]
pub async fn database_list_tables(
    state: State<'_, SharedState>,
    path: PathBuf,
    options: Option<FileInspectionOptions>,
) -> Result<Vec<DatabaseTablePreview>, String> {
    let path = resolve_workspace_path(&state, &path)?;
    tokio::task::spawn_blocking(move || {
        database_tables(&path, &options.unwrap_or_default()).map_err(String::from)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn database_execute_sql(
    state: State<'_, SharedState>,
    path: PathBuf,
    request: DatabaseExecuteRequest,
) -> Result<DatabaseExecuteResult, String> {
    let path = resolve_workspace_path(&state, &path)?;
    let sql = request.sql;
    tokio::task::spawn_blocking(move || database_execute(&path, &sql).map_err(String::from))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn database_update_cell(
    state: State<'_, SharedState>,
    path: PathBuf,
    update: DatabaseCellUpdate,
) -> Result<(), String> {
    let path = resolve_workspace_path(&state, &path)?;
    tokio::task::spawn_blocking(move || {
        apply_database_cell_update(&path, &update).map_err(String::from)
    })
    .await
    .map_err(|error| error.to_string())?
}
