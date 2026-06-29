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

mod agent_browser;
mod code_graph;
mod database;
mod debug;
mod editor;
mod extensions;
mod file_intel;
mod git;
mod lsp;
mod lsp_install;
mod mcp;
mod media_intel;
mod memory;
mod research;
mod runtime_provision;
mod search;
mod settings;
mod skills;
mod ssh;
mod terminal;
mod test_health;
mod updater;
mod voice_input;
mod web_fetch;
mod workspace_watcher;

mod ai_a2a;
mod ai_anthropic;
mod ai_chat_backend;
mod ai_checkpoint;
mod ai_compaction;
mod ai_context_sources;
mod ai_goal_eval;
mod ai_permissions;
mod ai_prompt;
mod ai_related;
mod ai_semantic;
mod ai_session;
mod ai_shell_safety;
mod ai_tokens;
mod ai_tool_defs;
mod ai_tools;
mod ai_turn;
mod ai_vision;
mod ai_workspace;

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

use agent_browser::{
    agent_browser_dashboard, agent_browser_install, agent_browser_invoke, agent_browser_read_image,
    agent_browser_skills, agent_browser_status, agent_browser_stream_status,
};
use ai_tools::{
    ai_file_delete, ai_file_patch, ai_file_str_replace, ai_file_write, ai_shell, ai_shell_classify,
    ai_symbol_context,
};
use code_graph::{code_graph_build, code_graph_export_html, code_graph_query, code_graph_status};
use database::{database_execute_sql, database_list_tables, database_update_cell};
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
use file_intel::{
    file_asset_data, file_inspect, file_media_ai_context, file_open_external,
    file_supported_formats,
};
use git::{
    git_branches, git_checkout_branch, git_commit, git_create_branch, git_diff, git_discard,
    git_file_diff, git_pull, git_push, git_stage, git_status, git_unstage,
};
use lsp::{
    diagnostics_snapshot, lsp_code_actions, lsp_completion, lsp_definition, lsp_document_symbols,
    lsp_folding_ranges, lsp_format_document, lsp_format_range, lsp_hover, lsp_inlay_hints,
    lsp_references, lsp_semantic_tokens, lsp_servers, lsp_signature_help, lsp_workspace_symbols,
};
use search::search_query;
use settings::{
    keybindings_get, keybindings_set, recent_workspace_forget, recent_workspaces,
    set_scan_concurrency, settings_get, settings_set,
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
    /// In-memory table of live SSH connection profiles for the AI `Ssh*` tools.
    /// No long-running remote process or credential is held — each command runs a
    /// fresh non-interactive `ssh`, so this is just routing + sticky-cwd state.
    ssh: lux_ssh::SshRegistry,
    code_graph: tokio::sync::Mutex<Option<lux_codegraph::Index>>,
    /// Currently-open per-project memory store, keyed by its on-disk path. Reopened
    /// lazily when the active workspace (hence the db path) changes, so each project
    /// gets its own isolated memory backend.
    memory: tokio::sync::Mutex<
        Option<(
            std::path::PathBuf,
            std::sync::Arc<std::sync::Mutex<lux_memory::MemoryStore>>,
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
    let workspace = lux_workspace::open_workspace(path).map_err(String::from)?;
    workspace_watcher::stop(&state)?;
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
    if let Err(error) = workspace_watcher::start(&app, &state, workspace.root.clone(), generation) {
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
        LuxEvent::WorkspaceChanged {
            workspace: Some(workspace.clone()),
        },
    )?;
    Ok(workspace)
}

#[tauri::command]
async fn workspace_close(app: AppHandle, state: State<'_, SharedState>) -> Result<(), String> {
    workspace_watcher::stop(&state)?;
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
            ai_checkpoint::clear_workspace(&workspace.root.to_string_lossy());
        }
    }
    ai_session::clear_all();
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

// Every raw FS command resolves caller-supplied paths through the workspace guard
// before touching disk: sources (must already exist) via `resolve_workspace_path`,
// new/destination paths via `resolve_workspace_path_for_write`. This confines the
// renderer/extension/AI surface to the open workspace and rejects absolute or
// `..`-escaping targets, instead of forwarding raw `PathBuf`s straight to `lux_fs`.
#[tauri::command]
fn fs_create_file(app: AppHandle, state: State<'_, SharedState>, path: PathBuf) -> Result<(), String> {
    let path = resolve_workspace_path_for_write(&state, &path)?;
    lux_fs::create_file(&path).map_err(String::from)?;
    emit_event(&app, LuxEvent::FsChanged { path })?;
    Ok(())
}

#[tauri::command]
fn fs_create_dir(app: AppHandle, state: State<'_, SharedState>, path: PathBuf) -> Result<(), String> {
    let path = resolve_workspace_path_for_write(&state, &path)?;
    lux_fs::create_dir(&path).map_err(String::from)?;
    emit_event(&app, LuxEvent::FsChanged { path })?;
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
    lux_fs::rename(&from, &to).map_err(String::from)?;
    emit_event(&app, LuxEvent::FsChanged { path: from })?;
    emit_event(&app, LuxEvent::FsChanged { path: to })?;
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
    lux_fs::copy_path(&from, &to).map_err(String::from)?;
    emit_event(&app, LuxEvent::FsChanged { path: to })?;
    Ok(())
}

#[tauri::command]
fn fs_delete(app: AppHandle, state: State<'_, SharedState>, path: PathBuf) -> Result<(), String> {
    let path = resolve_workspace_path(&state, &path)?;
    lux_fs::delete(&path).map_err(String::from)?;
    emit_event(&app, LuxEvent::FsChanged { path })?;
    Ok(())
}

#[tauri::command]
fn fs_reveal_in_file_explorer(
    state: State<'_, SharedState>,
    path: PathBuf,
) -> Result<(), String> {
    let path = resolve_workspace_path(&state, &path)?;
    lux_fs::reveal_in_file_explorer(path).map_err(String::from)
}

#[tauri::command]
async fn ai_chat_completion(
    request: ai_chat_backend::AiChatCompletionRequest,
) -> Result<ai_chat_backend::AiChatCompletionResponse, String> {
    ai_chat_backend::completion(request, |_| {}).await
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
) -> Result<web_fetch::WebFetchResponse, String> {
    // No private-host bypass is exposed: the SSRF guard is always on (H1).
    web_fetch::fetch(url, max_bytes, timeout_secs).await
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
                                store.get(lux_core::SettingsScope::User, mcp::MCP_SERVERS_KEY)
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
            file_media_ai_context,
            file_asset_data,
            ai_vision::ai_vision_encode,
            file_open_external,
            database_list_tables,
            database_execute_sql,
            database_update_cell,
            agent_browser_status,
            agent_browser_invoke,
            agent_browser_install,
            agent_browser_read_image,
            agent_browser_stream_status,
            agent_browser_dashboard,
            agent_browser_skills,
            editor_open_file,
            editor_new_file,
            editor_update_text,
            editor_apply_edits,
            editor_apply_workspace_edit,
            editor_save_file,
            editor_save_file_as,
            search_query,
            ai_chat_completion,
            ai_chat_backend::ai_list_provider_models,
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
            ai_goal_eval::ai_goal_eval_verdict,
            ai_compaction::ai_compaction_summary,
            ai_checkpoint::ai_checkpoint,
            ai_context_sources::ai_rules_context,
            ai_context_sources::ai_docs_context,
            ai_context_sources::ai_memory_context,
            memory::memory_create,
            memory::memory_search,
            memory::memory_get,
            memory::memory_update,
            memory::memory_delete,
            memory::memory_list,
            memory::memory_stats,
            memory::memory_wipe,
            skills::skills_list,
            skills::skills_get,
            skills::skills_match,
            skills::skills_save,
            skills::skills_delete,
            skills::skills_set_enabled,
            skills::skills_discover_importable,
            skills::skills_import,
            research::web_research,
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
            ai_a2a::ai_blackboard_post,
            ai_a2a::ai_blackboard_read,
            ai_a2a::ai_blackboard_clear,
            ai_permissions::ai_permission_decide,
            ai_prompt::ai_build_system_prompt,
            ai_related::ai_related_files,
            ai_semantic::ai_semantic_search,
            ai_tokens::ai_estimate_tokens,
            ai_tokens::ai_estimate_tokens_batch,
            ai_tokens::ai_format_tokens,
            ai_session::ai_session_goal_get,
            ai_session::ai_session_goal_set,
            ai_session::ai_session_todos_get,
            ai_session::ai_session_todos_set,
            ai_session::ai_session_dispose,
            ai_turn::ai_run_turn,
            ai_turn::ai_resolve_turn_approval,
            ai_turn::ai_resolve_turn_question,
            ai_turn::ai_cancel_turn,
            ai_turn::ai_inject_message,
            ai_workspace::ai_repo_map,
            ai_workspace::ai_workspace_index,
            ai_symbol_context,
            code_graph_build,
            code_graph_export_html,
            code_graph_query,
            code_graph_status,
            voice_input_status,
            voice_transcribe_local,
            terminal_create,
            terminal_write,
            terminal_resize,
            terminal_close,
            terminal_close_all,
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
            lsp_install::lsp_server_catalog,
            lsp_install::lsp_install_server,
            runtime_provision::runtime_catalog,
            runtime_provision::runtime_provision,
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
            set_scan_concurrency,
            keybindings_get,
            keybindings_set,
            updater::update_check,
            updater::update_install,
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
