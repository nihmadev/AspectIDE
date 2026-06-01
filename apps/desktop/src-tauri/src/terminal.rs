use std::path::PathBuf;

use lux_core::TerminalSessionInfo;
use tauri::State;
use uuid::Uuid;

use super::{lock_error, SharedState};

#[tauri::command]
pub fn terminal_create(
    state: State<'_, SharedState>,
    shell: Option<String>,
    cwd: Option<PathBuf>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<TerminalSessionInfo, String> {
    let cwd = match cwd {
        Some(path) => path,
        None => state
            .workspace
            .lock()
            .map_err(lock_error)?
            .as_ref()
            .map_or_else(
                || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                |workspace| workspace.root.clone(),
            ),
    };
    terminal_service(&state)?
        .create(shell, cwd, cols.unwrap_or(120), rows.unwrap_or(30))
        .map_err(String::from)
}

#[tauri::command]
pub fn terminal_write(
    state: State<'_, SharedState>,
    session_id: Uuid,
    data: String,
) -> Result<(), String> {
    terminal_service(&state)?
        .write(session_id, &data)
        .map_err(String::from)
}

#[tauri::command]
pub fn terminal_resize(
    state: State<'_, SharedState>,
    session_id: Uuid,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    terminal_service(&state)?
        .resize(session_id, cols, rows)
        .map_err(String::from)
}

#[tauri::command]
pub fn terminal_close(state: State<'_, SharedState>, session_id: Uuid) -> Result<(), String> {
    terminal_service(&state)?
        .close(session_id)
        .map_err(String::from)
}

#[tauri::command]
pub fn terminal_close_all(state: State<'_, SharedState>) -> Result<(), String> {
    close_all(&state)
}

pub fn close_all(state: &State<'_, SharedState>) -> Result<(), String> {
    terminal_service(state)?.close_all().map_err(String::from)
}

fn terminal_service(
    state: &State<'_, SharedState>,
) -> Result<std::sync::Arc<lux_terminal::TerminalService>, String> {
    state
        .terminals
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .cloned()
        .ok_or_else(|| "terminal service is not initialized".to_string())
}
