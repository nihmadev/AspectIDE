#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]
#![allow(
    clippy::items_after_statements,
    clippy::large_stack_frames,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value,
    clippy::option_if_let_else,
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

mod aspector;
mod files;
mod network;
mod platform;
mod services;
mod storage;
mod system;

use aspect_core::{
    BufferId, FsEntry, AspectEvent, WorkspaceDiagnostic, WorkspaceEditResult, WorkspaceInfo,
};
use aspect_editor::DocumentStore;
use aspect_settings::SettingsStore;
use aspect_terminal::TerminalService;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_log::{log::LevelFilter, Target, TargetKind};
use tokio::sync::oneshot;

use aspector::tools::executors::{
    ai_file_delete, ai_file_patch, ai_file_str_replace, ai_file_write, ai_shell, ai_shell_classify,
    ai_symbol_context,
};
use platform::extensions::{
    extensions_activate, extensions_activation_plan, extensions_command_routes,
    extensions_contribution_registry, extensions_execute_command, extensions_list,
};
use services::search::search_query;
use services::git::{
    git_branches, git_checkout_branch, git_commit, git_create_branch, git_diff, git_discard,
    git_file_diff, git_pull, git_push, git_stage, git_status, git_unstage,
};

use crate::network::mcp;
use crate::network::research;
use crate::network::ssh;
use crate::storage::settings;
use crate::storage::memory;
use crate::platform::skills;
use crate::system::updater;
use crate::platform::agent_browser;
use crate::storage::database;
use crate::files::file_intel;
use crate::system::fonts;
use crate::services::code_graph;
use crate::services::debug;
use crate::services::editor;
use crate::services::lsp;
use crate::services::terminal;
use crate::platform::runtimes;

const AI_READ_TEXT_MAX_BYTES: u64 = 1_000_000;

#[derive(Default)]
struct AppState {
    workspace: Mutex<Option<WorkspaceInfo>>,
    workspace_watcher: Mutex<Option<system::watcher::WorkspaceWatcher>>,
    documents: Mutex<DocumentStore>,
    diagnostics: Mutex<Vec<WorkspaceDiagnostic>>,
    ai_streams: Mutex<BTreeMap<String, oneshot::Sender<()>>>,
    lsp: tokio::sync::Mutex<Option<aspect_lsp::LspManager>>,
    debug: tokio::sync::Mutex<Option<aspect_dap::DebugSessionManager>>,
    settings: Mutex<Option<SettingsStore>>,
    terminals: Mutex<Option<Arc<TerminalService>>>,
    /// In-memory table of live SSH connection profiles for the AI `Ssh*` tools.
    /// No long-running remote process or credential is held — each command runs a
    /// fresh non-interactive `ssh`, so this is just routing + sticky-cwd state.
    ssh: aspect_ssh::SshRegistry,
    code_graph: tokio::sync::Mutex<Option<aspect_codegraph::Index>>,
    /// Currently-open per-project memory store, keyed by its on-disk path. Reopened
    /// lazily when the active workspace (hence the db path) changes, so each project
    /// gets its own isolated memory backend.
    memory: tokio::sync::Mutex<
        Option<(
            std::path::PathBuf,
            std::sync::Arc<std::sync::Mutex<aspect_memory::MemoryStore>>,
        )>,
    >,
    /// Bumped on every workspace open/close. A background graph build captures the
    /// value at start and only commits its result if it still matches — so a build
    /// for a workspace that was since closed or replaced is discarded, never
    /// overwriting the current one.
    workspace_generation: std::sync::atomic::AtomicU64,
    /// Single-flight slot for full code-graph builds: holds the workspace
    /// generation currently being built (`0` = none). Prevents the open-time
    /// background build and a manual rebuild from doing duplicate full walks for
    /// the same workspace. See `code_graph::BuildGuard`.
    code_graph_building_gen: std::sync::atomic::AtomicU64,
    /// Coalescing buffer for incremental code-graph updates. While one incremental
    /// rebuild runs (it takes the index out of `code_graph`), file-watch batches that
    /// arrive in the gap stash their paths here instead of being dropped; the active
    /// rebuild drains and re-applies them on completion. See
    /// `code_graph::handle_fs_batch`.
    code_graph_pending_paths: std::sync::Mutex<Vec<PathBuf>>,
    /// True while an incremental code-graph update is in flight. Distinguishes "index
    /// is `None` because a rebuild took it" from "index is `None` because nothing is
    /// built yet", so a concurrent batch can coalesce instead of bailing out.
    code_graph_updating: std::sync::atomic::AtomicBool,
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
    let workspace = aspect_workspace::open_workspace(path).map_err(String::from)?;
    system::watcher::stop(&state)?;
    lsp::shutdown(&state).await;
    debug::stop_all(&state).await;
    terminal::close_all(&state)?;
    *state.documents.lock().map_err(lock_error)? = DocumentStore::default();
    lsp::clear_diagnostics(&app, &state)?;
    *state.workspace.lock().map_err(lock_error)? = Some(workspace.clone());
    // Switching workspaces: invalidate any in-flight build and drop the old graph
    // so queries return "not built yet" rather than the previous workspace's data
    // during the rebuild window. Bump the generation BEFORE starting the watcher so
    // the watcher tags its batches with this workspace's generation (M5) — a watcher
    // started under the old generation would let late old-workspace events merge in.
    let generation = state
        .workspace_generation
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        + 1;
    if let Err(error) = system::watcher::start(&app, &state, workspace.root.clone(), generation) {
        tracing::warn!(%error, "workspace file watcher unavailable");
    }
    // Persist the outgoing workspace's parse cache (capturing this session's
    // incremental edits) before dropping the graph, so reopening it is fast.
    code_graph::persist_and_drop(state.inner()).await;
    // Kick off a background code-graph build — it streams progress on
    // `lux://code-graph`, never blocks workspace load, and only commits if this
    // generation is still current when it finishes.
    code_graph::start_build_on_workspace(
        app.clone(),
        state.inner().clone(),
        workspace.root.clone(),
        generation,
    );
    settings::record_recent_workspace(&state, &workspace)?;
    emit_event(
        &app,
        AspectEvent::WorkspaceChanged {
            workspace: Some(workspace.clone()),
        },
    )?;
    Ok(workspace)
}

#[tauri::command]
async fn workspace_close(app: AppHandle, state: State<'_, SharedState>) -> Result<(), String> {
    system::watcher::stop(&state)?;
    // Invalidate any in-flight build and drop the graph so the AI tools don't
    // serve symbols from a workspace that is no longer open.
    state
        .workspace_generation
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    // Persist this session's parse cache before dropping the graph so the next open
    // of this workspace reuses it.
    code_graph::persist_and_drop(state.inner()).await;
    // Release in-memory per-session/per-workspace AI state for the closing
    // workspace so it doesn't accumulate for the life of the process.
    if let Ok(guard) = state.workspace.lock() {
        if let Some(workspace) = guard.as_ref() {
            aspector::session::checkpoint::clear_workspace(&workspace.root.to_string_lossy());
        }
    }
    aspector::session::store::clear_all();
    *state.workspace.lock().map_err(lock_error)? = None;
    *state.documents.lock().map_err(lock_error)? = DocumentStore::default();
    lsp::clear_diagnostics(&app, &state)?;
    terminal::close_all(&state)?;
    lsp::shutdown(&state).await;
    debug::stop_all(&state).await;
    emit_event(&app, AspectEvent::WorkspaceChanged { workspace: None })?;
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
    let workspace = aspect_workspace::open_workspace(path).map_err(String::from)?;
    Ok(Some(workspace))
}

/// Open the native OS multi-file picker for chat attachments and return the
/// selected absolute paths. Empty when the user cancels. The frontend reads each
/// path via `read_external_file` and falls back to the HTML `<input type=file>`
/// when the native dialog is unavailable.
#[tauri::command]
async fn pick_attachment_files(app: AppHandle) -> Result<Vec<String>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_files(move |files| {
        let _ = sender.send(files);
    });
    let Some(files) = receiver.await.map_err(|error| error.to_string())? else {
        return Ok(Vec::new());
    };
    let paths = files
        .into_iter()
        .filter_map(|file| file.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    Ok(paths)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ClipboardImageData {
    width: u32,
    height: u32,
    rgba_base64: String,
}

/// Read an image from the OS clipboard as raw RGBA (base64). Used as a paste
/// fallback where the webview `ClipboardEvent` yields no image (Linux
/// `WebKitGTK`). Returns `None` when the clipboard holds no image; the frontend
/// encodes the RGBA to PNG via a canvas.
#[tauri::command]
async fn clipboard_read_image(app: AppHandle) -> Result<Option<ClipboardImageData>, String> {
    use base64::Engine as _;
    match app.clipboard().read_image() {
        Ok(image) => {
            let (width, height) = (image.width(), image.height());
            // Bound the base64 payload shipped over IPC (~40 MiB raw ceiling).
            if u64::from(width) * u64::from(height) * 4 > 40 * 1024 * 1024 {
                return Err("clipboard image too large".to_string());
            }
            Ok(Some(ClipboardImageData {
                width,
                height,
                rgba_base64: base64::engine::general_purpose::STANDARD.encode(image.rgba()),
            }))
        }
        Err(_) => Ok(None),
    }
}

#[tauri::command]
fn fs_read_dir(path: PathBuf) -> Result<Vec<FsEntry>, String> {
    aspect_fs::read_dir(path).map_err(String::from)
}

#[tauri::command]
async fn fs_read_tree(path: PathBuf) -> Result<Vec<FsEntry>, String> {
    tokio::task::spawn_blocking(move || aspect_fs::read_tree(path))
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
        let limit = max_bytes.min(size);
        let file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
        // Read the full requested range: a single `read()` call may legally return
        // fewer bytes than asked (observed with large files), which silently
        // truncated the text mid-file for every consumer of this command.
        let mut buffer = Vec::with_capacity(usize::try_from(limit).unwrap_or(0));
        file.take(limit)
            .read_to_end(&mut buffer)
            .map_err(|error| error.to_string())?;
        // A byte cap can split a multi-byte UTF-8 sequence; drop a trailing
        // incomplete sequence (only when truncated) so the text ends cleanly.
        if limit < size {
            if let Err(error) = std::str::from_utf8(&buffer) {
                if error.error_len().is_none() {
                    buffer.truncate(error.valid_up_to());
                }
            }
        }
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

    tokio::task::spawn_blocking(move || aspect_fs::list_files(root, max_results.unwrap_or(2_500)))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

// Every raw FS command resolves caller-supplied paths through the workspace guard
// before touching disk: sources (must already exist) via `resolve_workspace_path`,
// new/destination paths via `resolve_workspace_path_for_write`. This confines the
// renderer/extension/AI surface to the open workspace and rejects absolute or
// `..`-escaping targets, instead of forwarding raw `PathBuf`s straight to `aspect_fs`.
#[tauri::command]
fn fs_create_file(
    app: AppHandle,
    state: State<'_, SharedState>,
    path: PathBuf,
) -> Result<(), String> {
    let path = resolve_workspace_path_for_write(&state, &path)?;
    aspect_fs::create_file(&path).map_err(String::from)?;
    emit_event(&app, AspectEvent::FsChanged { path })?;
    Ok(())
}

#[tauri::command]
fn fs_create_dir(
    app: AppHandle,
    state: State<'_, SharedState>,
    path: PathBuf,
) -> Result<(), String> {
    let path = resolve_workspace_path_for_write(&state, &path)?;
    aspect_fs::create_dir(&path).map_err(String::from)?;
    emit_event(&app, AspectEvent::FsChanged { path })?;
    Ok(())
}

#[tauri::command]
fn fs_rename(
    app: AppHandle,
    state: State<'_, SharedState>,
    from: PathBuf,
    to: PathBuf,
) -> Result<(), String> {
    // Source must exist inside the workspace; destination is a new path inside it.
    let from = resolve_workspace_path(&state, &from)?;
    let to = resolve_workspace_path_for_write(&state, &to)?;
    aspect_fs::rename(&from, &to).map_err(String::from)?;
    emit_event(&app, AspectEvent::FsChanged { path: from })?;
    emit_event(&app, AspectEvent::FsChanged { path: to })?;
    Ok(())
}

#[tauri::command]
fn fs_copy(
    app: AppHandle,
    state: State<'_, SharedState>,
    from: PathBuf,
    to: PathBuf,
) -> Result<(), String> {
    let from = resolve_workspace_path(&state, &from)?;
    let to = resolve_workspace_path_for_write(&state, &to)?;
    aspect_fs::copy_path(&from, &to).map_err(String::from)?;
    emit_event(&app, AspectEvent::FsChanged { path: to })?;
    Ok(())
}

/// Imports one OS-dropped file into the workspace. The webview receives external
/// drag-drop as path-less `File` blobs (`dragDropEnabled=false` keeps HTML5 `DnD` for
/// internal drags), so the bytes travel over IPC as base64. Name collisions get
/// a " (n)" suffix instead of overwriting. Returns the final written path.
#[tauri::command]
fn fs_import_file(
    app: AppHandle,
    state: State<'_, SharedState>,
    path: PathBuf,
    contents_base64: String,
) -> Result<PathBuf, String> {
    use base64::Engine as _;
    let resolved = resolve_workspace_path_for_write(&state, &path)?;
    let contents = base64::engine::general_purpose::STANDARD
        .decode(contents_base64.as_bytes())
        .map_err(|error| format!("invalid file payload: {error}"))?;
    let target = unique_import_path(&resolved);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(&target, contents).map_err(|error| error.to_string())?;
    emit_event(
        &app,
        AspectEvent::FsChanged {
            path: target.clone(),
        },
    )?;
    Ok(target)
}

/// "report.pdf" → "report (1).pdf" → "report (2).pdf" … until free.
fn unique_import_path(path: &std::path::Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    for n in 1..10_000 {
        let candidate = parent.join(format!("{stem} ({n}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

#[tauri::command]
fn fs_delete(app: AppHandle, state: State<'_, SharedState>, path: PathBuf) -> Result<(), String> {
    let path = resolve_workspace_path(&state, &path)?;
    aspect_fs::delete(&path).map_err(String::from)?;
    emit_event(&app, AspectEvent::FsChanged { path })?;
    Ok(())
}

#[tauri::command]
fn fs_reveal_in_file_explorer(state: State<'_, SharedState>, path: PathBuf) -> Result<(), String> {
    let path = resolve_workspace_path(&state, &path)?;
    aspect_fs::reveal_in_file_explorer(path).map_err(String::from)
}

#[tauri::command]
async fn ai_chat_completion(
    request: aspector::transport::AiChatCompletionRequest,
) -> Result<aspector::transport::AiChatCompletionResponse, String> {
    aspector::transport::completion(request, |_| {}).await
}

#[tauri::command]
fn ai_chat_history_load(app: AppHandle) -> Result<aspector::transport::AiChatHistoryResponse, String> {
    aspector::transport::history_load_app(&app)
}

#[tauri::command]
fn ai_chat_history_save(
    app: AppHandle,
    request: aspector::transport::AiChatHistorySaveRequest,
) -> Result<aspector::transport::AiChatHistoryResponse, String> {
    aspector::transport::history_save_app(&app, request)
}

#[tauri::command]
async fn ai_provider_diagnostic(
    request: aspector::transport::AiChatCompletionRequest,
) -> Result<aspector::transport::AiProviderDiagnosticResponse, String> {
    aspector::transport::provider_diagnostic(request).await
}

#[tauri::command]
async fn ai_chat_completion_stream(
    app: AppHandle,
    state: State<'_, SharedState>,
    request: aspector::transport::AiChatCompletionStreamRequest,
) -> Result<aspector::transport::AiChatCompletionStreamResponse, String> {
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
        aspector::transport::run_completion_stream_app(app, stream_id_for_task.clone(), request, cancel_rx)
            .await;
        let _ = state
            .ai_streams
            .lock()
            .map(|mut streams| streams.remove(&stream_id_for_task));
    });

    Ok(aspector::transport::AiChatCompletionStreamResponse::new(
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
) -> Result<network::web_fetch::WebFetchResponse, String> {
    // No private-host bypass is exposed: the SSRF guard is always on (H1).
    network::web_fetch::fetch(url, max_bytes, timeout_secs).await
}

#[tauri::command]
async fn test_health(
    state: State<'_, SharedState>,
) -> Result<platform::test_health::TestHealthResponse, String> {
    let root = state
        .workspace
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .map(|workspace| workspace.root.clone())
        .ok_or_else(|| "no workspace is open".to_string())?;

    platform::test_health::run(root).await
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
) -> Result<system::voice::VoiceInputProviderStatus, String> {
    Ok(system::voice::status(provider, command, model_path))
}

#[tauri::command]
async fn voice_transcribe_local(
    request: system::voice::VoiceTranscriptionRequest,
) -> Result<system::voice::VoiceTranscriptionResult, String> {
    system::voice::transcribe_local(request).await
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
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(state)
        .setup(|app| {
            // The updater plugin deserializes `plugins.updater` into a required
            // struct at init and panics when it is absent. That section only
            // exists in CI-prepared release configs (injected by
            // prepare-release-config.mjs), not in source/dev builds — so register
            // the plugin only when its config is present. Without it, the
            // `update_check`/`update_install` commands degrade gracefully via
            // their `app.updater()` guards.
            if app.config().plugins.0.contains_key("updater") {
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())?;
            }

            let handle = app.handle();
            let lsp_handle = handle.clone();
            let terminal_handle = handle.clone();
            let terminal_service =
                Arc::new(TerminalService::new(Arc::new(move |session_id, data| {
                    let _ = terminal_handle
                        .emit("lux://event", AspectEvent::TerminalOutput { session_id, data });
                })));
            // Publish the app-data dir to the agent-browser resolver so it can find
            // the managed install and managed Node without an AppHandle.
            if let Ok(app_data) = handle.path().app_data_dir() {
                aspect_agent_browser::set_app_data_dir(app_data);
            }
            let app_config_dir = handle
                .path()
                .app_config_dir()
                .map_err(Box::<dyn std::error::Error>::from)?;
            // Always-on shell integration (PATH shim + Explorer verb) — quick
            // idempotent HKCU writes, but off the setup path regardless.
            std::thread::spawn(system::integration::apply_default_integration);
            system::integration::capture_startup_path();
            let settings_path = app_config_dir.join("settings.json");
            let state = app.state::<SharedState>();
            *state
                .settings
                .lock()
                .map_err(|_| "settings lock poisoned")? = Some(SettingsStore::load(settings_path)?);
            *state
                .terminals
                .lock()
                .map_err(|_| "terminals lock poisoned")? = Some(terminal_service);
            // Bring up enabled MCP servers in the background so their tools are live
            // for the agent without blocking app startup on the handshakes.
            {
                let mcp_state = state.inner().clone();
                tauri::async_runtime::spawn(async move {
                    let configs: Vec<mcp::McpServerConfig> = mcp_state
                        .settings
                        .lock()
                        .ok()
                        .and_then(|guard| {
                            guard.as_ref().and_then(|store| {
                                store.get(aspect_core::SettingsScope::User, mcp::MCP_SERVERS_KEY)
                            })
                        })
                        .and_then(|setting| serde_json::from_value(setting.value).ok())
                        .unwrap_or_default();
                    for config in configs.into_iter().filter(|config| config.enabled) {
                        let _ = mcp::connect_server(config).await;
                    }
                });
            }
            let (diagnostics_tx, mut diagnostics_rx) = tokio::sync::mpsc::unbounded_channel();
            let (debug_tx, mut debug_rx) = tokio::sync::mpsc::unbounded_channel();
            *state.lsp.blocking_lock() = Some(aspect_lsp::LspManager::new(diagnostics_tx));
            *state.debug.blocking_lock() = Some(aspect_dap::DebugSessionManager::new(debug_tx));
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
            pick_attachment_files,
            clipboard_read_image,
            fs_read_dir,
            fs_read_tree,
            fs_read_text,
            fs_list_files,
            fs_create_file,
            fs_create_dir,
            fs_rename,
            fs_copy,
            fs_delete,
            fs_import_file,
            fs_reveal_in_file_explorer,
            file_intel::file_supported_formats,
            file_intel::file_inspect,
            file_intel::file_media_ai_context,
            file_intel::file_asset_data,
            file_intel::read_external_file,
            aspector::context::vision::ai_vision_encode,
            file_intel::file_open_external,
            database::database_list_tables,
            database::database_execute_sql,
            database::database_update_cell,
            agent_browser::agent_browser_status,
            agent_browser::agent_browser_invoke,
            agent_browser::agent_browser_install,
            agent_browser::agent_browser_read_image,
            agent_browser::agent_browser_stream_status,
            agent_browser::agent_browser_dashboard,
            agent_browser::agent_browser_skills,
            editor::editor_open_file,
            editor::editor_new_file,
            editor::editor_update_text,
            editor::editor_apply_edits,
            editor::editor_apply_workspace_edit,
            editor::editor_save_file,
            editor::editor_save_file_as,
            search_query,
            ai_chat_completion,
            aspector::transport::ai_list_provider_models,
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
            ai_shell_classify,
            aspector::session::goal_eval::ai_goal_eval_verdict,
            aspector::session::compaction::ai_compaction_summary,
            aspector::session::checkpoint::ai_checkpoint,
            aspector::context::sources::ai_rules_context,
            aspector::context::sources::ai_docs_context,
            aspector::context::sources::ai_memory_context,
            memory::memory_create,
            memory::memory_search,
            memory::memory_get,
            memory::memory_update,
            memory::memory_delete,
            memory::memory_list,
            memory::memory_stats,
            memory::memory_wipe,
            memory::memory_prune,
            memory::memory_relate,
            memory::memory_unrelate,
            memory::memory_relations,
            memory::memory_related,
            memory::memory_retention,
            skills::skills_list,
            skills::skills_get,
            skills::skills_match,
            skills::skills_save,
            skills::skills_delete,
            skills::skills_set_enabled,
            skills::skills_discover_importable,
            skills::skills_import,
            research::web_research,
            research::multi_web_research,
            mcp::mcp_connect_all,
            mcp::mcp_connect,
            mcp::mcp_disconnect,
            mcp::mcp_status,
            mcp::mcp_call,
            mcp::mcp_add,
            mcp::mcp_remove,
            mcp::mcp_enable,
            ssh::ssh_connect,
            ssh::ssh_exec,
            ssh::ssh_transfer,
            ssh::ssh_list,
            ssh::ssh_disconnect,
            aspector::session::a2a::ai_blackboard_post,
            aspector::session::a2a::ai_blackboard_read,
            aspector::session::a2a::ai_blackboard_clear,
            aspector::tools::permissions::ai_permission_decide,
            aspector::context::prompt::ai_build_system_prompt,
            aspector::context::related::ai_related_files,
            aspector::context::semantic::ai_semantic_search,
            aspector::analysis::tokens::ai_estimate_tokens,
            aspector::analysis::tokens::ai_estimate_tokens_batch,
            aspector::analysis::tokens::ai_format_tokens,
            aspector::session::store::ai_session_goal_get,
            aspector::session::store::ai_session_goal_set,
            aspector::session::store::ai_session_todos_get,
            aspector::session::store::ai_session_todos_set,
            aspector::session::store::ai_session_dispose,
            aspector::turn::ai_run_turn,
            aspector::turn::ai_resolve_turn_approval,
            aspector::turn::ai_resolve_turn_question,
            aspector::turn::ai_cancel_turn,
            aspector::turn::ai_cancel_subagent,
            aspector::turn::ai_inject_message,
            aspector::gateway::aspect_link_start,
            aspector::gateway::aspect_link_poll,
            aspector::gateway::aspect_open_url,
            aspector::gateway::aspect_usage,
            aspector::context::workspace::ai_repo_map,
            aspector::context::workspace::ai_workspace_index,
            aspector::context::workspace::resolve_file_languages,
            aspector::context::workspace::ai_index_languages,
            ai_symbol_context,
            code_graph::code_graph_build,
            code_graph::code_graph_export_html,
            code_graph::code_graph_query,
            code_graph::code_graph_status,
            voice_input_status,
            voice_transcribe_local,
            terminal::terminal_create,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_close,
            terminal::terminal_close_all,
            git_status,
            git_diff,
            git_stage,
            git_unstage,
            git_discard,
            git_commit,
            git_push,
            git_pull,
            git_branches,
            git_checkout_branch,
            git_create_branch,
            git_file_diff,
            extensions_list,
            extensions_activation_plan,
            extensions_activate,
            extensions_contribution_registry,
            extensions_command_routes,
            extensions_execute_command,
            debug::debug_workspace_info,
            debug::debug_start,
            debug::debug_stop,
            debug::debug_sessions,
            debug::debug_stack_trace,
            debug::debug_scopes,
            debug::debug_variables,
            debug::debug_evaluate,
            debug::debug_execute,
            debug::debug_set_breakpoints,
            lsp::lsp_servers,
            runtimes::lsp_server_catalog,
            runtimes::lsp_install_server,
            runtimes::lsp_uninstall_server,
            runtimes::runtime_catalog,
            runtimes::runtime_provision,
            lsp::diagnostics_snapshot,
            lsp::lsp_hover,
            lsp::lsp_definition,
            lsp::lsp_references,
            lsp::lsp_document_symbols,
            lsp::lsp_workspace_symbols,
            lsp::lsp_folding_ranges,
            lsp::lsp_inlay_hints,
            lsp::lsp_semantic_tokens,
            lsp_rename,
            lsp::lsp_completion,
            lsp::lsp_code_actions,
            lsp::lsp_format_document,
            lsp::lsp_format_range,
            lsp::lsp_signature_help,
            settings::recent_workspaces,
            settings::recent_workspace_forget,
            settings::settings_get,
            settings::settings_set,
            fonts::list_system_font_families,
            settings::set_scan_concurrency,
            settings::keybindings_get,
            settings::keybindings_set,
            updater::update_check,
            updater::update_install,
            system::integration::startup_open_path,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Lux IDE")
        .run(|app_handle, event| {
            // On app exit, synchronously flush the code-graph parse cache so a
            // session's incremental edits survive to the next open. The normal
            // open/close path already persists; this covers a plain quit (the
            // window can be destroyed without a `workspace_close`). Best-effort.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                if let Some(state) = app_handle.try_state::<SharedState>() {
                    code_graph::flush_cache_blocking(state.inner());
                    // Kill debug-adapter children (codelldb/debugpy + the debuggee)
                    // on a plain quit: a window-destroy exit skips `workspace_close`,
                    // so without this they outlive the IDE (H9). `kill_on_drop` is
                    // only a backstop — the session map may not drop before exit.
                    tauri::async_runtime::block_on(debug::stop_all(&state));
                }
            }
        });
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

fn emit_event(app: &AppHandle, event: AspectEvent) -> Result<(), String> {
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
    // dunce, not std: on Windows std::fs::canonicalize returns a `\\?\` verbatim
    // path, which corrupts every downstream consumer that stringifies it — LSP
    // file URIs become `file:////%3F/...` (SymbolContext silently empty) and
    // permission-rule globs see `//?/E:/...`. dunce yields the plain `E:\...` form.
    let root = dunce::canonicalize(root).map_err(|error| error.to_string())?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let resolved = if must_exist || candidate.exists() {
        dunce::canonicalize(&candidate).map_err(|error| error.to_string())?
    } else {
        let resolved = normalize_path_lexically(&candidate);
        let ancestor = dunce::canonicalize(nearest_existing_ancestor(&resolved)?)
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
            dunce::canonicalize(&root)
                .unwrap()
                .join("src")
                .join("new-file.ts")
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
            dunce::canonicalize(&root)
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

        assert_eq!(
            resolved,
            dunce::canonicalize(&root).unwrap().join("scripts")
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
