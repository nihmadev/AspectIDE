use std::path::PathBuf;

use aspect_core::TerminalSessionInfo;
use tauri::State;
use uuid::Uuid;

use crate::{lock_error, resolve_workspace_path, SharedState};

#[tauri::command]
pub fn terminal_create(
    state: State<'_, SharedState>,
    shell: Option<String>,
    cwd: Option<PathBuf>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<TerminalSessionInfo, String> {
    let cwd = match cwd {
        // A caller-supplied cwd is resolved through the workspace guard so a shell
        // can never be spawned outside the open project (the terminal is a powerful
        // AI/renderer surface). `resolve_workspace_path` requires the path to exist
        // and rejects absolute/`..`-escaping targets; reject non-directories too.
        Some(path) => {
            let resolved = resolve_workspace_path(&state, &path)?;
            if !resolved.is_dir() {
                return Err(format!(
                    "terminal cwd is not a directory: {}",
                    resolved.display()
                ));
            }
            resolved
        }
        // No cwd given: default to the workspace root. With no workspace open this
        // falls back to the process cwd — a user-only convenience that carries no
        // caller-controlled path, so it can't be used to escape a workspace.
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
) -> Result<std::sync::Arc<aspect_terminal::TerminalService>, String> {
    state
        .terminals
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .cloned()
        .ok_or_else(|| "terminal service is not initialized".to_string())
}
