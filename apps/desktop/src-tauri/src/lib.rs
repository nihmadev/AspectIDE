#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]
#![allow(
    clippy::large_stack_frames,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value,
    clippy::significant_drop_tightening,
    clippy::similar_names,
    clippy::struct_excessive_bools,
    clippy::too_many_lines
)]

use std::{
    collections::BTreeMap,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

mod debug;
mod editor;
mod extensions;
mod file_intel;
mod git;
mod lsp;
mod search;
mod settings;
mod terminal;
mod test_health;
mod voice_input;
mod web_fetch;
mod workspace_watcher;

mod ai_chat_backend;
mod ai_tools;

use lux_core::{
    BufferId, FsEntry, LuxEvent, WorkspaceDiagnostic, WorkspaceEditResult, WorkspaceInfo,
};
use lux_editor::DocumentStore;
use lux_settings::SettingsStore;
use lux_terminal::TerminalService;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_log::{log::LevelFilter, Target, TargetKind};
use tokio::sync::oneshot;

use ai_tools::{
    ai_file_delete, ai_file_patch, ai_file_str_replace, ai_file_write, ai_shell, ai_symbol_context,
};
use debug::{
    debug_evaluate, debug_execute, debug_scopes, debug_sessions, debug_set_breakpoints,
    debug_stack_trace, debug_start, debug_stop, debug_variables, debug_workspace_info,
};
use editor::{
    editor_apply_edits, editor_apply_workspace_edit, editor_new_file, editor_open_file,
    editor_save_file, editor_save_file_as, editor_update_text,
};
use extensions::{
    extensions_activate, extensions_activation_plan, extensions_command_routes,
    extensions_contribution_registry, extensions_execute_command, extensions_list,
};
use file_intel::{file_asset_data, file_inspect, file_open_external, file_supported_formats};
use git::{git_diff, git_status};
use lsp::{
    diagnostics_snapshot, lsp_code_actions, lsp_completion, lsp_definition, lsp_document_symbols,
    lsp_folding_ranges, lsp_format_document, lsp_format_range, lsp_hover, lsp_inlay_hints,
    lsp_references, lsp_semantic_tokens, lsp_servers, lsp_signature_help, lsp_workspace_symbols,
};
use search::search_query;
use settings::{
    keybindings_get, keybindings_set, recent_workspace_forget, recent_workspaces, settings_get,
    settings_set,
};
use terminal::{
    terminal_close, terminal_close_all, terminal_create, terminal_resize, terminal_write,
};

const AI_READ_TEXT_MAX_BYTES: u64 = 1_000_000;

#[derive(Default)]
struct AppState {
    workspace: Mutex<Option<WorkspaceInfo>>,
    workspace_watcher: Mutex<Option<workspace_watcher::WorkspaceWatcher>>,
    documents: Mutex<DocumentStore>,
    diagnostics: Mutex<Vec<WorkspaceDiagnostic>>,
    ai_streams: Mutex<BTreeMap<String, oneshot::Sender<()>>>,
    lsp: tokio::sync::Mutex<Option<lux_lsp::LspManager>>,
    debug: tokio::sync::Mutex<Option<lux_dap::DebugSessionManager>>,
    settings: Mutex<Option<SettingsStore>>,
    terminals: Mutex<Option<Arc<TerminalService>>>,
}

type SharedState = Arc<AppState>;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FsReadTextResponse {
    path: PathBuf,
    text: String,
    truncated: bool,
    size: u64,
}

#[tauri::command]
async fn workspace_open(
    app: AppHandle,
    state: State<'_, SharedState>,
    path: PathBuf,
) -> Result<WorkspaceInfo, String> {
    let workspace = lux_workspace::open_workspace(path).map_err(String::from)?;
    workspace_watcher::stop(&state)?;
    lsp::shutdown(&state).await;
    debug::stop_all(&state).await;
    terminal::close_all(&state)?;
    *state.documents.lock().map_err(lock_error)? = DocumentStore::default();
    lsp::clear_diagnostics(&app, &state)?;
    *state.workspace.lock().map_err(lock_error)? = Some(workspace.clone());
    if let Err(error) = workspace_watcher::start(&app, &state, workspace.root.clone()) {
        tracing::warn!(%error, "workspace file watcher unavailable");
    }
    settings::record_recent_workspace(&state, &workspace)?;
    emit_event(
        &app,
        LuxEvent::WorkspaceChanged {
            workspace: Some(workspace.clone()),
        },
    )?;
    Ok(workspace)
}

#[tauri::command]
async fn workspace_close(app: AppHandle, state: State<'_, SharedState>) -> Result<(), String> {
    workspace_watcher::stop(&state)?;
    *state.workspace.lock().map_err(lock_error)? = None;
    *state.documents.lock().map_err(lock_error)? = DocumentStore::default();
    lsp::clear_diagnostics(&app, &state)?;
    terminal::close_all(&state)?;
    lsp::shutdown(&state).await;
    debug::stop_all(&state).await;
    emit_event(&app, LuxEvent::WorkspaceChanged { workspace: None })?;
    Ok(())
}

#[tauri::command]
async fn workspace_pick_folder(app: AppHandle) -> Result<Option<WorkspaceInfo>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |folder| {
        let _ = sender.send(folder);
    });

    let Some(folder) = receiver.await.map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    let path = folder.into_path().map_err(|error| error.to_string())?;
    let workspace = lux_workspace::open_workspace(path).map_err(String::from)?;
    Ok(Some(workspace))
}

#[tauri::command]
fn fs_read_dir(path: PathBuf) -> Result<Vec<FsEntry>, String> {
    lux_fs::read_dir(path).map_err(String::from)
}

#[tauri::command]
async fn fs_read_tree(path: PathBuf) -> Result<Vec<FsEntry>, String> {
    tokio::task::spawn_blocking(move || lux_fs::read_tree(path))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
async fn fs_read_text(
    state: State<'_, SharedState>,
    path: PathBuf,
    max_bytes: Option<u64>,
) -> Result<FsReadTextResponse, String> {
    let max_bytes = max_bytes.unwrap_or(AI_READ_TEXT_MAX_BYTES).max(1);
    let path = resolve_workspace_path(&state, &path)?;
    tokio::task::spawn_blocking(move || -> Result<FsReadTextResponse, String> {
        let metadata = std::fs::metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err("path is not a file".to_string());
        }

        let size = metadata.len();
        let limit = usize::try_from(max_bytes.min(size)).unwrap_or(usize::MAX);
        let mut file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
        let mut buffer = vec![0; limit];
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        buffer.truncate(read);
        let text = String::from_utf8_lossy(&buffer).into_owned();

        Ok(FsReadTextResponse {
            path,
            text,
            truncated: size > max_bytes,
            size,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn fs_list_files(
    state: State<'_, SharedState>,
    max_results: Option<usize>,
) -> Result<Vec<FsEntry>, String> {
    let root = state
        .workspace
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .map(|workspace| workspace.root.clone())
        .ok_or_else(|| "no workspace is open".to_string())?;

    tokio::task::spawn_blocking(move || lux_fs::list_files(root, max_results.unwrap_or(2_500)))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
fn fs_create_file(app: AppHandle, path: PathBuf) -> Result<(), String> {
    lux_fs::create_file(&path).map_err(String::from)?;
    emit_event(&app, LuxEvent::FsChanged { path })?;
    Ok(())
}

#[tauri::command]
fn fs_create_dir(app: AppHandle, path: PathBuf) -> Result<(), String> {
    lux_fs::create_dir(&path).map_err(String::from)?;
    emit_event(&app, LuxEvent::FsChanged { path })?;
    Ok(())
}

#[tauri::command]
fn fs_rename(app: AppHandle, from: PathBuf, to: PathBuf) -> Result<(), String> {
    lux_fs::rename(&from, &to).map_err(String::from)?;
    emit_event(&app, LuxEvent::FsChanged { path: from })?;
    emit_event(&app, LuxEvent::FsChanged { path: to })?;
    Ok(())
}

#[tauri::command]
fn fs_copy(app: AppHandle, from: PathBuf, to: PathBuf) -> Result<(), String> {
    lux_fs::copy_path(&from, &to).map_err(String::from)?;
    emit_event(&app, LuxEvent::FsChanged { path: to })?;
    Ok(())
}

#[tauri::command]
fn fs_delete(app: AppHandle, path: PathBuf) -> Result<(), String> {
    lux_fs::delete(&path).map_err(String::from)?;
    emit_event(&app, LuxEvent::FsChanged { path })?;
    Ok(())
}

#[tauri::command]
fn fs_reveal_in_file_explorer(path: PathBuf) -> Result<(), String> {
    lux_fs::reveal_in_file_explorer(path).map_err(String::from)
}

#[tauri::command]
async fn ai_chat_completion(
    request: ai_chat_backend::AiChatCompletionRequest,
) -> Result<ai_chat_backend::AiChatCompletionResponse, String> {
    ai_chat_backend::completion(request).await
}

#[tauri::command]
fn ai_chat_history_load(app: AppHandle) -> Result<ai_chat_backend::AiChatHistoryResponse, String> {
    ai_chat_backend::history_load(&app)
}

#[tauri::command]
fn ai_chat_history_save(
    app: AppHandle,
    request: ai_chat_backend::AiChatHistorySaveRequest,
) -> Result<ai_chat_backend::AiChatHistoryResponse, String> {
    ai_chat_backend::history_save(&app, request)
}

#[tauri::command]
async fn ai_provider_diagnostic(
    request: ai_chat_backend::AiChatCompletionRequest,
) -> Result<ai_chat_backend::AiProviderDiagnosticResponse, String> {
    ai_chat_backend::provider_diagnostic(request).await
}

#[tauri::command]
async fn ai_chat_completion_stream(
    app: AppHandle,
    state: State<'_, SharedState>,
    request: ai_chat_backend::AiChatCompletionStreamRequest,
) -> Result<ai_chat_backend::AiChatCompletionStreamResponse, String> {
    let stream_id = request.resolved_stream_id();
    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    state
        .ai_streams
        .lock()
        .map_err(lock_error)?
        .insert(stream_id.clone(), cancel_tx);

    let state = state.inner().clone();
    let stream_id_for_task = stream_id.clone();
    tauri::async_runtime::spawn(async move {
        ai_chat_backend::run_completion_stream(app, stream_id_for_task.clone(), request, cancel_rx)
            .await;
        let _ = state
            .ai_streams
            .lock()
            .map(|mut streams| streams.remove(&stream_id_for_task));
    });

    Ok(ai_chat_backend::AiChatCompletionStreamResponse::new(
        stream_id,
    ))
}

#[tauri::command]
async fn ai_chat_completion_stream_cancel(
    state: State<'_, SharedState>,
    stream_id: String,
) -> Result<(), String> {
    let cancel = state
        .ai_streams
        .lock()
        .map_err(lock_error)?
        .remove(&stream_id);
    if let Some(cancel) = cancel {
        let _ = cancel.send(());
    }
    Ok(())
}

#[tauri::command]
async fn web_fetch(
    url: String,
    max_bytes: Option<u64>,
    timeout_secs: Option<u64>,
    allow_private_hosts: Option<bool>,
) -> Result<web_fetch::WebFetchResponse, String> {
    web_fetch::fetch(url, max_bytes, timeout_secs, allow_private_hosts).await
}

#[tauri::command]
async fn test_health(
    state: State<'_, SharedState>,
) -> Result<test_health::TestHealthResponse, String> {
    let root = state
        .workspace
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .map(|workspace| workspace.root.clone())
        .ok_or_else(|| "no workspace is open".to_string())?;

    test_health::run(root).await
}

#[tauri::command]
#[allow(
    clippy::unnecessary_wraps,
    reason = "Tauri commands use Result for a stable IPC error ABI"
)]
fn voice_input_status(
    provider: String,
    command: Option<String>,
    model_path: Option<PathBuf>,
) -> Result<voice_input::VoiceInputProviderStatus, String> {
    Ok(voice_input::status(provider, command, model_path))
}

#[tauri::command]
async fn voice_transcribe_local(
    request: voice_input::VoiceTranscriptionRequest,
) -> Result<voice_input::VoiceTranscriptionResult, String> {
    voice_input::transcribe_local(request).await
}

#[tauri::command]
async fn lsp_rename(
    app: AppHandle,
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    line: u32,
    column: u32,
    new_name: String,
) -> Result<WorkspaceEditResult, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(WorkspaceEditResult {
            edited_documents: Vec::new(),
            changed_paths: Vec::new(),
        });
    };
    let Some(edit) = manager
        .rename(&document, line, column, new_name)
        .await
        .map_err(String::from)?
    else {
        return Ok(WorkspaceEditResult {
            edited_documents: Vec::new(),
            changed_paths: Vec::new(),
        });
    };
    drop(lsp);
    editor::apply_workspace_edit(&app, &state, edit).await
}

pub fn run() {
    let state = Arc::new(AppState::default());

    tauri::Builder::default()
        .plugin(log_plugin())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .manage(state)
        .setup(|app| {
            let handle = app.handle();
            let lsp_handle = handle.clone();
            let terminal_handle = handle.clone();
            let terminal_service =
                Arc::new(TerminalService::new(Arc::new(move |session_id, data| {
                    let _ = terminal_handle
                        .emit("lux://event", LuxEvent::TerminalOutput { session_id, data });
                })));
            let settings_path = handle
                .path()
                .app_config_dir()
                .map_err(Box::<dyn std::error::Error>::from)?
                .join("settings.json");
            let state = app.state::<SharedState>();
            *state
                .settings
                .lock()
                .map_err(|_| "settings lock poisoned")? = Some(SettingsStore::load(settings_path)?);
            *state
                .terminals
                .lock()
                .map_err(|_| "terminals lock poisoned")? = Some(terminal_service);
            let (diagnostics_tx, mut diagnostics_rx) = tokio::sync::mpsc::unbounded_channel();
            let (debug_tx, mut debug_rx) = tokio::sync::mpsc::unbounded_channel();
            *state.lsp.blocking_lock() = Some(lux_lsp::LspManager::new(diagnostics_tx));
            *state.debug.blocking_lock() = Some(lux_dap::DebugSessionManager::new(debug_tx));
            let diagnostics_state = state.inner().clone();
            tauri::async_runtime::spawn(async move {
                while let Some(update) = diagnostics_rx.recv().await {
                    let _ = lsp::apply_diagnostics_update(&lsp_handle, &diagnostics_state, update);
                }
            });
            let debug_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                while let Some(update) = debug_rx.recv().await {
                    let _ = debug::apply_debug_update(&debug_handle, update);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            workspace_open,
            workspace_close,
            workspace_pick_folder,
            fs_read_dir,
            fs_read_tree,
            fs_read_text,
            fs_list_files,
            fs_create_file,
            fs_create_dir,
            fs_rename,
            fs_copy,
            fs_delete,
            fs_reveal_in_file_explorer,
            file_supported_formats,
            file_inspect,
            file_asset_data,
            file_open_external,
            editor_open_file,
            editor_new_file,
            editor_update_text,
            editor_apply_edits,
            editor_apply_workspace_edit,
            editor_save_file,
            editor_save_file_as,
            search_query,
            ai_chat_completion,
            ai_chat_history_load,
            ai_chat_history_save,
            ai_provider_diagnostic,
            ai_chat_completion_stream,
            ai_chat_completion_stream_cancel,
            web_fetch,
            test_health,
            ai_file_write,
            ai_file_str_replace,
            ai_file_patch,
            ai_file_delete,
            ai_shell,
            ai_symbol_context,
            voice_input_status,
            voice_transcribe_local,
            terminal_create,
            terminal_write,
            terminal_resize,
            terminal_close,
            terminal_close_all,
            git_status,
            git_diff,
            extensions_list,
            extensions_activation_plan,
            extensions_activate,
            extensions_contribution_registry,
            extensions_command_routes,
            extensions_execute_command,
            debug_workspace_info,
            debug_start,
            debug_stop,
            debug_sessions,
            debug_stack_trace,
            debug_scopes,
            debug_variables,
            debug_evaluate,
            debug_execute,
            debug_set_breakpoints,
            lsp_servers,
            diagnostics_snapshot,
            lsp_hover,
            lsp_definition,
            lsp_references,
            lsp_document_symbols,
            lsp_workspace_symbols,
            lsp_folding_ranges,
            lsp_inlay_hints,
            lsp_semantic_tokens,
            lsp_rename,
            lsp_completion,
            lsp_code_actions,
            lsp_format_document,
            lsp_format_range,
            lsp_signature_help,
            recent_workspaces,
            recent_workspace_forget,
            settings_get,
            settings_set,
            keybindings_get,
            keybindings_set,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Lux IDE");
}

fn log_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    let builder = tauri_plugin_log::Builder::new().level(if cfg!(debug_assertions) {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    });

    if cfg!(debug_assertions) {
        builder.build()
    } else {
        builder
            .clear_targets()
            .target(Target::new(TargetKind::LogDir { file_name: None }))
            .build()
    }
}

fn emit_event(app: &AppHandle, event: LuxEvent) -> Result<(), String> {
    app.emit("lux://event", event)
        .map_err(|error| error.to_string())
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

fn resolve_workspace_path(state: &State<'_, SharedState>, path: &Path) -> Result<PathBuf, String> {
    let root = workspace_root(state)?;
    resolve_workspace_path_from_root(&root, path, true)
}

fn resolve_workspace_path_for_write(
    state: &State<'_, SharedState>,
    path: &Path,
) -> Result<PathBuf, String> {
    let root = workspace_root(state)?;
    resolve_workspace_path_from_root(&root, path, false)
}

fn resolve_workspace_path_from_root(
    root: &Path,
    path: &Path,
    must_exist: bool,
) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let resolved = if must_exist || candidate.exists() {
        candidate
            .canonicalize()
            .map_err(|error| error.to_string())?
    } else {
        let resolved = normalize_path_lexically(&candidate);
        let ancestor = nearest_existing_ancestor(&resolved)?
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if !path_starts_with(&ancestor, &root) {
            return Err(format!(
                "path is outside the workspace: {}",
                resolved.display()
            ));
        }
        resolved
    };
    if !path_starts_with(&resolved, &root) {
        return Err(format!(
            "path is outside the workspace: {}",
            resolved.display()
        ));
    }
    Ok(resolved)
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    normalized
}

fn nearest_existing_ancestor(path: &Path) -> Result<PathBuf, String> {
    let mut current = path;
    loop {
        if current.exists() {
            return Ok(current.to_path_buf());
        }
        current = current
            .parent()
            .ok_or_else(|| format!("invalid path: {}", path.display()))?;
    }
}

fn path_starts_with(path: &Path, root: &Path) -> bool {
    if path.starts_with(root) {
        return true;
    }

    #[cfg(windows)]
    {
        let path_text = comparable_windows_path(path);
        let root_text = comparable_windows_path(root);
        path_text == root_text || path_text.starts_with(&format!("{root_text}/"))
    }

    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn comparable_windows_path(path: &Path) -> String {
    let mut text = path.to_string_lossy().replace('\\', "/").to_lowercase();
    if let Some(rest) = text.strip_prefix("//?/") {
        text = rest.to_string();
    }
    if let Some(rest) = text.strip_prefix("//./") {
        text = rest.to_string();
    }
    text.trim_end_matches('/').to_string()
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> String {
    "application state lock poisoned".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn workspace_path_guard_accepts_relative_paths_inside_root() {
        let root = std::env::temp_dir().join(format!("lux-path-guard-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("src")).unwrap();

        let resolved =
            resolve_workspace_path_from_root(&root, Path::new("src/new-file.ts"), false).unwrap();

        assert_eq!(
            resolved,
            root.canonicalize().unwrap().join("src").join("new-file.ts")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_path_guard_accepts_new_nested_write_paths() {
        let root = std::env::temp_dir().join(format!("lux-path-guard-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();

        let resolved =
            resolve_workspace_path_from_root(&root, Path::new("src/generated/new-file.ts"), false)
                .unwrap();

        assert_eq!(
            resolved,
            root.canonicalize()
                .unwrap()
                .join("src")
                .join("generated")
                .join("new-file.ts")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_path_guard_rejects_paths_outside_root() {
        let root = std::env::temp_dir().join(format!("lux-path-guard-{}", Uuid::new_v4()));
        let sibling = root.with_file_name(format!(
            "{}-sibling",
            root.file_name().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();

        let outside_relative = PathBuf::from("..")
            .join(sibling.file_name().unwrap())
            .join("file.ts");
        let outside_absolute = sibling.join("file.ts");
        std::fs::write(&outside_absolute, "text").unwrap();

        assert!(resolve_workspace_path_from_root(&root, &outside_relative, false).is_err());
        assert!(resolve_workspace_path_from_root(&root, &outside_absolute, true).is_err());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(sibling);
    }

    #[test]
    fn shell_cwd_guard_accepts_workspace_directory() {
        let root = std::env::temp_dir().join(format!("lux-shell-guard-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("scripts")).unwrap();

        let resolved = resolve_workspace_path_from_root(&root, Path::new("scripts"), true).unwrap();

        assert_eq!(resolved, root.canonicalize().unwrap().join("scripts"));
        let _ = std::fs::remove_dir_all(root);
    }
}
