use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    env,
    net::{IpAddr, ToSocketAddrs},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

use base64::Engine;
use chrono::Utc;
use futures_util::StreamExt;
use lux_core::{
    BufferId, DebugWorkspaceInfo, DocumentEditResult, DocumentSnapshot, ExtensionInfo, FsEntry,
    GitDiff, GitStatus, KeybindingProfile, LanguageServerInfo, LspCodeAction,
    LspCodeActionDiagnostic, LspCodeActionTrigger, LspCompletionList, LspDocumentSymbol,
    LspFoldingRange, LspFormattingOptions, LspHover, LspInlayHint, LspLocation, LspRange,
    LspSemanticTokens, LspSignatureHelp, LspTextEdit, LspWorkspaceEdit, LspWorkspaceSymbol,
    LuxEvent, RecentWorkspace, SearchOptions, SearchResponse, SettingValue, SettingsScope,
    TerminalSessionInfo, TextEdit, WorkspaceDiagnostic, WorkspaceEditResult, WorkspaceInfo,
};
use lux_editor::DocumentStore;
use lux_settings::SettingsStore;
use lux_terminal::TerminalService;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::{mpsc::UnboundedReceiver, oneshot};
use tokio::time::{sleep, timeout, Duration};
use uuid::Uuid;

const WATCH_DEBOUNCE_MS: u64 = 120;
const WATCH_MAX_BATCHED_PATHS: usize = 512;
const AI_CHAT_TIMEOUT_SECS: u64 = 180;
const AI_READ_TEXT_MAX_BYTES: u64 = 1_000_000;
const AI_TEST_HEALTH_TIMEOUT_SECS: u64 = 180;
const AI_TEST_HEALTH_MAX_OUTPUT_CHARS: usize = 24_000;
const AI_TEST_HEALTH_SCAN_MAX_DEPTH: usize = 4;
const AI_TEST_HEALTH_MAX_RUNNERS: usize = 12;
const AI_SHELL_DEFAULT_TIMEOUT_SECS: u64 = 120;
const AI_SHELL_MAX_TIMEOUT_SECS: u64 = 600;
const WEB_FETCH_DEFAULT_TIMEOUT_SECS: u64 = 20;
const WEB_FETCH_MAX_TIMEOUT_SECS: u64 = 60;
const WEB_FETCH_DEFAULT_MAX_BYTES: u64 = 250_000;
const WEB_FETCH_MAX_BYTES: u64 = 1_000_000;
const WEB_FETCH_USER_AGENT: &str = "LuxIDE-WebFetch/0.1";
const LOCAL_STT_COMMAND_ENV: &str = "LUX_STT_COMMAND";
const LOCAL_STT_MODEL_ENV: &str = "LUX_STT_MODEL";
const WATCH_EXCLUDED_COMPONENTS: &[&str] = &[
    ".git",
    ".next",
    ".turbo",
    ".vite",
    "coverage",
    "dist",
    "node_modules",
    "target",
];

#[derive(Default)]
struct AppState {
    workspace: Mutex<Option<WorkspaceInfo>>,
    workspace_watcher: Mutex<Option<RecommendedWatcher>>,
    documents: Mutex<DocumentStore>,
    diagnostics: Mutex<Vec<WorkspaceDiagnostic>>,
    ai_streams: Mutex<BTreeMap<String, oneshot::Sender<()>>>,
    lsp: tokio::sync::Mutex<Option<lux_lsp::LspManager>>,
    settings: Mutex<Option<SettingsStore>>,
    terminals: Mutex<Option<Arc<TerminalService>>>,
}

type SharedState = Arc<AppState>;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceInputProviderStatus {
    provider: String,
    available: bool,
    detail: String,
    command: Option<String>,
    model_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiChatCompletionRequest {
    base_url: String,
    api_key: Option<String>,
    payload: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiChatCompletionResponse {
    status: u16,
    body: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiChatCompletionStreamRequest {
    base_url: String,
    api_key: Option<String>,
    payload: Value,
    stream_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiChatCompletionStreamResponse {
    stream_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiChatStreamEvent {
    stream_id: String,
    kind: String,
    data: Option<Value>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebFetchResponse {
    url: String,
    final_url: String,
    status: u16,
    content_type: Option<String>,
    title: Option<String>,
    text: String,
    bytes_read: u64,
    truncated: bool,
    elapsed_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FsReadTextResponse {
    path: PathBuf,
    text: String,
    truncated: bool,
    size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiFileOperationStats {
    lines_added: usize,
    lines_removed: usize,
    files_changed: usize,
    files_created: usize,
    files_deleted: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiFileOperationResult {
    operation: String,
    path: PathBuf,
    saved_to_disk: bool,
    changed_paths: Vec<PathBuf>,
    edited_documents: Vec<DocumentSnapshot>,
    stats: AiFileOperationStats,
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiFilePatchOperation {
    #[serde(alias = "kind", alias = "operation")]
    action: String,
    path: PathBuf,
    text: Option<String>,
    old_text: Option<String>,
    new_text: Option<String>,
    expected_replacements: Option<usize>,
    overwrite: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AiPreparedPatchKind {
    Create,
    Rewrite,
    Replace,
    Delete,
}

#[derive(Debug, Clone)]
struct AiPreparedPatchOperation {
    kind: AiPreparedPatchKind,
    path: PathBuf,
    after_text: Option<String>,
    stats: AiFileOperationStats,
}

#[derive(Debug, Clone)]
struct AiPatchRollbackEntry {
    path: PathBuf,
    previous_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiShellResponse {
    workspace_root: PathBuf,
    cwd: PathBuf,
    command: String,
    exit_code: Option<i32>,
    duration_ms: u128,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiSymbolContextResponse {
    workspace_root: PathBuf,
    query: String,
    path: Option<PathBuf>,
    position: Option<AiSymbolPosition>,
    workspace_symbols: Vec<LspWorkspaceSymbol>,
    document_symbols: Vec<LspDocumentSymbol>,
    hover: Option<LspHover>,
    definitions: Vec<LspLocation>,
    references: Vec<LspLocation>,
    signature_help: Option<LspSignatureHelp>,
    diagnostics: Vec<WorkspaceDiagnostic>,
    notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiSymbolPosition {
    line: u32,
    column: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TestHealthResponse {
    workspace_root: PathBuf,
    status: String,
    summary: TestHealthSummary,
    runners: Vec<TestHealthRunnerResult>,
    language: String,
    framework: String,
    command: String,
    exit_code: Option<i32>,
    duration_ms: u128,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TestHealthSummary {
    total: usize,
    passed: usize,
    failed: usize,
    timed_out: usize,
    errored: usize,
    skipped: usize,
    duration_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TestHealthRunnerResult {
    id: String,
    workspace_relative_path: String,
    status: String,
    kind: String,
    language: String,
    framework: String,
    command: String,
    exit_code: Option<i32>,
    duration_ms: u128,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

#[derive(Debug, Clone)]
struct TestHealthPlan {
    kind: &'static str,
    language: &'static str,
    framework: &'static str,
    working_dir: PathBuf,
    command: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VoiceTranscriptionRequest {
    provider: String,
    audio_base64: String,
    mime_type: String,
    language: Option<String>,
    command: Option<String>,
    model_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceTranscriptionResult {
    text: String,
}

#[tauri::command]
async fn workspace_open(
    app: AppHandle,
    state: State<'_, SharedState>,
    path: PathBuf,
) -> Result<WorkspaceInfo, String> {
    let workspace = lux_workspace::open_workspace(path).map_err(String::from)?;
    stop_workspace_watcher(&state)?;
    shutdown_lsp(&state).await;
    close_all_terminals(&state)?;
    *state.documents.lock().map_err(lock_error)? = DocumentStore::default();
    clear_diagnostics(&app, &state)?;
    *state.workspace.lock().map_err(lock_error)? = Some(workspace.clone());
    if let Err(error) = start_workspace_watcher(&app, &state, workspace.root.clone()) {
        tracing::warn!(%error, "workspace file watcher unavailable");
    }
    record_recent_workspace(&state, &workspace)?;
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
    stop_workspace_watcher(&state)?;
    *state.workspace.lock().map_err(lock_error)? = None;
    *state.documents.lock().map_err(lock_error)? = DocumentStore::default();
    clear_diagnostics(&app, &state)?;
    close_all_terminals(&state)?;
    shutdown_lsp(&state).await;
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
        let limit = max_bytes.min(size) as usize;
        let mut file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
        let mut buffer = vec![0; limit];
        use std::io::Read;
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
async fn editor_open_file(
    app: AppHandle,
    state: State<'_, SharedState>,
    path: PathBuf,
) -> Result<DocumentSnapshot, String> {
    let canonical = tokio::task::spawn_blocking(move || path.canonicalize())
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;

    let existing = {
        let documents = state.documents.lock().map_err(lock_error)?;
        documents
            .snapshot_for_path(&canonical)
            .map_err(String::from)?
    };
    if let Some(document) = existing {
        emit_event(
            &app,
            LuxEvent::EditorDocumentChanged {
                document: document.clone(),
            },
        )?;
        forward_document_open(&app, &state, &document).await?;
        return Ok(document);
    }

    let read_path = canonical.clone();
    let text = tokio::task::spawn_blocking(move || std::fs::read_to_string(read_path))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;

    let document = {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        documents
            .open_loaded_file(canonical, text)
            .map_err(String::from)?
    };
    emit_event(
        &app,
        LuxEvent::EditorDocumentChanged {
            document: document.clone(),
        },
    )?;
    forward_document_open(&app, &state, &document).await?;
    Ok(document)
}

#[tauri::command]
async fn editor_new_file(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<DocumentSnapshot, String> {
    let document = {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        documents.new_untitled()
    };
    emit_event(
        &app,
        LuxEvent::EditorDocumentChanged {
            document: document.clone(),
        },
    )?;
    Ok(document)
}

#[tauri::command]
async fn editor_update_text(
    app: AppHandle,
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    text: String,
) -> Result<DocumentSnapshot, String> {
    let document = {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        documents
            .update_text(buffer_id, text)
            .map_err(String::from)?
    };
    emit_event(
        &app,
        LuxEvent::EditorDocumentChanged {
            document: document.clone(),
        },
    )?;
    forward_document_update(&app, &state, &document).await?;
    Ok(document)
}

#[tauri::command]
async fn editor_apply_edits(
    app: AppHandle,
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    edits: Vec<TextEdit>,
) -> Result<DocumentEditResult, String> {
    let document = {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        documents
            .apply_edits(buffer_id, &edits)
            .map_err(String::from)?
    };
    let result = DocumentEditResult::from(&document);
    emit_event(
        &app,
        LuxEvent::EditorDocumentEdited {
            document: result.clone(),
        },
    )?;
    forward_document_edits(&app, &state, &document, &edits).await?;
    Ok(result)
}

#[tauri::command]
async fn editor_apply_workspace_edit(
    app: AppHandle,
    state: State<'_, SharedState>,
    edit: LspWorkspaceEdit,
) -> Result<WorkspaceEditResult, String> {
    apply_workspace_edit(&app, &state, edit).await
}

#[tauri::command]
async fn editor_save_file(
    app: AppHandle,
    state: State<'_, SharedState>,
    buffer_id: BufferId,
) -> Result<DocumentSnapshot, String> {
    let payload = {
        let documents = state.documents.lock().map_err(lock_error)?;
        documents.save_payload(buffer_id).map_err(String::from)?
    };

    let save_path = match payload.path.clone() {
        Some(path) => path,
        None => pick_save_path(&app, &payload.suggested_name)
            .await?
            .ok_or_else(|| "save cancelled".to_string())?,
    };
    let attach_path = payload.is_untitled;
    save_document_to_path(app, state, buffer_id, payload, save_path, attach_path).await
}

#[tauri::command]
async fn editor_save_file_as(
    app: AppHandle,
    state: State<'_, SharedState>,
    buffer_id: BufferId,
) -> Result<DocumentSnapshot, String> {
    let payload = {
        let documents = state.documents.lock().map_err(lock_error)?;
        documents.save_payload(buffer_id).map_err(String::from)?
    };

    let save_path = pick_save_path(&app, &payload.suggested_name)
        .await?
        .ok_or_else(|| "save cancelled".to_string())?;
    save_document_to_path(app, state, buffer_id, payload, save_path, true).await
}

async fn save_document_to_path(
    app: AppHandle,
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    payload: lux_editor::DocumentSavePayload,
    save_path: PathBuf,
    attach_path: bool,
) -> Result<DocumentSnapshot, String> {
    let saved_version = payload.version;
    let save_path = if attach_path {
        let documents = state.documents.lock().map_err(lock_error)?;
        documents
            .validate_attach_path(buffer_id, &save_path)
            .map_err(String::from)?
    } else {
        save_path
    };

    let write_path = save_path.clone();
    tokio::task::spawn_blocking(move || std::fs::write(write_path, payload.text))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;

    let path_attachment = if attach_path {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        Some(
            documents
                .attach_path_with_previous(buffer_id, save_path.clone())
                .map_err(String::from)?,
        )
    } else {
        None
    };

    let (document, saved_current_version) = {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        documents
            .finish_save(buffer_id, saved_version)
            .map_err(String::from)?
    };
    emit_event(
        &app,
        LuxEvent::EditorDocumentChanged {
            document: document.clone(),
        },
    )?;
    if let Some(attachment) = &path_attachment {
        if let Some(previous_path) = &attachment.previous_path {
            forward_document_close(&app, &state, previous_path).await?;
            apply_diagnostics_update(
                &app,
                state.inner(),
                lux_lsp::DiagnosticsUpdate {
                    path: previous_path.clone(),
                    diagnostics: Vec::new(),
                },
            )?;
            emit_event(
                &app,
                LuxEvent::FsChanged {
                    path: previous_path.clone(),
                },
            )?;
        }
        emit_event(
            &app,
            LuxEvent::FsChanged {
                path: save_path.clone(),
            },
        )?;
    }
    if saved_current_version {
        if path_attachment.is_some() {
            forward_document_open(&app, &state, &document).await?;
        }
        forward_document_save(&app, &state, &document).await?;
    }
    Ok(document)
}

async fn apply_workspace_edit(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    edit: LspWorkspaceEdit,
) -> Result<WorkspaceEditResult, String> {
    let mut edited_documents = Vec::new();
    let mut changed_paths = Vec::new();

    for file_edit in edit.files {
        let path = file_edit.path;
        let text_edits = file_edit
            .edits
            .into_iter()
            .map(|edit| TextEdit {
                start_line: edit.range.start_line,
                start_column: edit.range.start_column,
                end_line: edit.range.end_line,
                end_column: edit.range.end_column,
                text: edit.text,
            })
            .collect::<Vec<_>>();
        if text_edits.is_empty() {
            continue;
        }

        let edited_open_document = {
            let mut documents = state.documents.lock().map_err(lock_error)?;
            documents
                .apply_edits_for_path(&path, &text_edits)
                .map_err(String::from)?
        };

        if let Some(document) = edited_open_document {
            forward_document_update(app, state, &document).await?;
            edited_documents.push(document);
            changed_paths.push(path);
            continue;
        }

        let write_path = path.clone();
        let text_edits_for_file = text_edits.clone();
        tokio::task::spawn_blocking(move || -> lux_core::AppResult<()> {
            let mut text = std::fs::read_to_string(&write_path)?;
            lux_editor::apply_text_edits(&mut text, &text_edits_for_file)?;
            std::fs::write(&write_path, text)?;
            Ok(())
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)?;
        emit_event(app, LuxEvent::FsChanged { path: path.clone() })?;
        changed_paths.push(path);
    }

    if !edited_documents.is_empty() {
        emit_event(
            app,
            LuxEvent::EditorDocumentsChanged {
                documents: edited_documents.clone(),
            },
        )?;
    }

    Ok(WorkspaceEditResult {
        edited_documents,
        changed_paths,
    })
}

#[tauri::command]
async fn search_query(
    state: State<'_, SharedState>,
    query: String,
    options: SearchOptions,
) -> Result<SearchResponse, String> {
    let root = state
        .workspace
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .map(|workspace| workspace.root.clone())
        .ok_or_else(|| "no workspace is open".to_string())?;

    tokio::task::spawn_blocking(move || lux_search::query(root, query, options))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
async fn ai_chat_completion(
    request: AiChatCompletionRequest,
) -> Result<AiChatCompletionResponse, String> {
    let endpoint = ai_chat_completion_endpoint(&request.base_url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(AI_CHAT_TIMEOUT_SECS))
        .build()
        .map_err(|error| error.to_string())?;

    let mut builder = client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&request.payload);

    if let Some(api_key) = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        builder = builder.bearer_auth(api_key);
    }

    let response = timeout(
        Duration::from_secs(AI_CHAT_TIMEOUT_SECS + 5),
        builder.send(),
    )
    .await
    .map_err(|_| "AI request timed out".to_string())?
    .map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let body = response
        .json::<Value>()
        .await
        .map_err(|error| error.to_string())?;

    if status >= 400 {
        return Err(ai_response_error(status, &body));
    }

    Ok(AiChatCompletionResponse { status, body })
}

#[tauri::command]
async fn ai_chat_completion_stream(
    app: AppHandle,
    state: State<'_, SharedState>,
    request: AiChatCompletionStreamRequest,
) -> Result<AiChatCompletionStreamResponse, String> {
    let stream_id = request
        .stream_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    state
        .ai_streams
        .lock()
        .map_err(lock_error)?
        .insert(stream_id.clone(), cancel_tx);

    let state = state.inner().clone();
    let app_handle = app.clone();
    let stream_id_for_task = stream_id.clone();
    tauri::async_runtime::spawn(async move {
        run_ai_chat_completion_stream(app_handle, state, stream_id_for_task, request, cancel_rx)
            .await;
    });

    Ok(AiChatCompletionStreamResponse { stream_id })
}

#[tauri::command]
async fn ai_chat_completion_stream_cancel(
    state: State<'_, SharedState>,
    stream_id: String,
) -> Result<(), String> {
    if let Some(cancel) = state
        .ai_streams
        .lock()
        .map_err(lock_error)?
        .remove(&stream_id)
    {
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
) -> Result<WebFetchResponse, String> {
    let started = std::time::Instant::now();
    let url = validate_web_fetch_url(&url)?;
    if !allow_private_hosts.unwrap_or(false) {
        reject_private_web_fetch_host(&url).await?;
    }
    let max_bytes = max_bytes
        .unwrap_or(WEB_FETCH_DEFAULT_MAX_BYTES)
        .clamp(1_024, WEB_FETCH_MAX_BYTES);
    let timeout_secs = timeout_secs
        .unwrap_or(WEB_FETCH_DEFAULT_TIMEOUT_SECS)
        .clamp(1, WEB_FETCH_MAX_TIMEOUT_SECS);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(WEB_FETCH_USER_AGENT)
        .build()
        .map_err(|error| error.to_string())?;
    let response = timeout(
        Duration::from_secs(timeout_secs + 5),
        client.get(url.clone()).send(),
    )
    .await
    .map_err(|_| "WebFetch request timed out".to_string())?
    .map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    if !allow_private_hosts.unwrap_or(false) {
        reject_private_web_fetch_host(response.url()).await?;
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let bytes = response.bytes().await.map_err(|error| error.to_string())?;
    let truncated = bytes.len() as u64 > max_bytes;
    let visible = &bytes[..usize::min(bytes.len(), max_bytes as usize)];
    let raw_text = String::from_utf8_lossy(visible).to_string();
    let text = normalize_web_fetch_text(&raw_text, content_type.as_deref());
    let title = extract_html_title(&raw_text);

    Ok(WebFetchResponse {
        url: url.to_string(),
        final_url,
        status,
        content_type,
        title,
        text,
        bytes_read: visible.len() as u64,
        truncated,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

#[tauri::command]
async fn test_health(state: State<'_, SharedState>) -> Result<TestHealthResponse, String> {
    let root = state
        .workspace
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .map(|workspace| workspace.root.clone())
        .ok_or_else(|| "no workspace is open".to_string())?;

    let plans = detect_test_health_plans(&root)?;
    run_test_health_plans(root, plans).await
}

#[tauri::command]
async fn ai_file_write(
    app: AppHandle,
    state: State<'_, SharedState>,
    path: PathBuf,
    text: String,
    overwrite: Option<bool>,
    save_to_disk: Option<bool>,
) -> Result<AiFileOperationResult, String> {
    let path = resolve_workspace_path_for_write(&state, &path)?;
    let exists = path.exists();
    if exists && !overwrite.unwrap_or(false) {
        return Err(format!("file already exists: {}", path.display()));
    }

    let previous_text = if exists {
        Some(
            tokio::fs::read_to_string(&path)
                .await
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    let stats = diff_stats(
        previous_text.as_deref().unwrap_or(""),
        &text,
        exists.then_some(false).unwrap_or(true),
    );
    let save_to_disk = save_to_disk.unwrap_or(true);
    if save_to_disk {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| error.to_string())?;
        }
        tokio::fs::write(&path, &text)
            .await
            .map_err(|error| error.to_string())?;
        emit_event(&app, LuxEvent::FsChanged { path: path.clone() })?;
    }

    let edited_document = if save_to_disk {
        let maybe_existing = {
            let mut documents = state.documents.lock().map_err(lock_error)?;
            documents
                .replace_text_for_path(&path, text.clone(), false)
                .map_err(String::from)?
        };
        let document = if let Some(document) = maybe_existing {
            document
        } else {
            let mut documents = state.documents.lock().map_err(lock_error)?;
            documents
                .open_loaded_file(path.clone(), text)
                .map_err(String::from)?
        };
        emit_event(
            &app,
            LuxEvent::EditorDocumentChanged {
                document: document.clone(),
            },
        )?;
        if exists {
            forward_document_update(&app, &state, &document).await?;
        } else {
            forward_document_open(&app, &state, &document).await?;
        }
        Some(document)
    } else {
        let existing = {
            let mut documents = state.documents.lock().map_err(lock_error)?;
            let existing = documents.snapshot_for_path(&path).map_err(String::from)?;
            match existing {
                Some(document) => Some(
                    documents
                        .update_text(document.id, text)
                        .map_err(String::from)?,
                ),
                None => None,
            }
        };
        if let Some(document) = &existing {
            emit_event(
                &app,
                LuxEvent::EditorDocumentChanged {
                    document: document.clone(),
                },
            )?;
            forward_document_update(&app, &state, document).await?;
        }
        existing
    };

    Ok(AiFileOperationResult {
        operation: "write".to_string(),
        path: path.clone(),
        saved_to_disk: save_to_disk,
        changed_paths: vec![path.clone()],
        edited_documents: edited_document.into_iter().collect(),
        stats,
        message: if exists {
            "file overwritten"
        } else {
            "file created"
        }
        .to_string(),
    })
}

#[tauri::command]
async fn ai_file_str_replace(
    app: AppHandle,
    state: State<'_, SharedState>,
    path: PathBuf,
    old_text: String,
    new_text: String,
    expected_replacements: Option<usize>,
    save_to_disk: Option<bool>,
) -> Result<AiFileOperationResult, String> {
    if old_text.is_empty() {
        return Err("oldText must not be empty".to_string());
    }
    let path = resolve_workspace_path(&state, &path)?;
    let before = current_text_for_path(&state, &path).await?;
    let replacement_count = before.matches(&old_text).count();
    let expected = expected_replacements.unwrap_or(1);
    if replacement_count != expected {
        return Err(format!(
            "replacement count mismatch for {}: expected {expected}, found {replacement_count}",
            path.display()
        ));
    }
    let after = before.replacen(&old_text, &new_text, expected);
    let stats = diff_stats(&before, &after, false);
    let save_to_disk = save_to_disk.unwrap_or(true);
    if save_to_disk {
        tokio::fs::write(&path, &after)
            .await
            .map_err(|error| error.to_string())?;
        emit_event(&app, LuxEvent::FsChanged { path: path.clone() })?;
    }

    let edited_document =
        update_open_document_after_text_change(&app, &state, &path, after, !save_to_disk).await?;
    Ok(AiFileOperationResult {
        operation: "strReplace".to_string(),
        path: path.clone(),
        saved_to_disk: save_to_disk,
        changed_paths: vec![path.clone()],
        edited_documents: edited_document.into_iter().collect(),
        stats,
        message: format!("replaced {replacement_count} occurrence(s)"),
    })
}

#[tauri::command]
async fn ai_file_patch(
    app: AppHandle,
    state: State<'_, SharedState>,
    operations: Vec<AiFilePatchOperation>,
    save_to_disk: Option<bool>,
    dry_run: Option<bool>,
) -> Result<AiFileOperationResult, String> {
    if operations.is_empty() {
        return Err("patch operations must not be empty".to_string());
    }
    if operations.len() > 80 {
        return Err("patch operation limit exceeded: maximum 80 operations".to_string());
    }

    let prepared = prepare_ai_patch_operations(&state, operations).await?;
    let dry_run = dry_run.unwrap_or(false);
    let save_to_disk = save_to_disk.unwrap_or(true);
    let stats = combine_patch_stats(&prepared);
    let changed_paths = unique_patch_paths(&prepared);

    if dry_run {
        return Ok(AiFileOperationResult {
            operation: "patch".to_string(),
            path: changed_paths.first().cloned().unwrap_or_default(),
            saved_to_disk: false,
            changed_paths,
            edited_documents: Vec::new(),
            stats,
            message: format!("patch dry-run passed for {} operation(s)", prepared.len()),
        });
    }

    let mut rollback = Vec::new();
    let write_result = apply_ai_patch_to_disk(&prepared, save_to_disk, &mut rollback).await;
    if let Err(error) = write_result {
        rollback_ai_patch(rollback).await;
        return Err(error);
    }

    let mut edited_documents = Vec::new();
    let mut document_events: Vec<(DocumentSnapshot, bool)> = Vec::new();
    let mut closed_documents = Vec::new();
    let document_result: Result<(), String> = (|| {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        for operation in &prepared {
            match operation.kind {
                AiPreparedPatchKind::Delete => {
                    if let Some(document) = documents
                        .close_path(&operation.path)
                        .map_err(String::from)?
                    {
                        closed_documents.push(document);
                    }
                }
                AiPreparedPatchKind::Create
                | AiPreparedPatchKind::Rewrite
                | AiPreparedPatchKind::Replace => {
                    let after_text = operation.after_text.clone().unwrap_or_default();
                    let (document, is_new_document) = match documents
                        .replace_text_for_path(&operation.path, after_text.clone(), !save_to_disk)
                        .map_err(String::from)?
                    {
                        Some(document) => (document, false),
                        None if save_to_disk => (
                            documents
                                .open_loaded_file(operation.path.clone(), after_text)
                                .map_err(String::from)?,
                            true,
                        ),
                        None => continue,
                    };
                    document_events.push((document.clone(), is_new_document));
                    edited_documents.push(document);
                }
            }
        }
        Ok(())
    })();

    if let Err(error) = document_result {
        rollback_ai_patch(rollback).await;
        return Err(error);
    }

    for document in &closed_documents {
        emit_event(
            &app,
            LuxEvent::EditorDocumentClosed {
                document: document.clone(),
            },
        )?;
        if let Some(path) = &document.path {
            forward_document_close(&app, &state, path).await?;
        }
    }
    for (document, is_new_document) in &document_events {
        emit_event(
            &app,
            LuxEvent::EditorDocumentChanged {
                document: document.clone(),
            },
        )?;
        if *is_new_document {
            forward_document_open(&app, &state, document).await?;
        } else {
            forward_document_update(&app, &state, document).await?;
        }
    }
    for path in &changed_paths {
        if save_to_disk {
            emit_event(&app, LuxEvent::FsChanged { path: path.clone() })?;
        }
        if prepared.iter().any(|operation| {
            operation.path == *path && operation.kind == AiPreparedPatchKind::Delete
        }) {
            apply_diagnostics_update(
                &app,
                state.inner(),
                lux_lsp::DiagnosticsUpdate {
                    path: path.clone(),
                    diagnostics: Vec::new(),
                },
            )?;
        }
    }

    Ok(AiFileOperationResult {
        operation: "patch".to_string(),
        path: changed_paths.first().cloned().unwrap_or_default(),
        saved_to_disk: save_to_disk,
        changed_paths,
        edited_documents,
        stats,
        message: format!(
            "patch applied: {} operation(s), {} path(s)",
            prepared.len(),
            unique_patch_paths(&prepared).len()
        ),
    })
}

#[tauri::command]
async fn ai_file_delete(
    app: AppHandle,
    state: State<'_, SharedState>,
    path: PathBuf,
) -> Result<AiFileOperationResult, String> {
    let path = resolve_workspace_path(&state, &path)?;
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|error| error.to_string())?;
    let previous_text = if metadata.is_file() {
        tokio::fs::read_to_string(&path).await.unwrap_or_default()
    } else {
        String::new()
    };
    if metadata.is_dir() {
        tokio::fs::remove_dir_all(&path)
            .await
            .map_err(|error| error.to_string())?;
    } else {
        tokio::fs::remove_file(&path)
            .await
            .map_err(|error| error.to_string())?;
    }
    let stats = AiFileOperationStats {
        lines_added: 0,
        lines_removed: previous_text.lines().count(),
        files_changed: 0,
        files_created: 0,
        files_deleted: 1,
    };
    let closed_document = {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        documents.close_path(&path).map_err(String::from)?
    };
    if let Some(document) = &closed_document {
        emit_event(
            &app,
            LuxEvent::EditorDocumentClosed {
                document: document.clone(),
            },
        )?;
    }
    forward_document_close(&app, &state, &path).await?;
    apply_diagnostics_update(
        &app,
        state.inner(),
        lux_lsp::DiagnosticsUpdate {
            path: path.clone(),
            diagnostics: Vec::new(),
        },
    )?;
    emit_event(&app, LuxEvent::FsChanged { path: path.clone() })?;
    Ok(AiFileOperationResult {
        operation: "delete".to_string(),
        path: path.clone(),
        saved_to_disk: true,
        changed_paths: vec![path.clone()],
        edited_documents: Vec::new(),
        stats,
        message: "file deleted".to_string(),
    })
}

#[tauri::command]
async fn ai_shell(
    state: State<'_, SharedState>,
    command: String,
    cwd: Option<PathBuf>,
    timeout_secs: Option<u64>,
) -> Result<AiShellResponse, String> {
    let root = workspace_root(&state)?;
    let cwd = match cwd {
        Some(path) => resolve_workspace_path_from_root(&root, &path, true)?,
        None => root.clone(),
    };
    if !cwd.is_dir() {
        return Err(format!("shell cwd is not a directory: {}", cwd.display()));
    }
    let command = command.trim().to_string();
    if command.is_empty() {
        return Err("shell command must not be empty".to_string());
    }
    let timeout_secs = timeout_secs
        .unwrap_or(AI_SHELL_DEFAULT_TIMEOUT_SECS)
        .clamp(1, AI_SHELL_MAX_TIMEOUT_SECS);

    let started = std::time::Instant::now();
    let mut process = shell_command(&command);
    process.current_dir(&cwd);
    let output_result = timeout(Duration::from_secs(timeout_secs), process.output()).await;
    let duration_ms = started.elapsed().as_millis();

    match output_result {
        Ok(Ok(output)) => Ok(AiShellResponse {
            workspace_root: root,
            cwd,
            command,
            exit_code: output.status.code(),
            duration_ms,
            stdout: truncate_output(&String::from_utf8_lossy(&output.stdout)),
            stderr: truncate_output(&String::from_utf8_lossy(&output.stderr)),
            timed_out: false,
        }),
        Ok(Err(error)) => Err(format!("Failed to start shell command: {error}")),
        Err(_) => Ok(AiShellResponse {
            workspace_root: root,
            cwd,
            command,
            exit_code: None,
            duration_ms,
            stdout: String::new(),
            stderr: format!("Shell command timed out after {timeout_secs} seconds"),
            timed_out: true,
        }),
    }
}

#[tauri::command]
async fn ai_symbol_context(
    state: State<'_, SharedState>,
    query: Option<String>,
    path: Option<PathBuf>,
    line: Option<u32>,
    column: Option<u32>,
    max_results: Option<usize>,
) -> Result<AiSymbolContextResponse, String> {
    let root = workspace_root(&state)?;
    let query = query.unwrap_or_default().trim().to_string();
    let max_results = max_results.unwrap_or(80).clamp(1, 300);
    let position = line
        .zip(column)
        .map(|(line, column)| AiSymbolPosition { line, column });
    let resolved_path = path
        .as_deref()
        .map(|path| resolve_workspace_path(&state, path))
        .transpose()?;
    let diagnostics = state.diagnostics.lock().map_err(lock_error)?.clone();
    let mut notes = Vec::new();
    let mut workspace_symbols = Vec::new();
    let mut document_symbols = Vec::new();
    let mut hover = None;
    let mut definitions = Vec::new();
    let mut references = Vec::new();
    let mut signature_help = None;

    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        notes.push("language service is not initialized".to_string());
        return Ok(AiSymbolContextResponse {
            workspace_root: root,
            query,
            path: resolved_path,
            position,
            workspace_symbols,
            document_symbols,
            hover,
            definitions,
            references,
            signature_help,
            diagnostics,
            notes,
        });
    };

    if !query.is_empty() {
        workspace_symbols = manager
            .workspace_symbols(query.clone())
            .await
            .map_err(String::from)?;
        workspace_symbols.truncate(max_results);
    }

    if let Some(path) = &resolved_path {
        let document = symbol_context_document_for_path(&state, path).await?;
        manager
            .open_document(&document)
            .await
            .map_err(String::from)?;
        document_symbols = manager
            .document_symbols(&document)
            .await
            .map_err(String::from)?;
        truncate_document_symbols(&mut document_symbols, max_results);

        if let Some(position) = &position {
            hover = manager
                .hover(&document, position.line, position.column)
                .await
                .map_err(String::from)?;
            definitions = manager
                .definition(&document, position.line, position.column)
                .await
                .map_err(String::from)?;
            references = manager
                .references(&document, position.line, position.column)
                .await
                .map_err(String::from)?;
            signature_help = manager
                .signature_help(&document, position.line, position.column)
                .await
                .map_err(String::from)?;
            definitions.truncate(max_results);
            references.truncate(max_results);
        } else if !query.is_empty() {
            document_symbols = filter_document_symbols(&document_symbols, &query, max_results);
        }

        if document_symbols.is_empty() && query.is_empty() {
            notes.push("no document symbols returned; the language server may still be indexing or may not support document symbols for this file".to_string());
        }
    }

    if workspace_symbols.is_empty()
        && document_symbols.is_empty()
        && resolved_path.is_none()
        && query.is_empty()
    {
        notes.push("provide a query or path for semantic symbol context".to_string());
    }

    Ok(AiSymbolContextResponse {
        workspace_root: root,
        query,
        path: resolved_path,
        position,
        workspace_symbols,
        document_symbols,
        hover,
        definitions,
        references,
        signature_help,
        diagnostics,
        notes,
    })
}

#[tauri::command]
fn voice_input_status(
    provider: String,
    command: Option<String>,
    model_path: Option<PathBuf>,
) -> Result<VoiceInputProviderStatus, String> {
    Ok(match provider.as_str() {
        "native-webview" => VoiceInputProviderStatus {
            provider,
            available: true,
            detail: "Native WebView speech recognition is checked in the frontend runtime"
                .to_string(),
            command: None,
            model_path: None,
        },
        "local" => local_voice_input_status(command, model_path),
        unknown => VoiceInputProviderStatus {
            provider: unknown.to_string(),
            available: false,
            detail: "Unknown voice input provider".to_string(),
            command: None,
            model_path: None,
        },
    })
}

#[tauri::command]
async fn voice_transcribe_local(
    request: VoiceTranscriptionRequest,
) -> Result<VoiceTranscriptionResult, String> {
    if request.provider != "local" {
        return Err("only local voice transcription is supported by this command".to_string());
    }

    let status = local_voice_input_status(request.command.clone(), request.model_path.clone());
    if !status.available {
        return Err(status.detail);
    }

    let command = status
        .command
        .ok_or_else(|| "Local STT command is not configured".to_string())?;
    let audio = base64::engine::general_purpose::STANDARD
        .decode(request.audio_base64.as_bytes())
        .map_err(|error| format!("Invalid recorded audio: {error}"))?;
    if audio.is_empty() {
        return Err("Recorded audio is empty".to_string());
    }

    run_local_stt_command(
        &command,
        &audio,
        &request.mime_type,
        request.language.as_deref(),
        status.model_path.as_deref(),
    )
    .await
    .map(|text| VoiceTranscriptionResult { text })
}

#[tauri::command]
fn terminal_create(
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
            .map(|workspace| workspace.root.clone())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
    };
    let service = state
        .terminals
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .cloned()
        .ok_or_else(|| "terminal service is not initialized".to_string())?;
    service
        .create(shell, cwd, cols.unwrap_or(120), rows.unwrap_or(30))
        .map_err(String::from)
}

#[tauri::command]
fn terminal_write(
    state: State<'_, SharedState>,
    session_id: Uuid,
    data: String,
) -> Result<(), String> {
    let service = state
        .terminals
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .cloned()
        .ok_or_else(|| "terminal service is not initialized".to_string())?;
    service.write(session_id, &data).map_err(String::from)
}

#[tauri::command]
fn terminal_resize(
    state: State<'_, SharedState>,
    session_id: Uuid,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let service = state
        .terminals
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .cloned()
        .ok_or_else(|| "terminal service is not initialized".to_string())?;
    service.resize(session_id, cols, rows).map_err(String::from)
}

#[tauri::command]
fn terminal_close(state: State<'_, SharedState>, session_id: Uuid) -> Result<(), String> {
    let service = state
        .terminals
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .cloned()
        .ok_or_else(|| "terminal service is not initialized".to_string())?;
    service.close(session_id).map_err(String::from)
}

#[tauri::command]
fn terminal_close_all(state: State<'_, SharedState>) -> Result<(), String> {
    close_all_terminals(&state)
}

#[tauri::command]
async fn git_status(state: State<'_, SharedState>) -> Result<GitStatus, String> {
    let root = state
        .workspace
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .map(|workspace| workspace.root.clone())
        .ok_or_else(|| "no workspace is open".to_string())?;

    tokio::task::spawn_blocking(move || lux_git::status(root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
async fn git_diff(state: State<'_, SharedState>) -> Result<GitDiff, String> {
    let root = state
        .workspace
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .map(|workspace| workspace.root.clone())
        .ok_or_else(|| "no workspace is open".to_string())?;

    tokio::task::spawn_blocking(move || lux_git::diff(root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
async fn extensions_list(app: AppHandle) -> Result<Vec<ExtensionInfo>, String> {
    let extensions_root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("extensions");

    tokio::task::spawn_blocking(move || lux_extensions::discover_extensions(extensions_root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
async fn debug_workspace_info(state: State<'_, SharedState>) -> Result<DebugWorkspaceInfo, String> {
    let root = state
        .workspace
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .map(|workspace| workspace.root.clone())
        .ok_or_else(|| "no workspace is open".to_string())?;

    tokio::task::spawn_blocking(move || lux_dap::workspace_debug_info(root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
async fn lsp_servers(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<Vec<LanguageServerInfo>, String> {
    let root = state
        .workspace
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .map(|workspace| workspace.root.clone())
        .ok_or_else(|| "no workspace is open".to_string())?;

    let servers = tokio::task::spawn_blocking(move || lux_lsp::workspace_language_servers(root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)?;
    let mut diagnostics = lux_lsp::language_server_diagnostics(&servers);
    diagnostics.extend(start_lsp_servers(&state, &servers).await?);
    replace_diagnostics(&app, &state, diagnostics)?;

    let open_documents = state.documents.lock().map_err(lock_error)?.snapshots();
    for document in open_documents {
        forward_document_open(&app, &state, &document).await?;
    }

    Ok(servers)
}

#[tauri::command]
fn diagnostics_snapshot(state: State<'_, SharedState>) -> Result<Vec<WorkspaceDiagnostic>, String> {
    Ok(state.diagnostics.lock().map_err(lock_error)?.clone())
}

#[tauri::command]
async fn lsp_hover(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    line: u32,
    column: u32,
) -> Result<Option<LspHover>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(None);
    };
    manager
        .hover(&document, line, column)
        .await
        .map_err(String::from)
}

#[tauri::command]
async fn lsp_definition(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    line: u32,
    column: u32,
) -> Result<Vec<LspLocation>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .definition(&document, line, column)
        .await
        .map_err(String::from)
}

#[tauri::command]
async fn lsp_references(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    line: u32,
    column: u32,
) -> Result<Vec<LspLocation>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .references(&document, line, column)
        .await
        .map_err(String::from)
}

#[tauri::command]
async fn lsp_document_symbols(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
) -> Result<Vec<LspDocumentSymbol>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .document_symbols(&document)
        .await
        .map_err(String::from)
}

#[tauri::command]
async fn lsp_workspace_symbols(
    state: State<'_, SharedState>,
    query: String,
) -> Result<Vec<LspWorkspaceSymbol>, String> {
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager.workspace_symbols(query).await.map_err(String::from)
}

#[tauri::command]
async fn lsp_folding_ranges(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
) -> Result<Vec<LspFoldingRange>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .folding_ranges(&document)
        .await
        .map_err(String::from)
}

#[tauri::command]
async fn lsp_inlay_hints(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    range: LspRange,
) -> Result<Vec<LspInlayHint>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .inlay_hints(&document, range)
        .await
        .map_err(String::from)
}

#[tauri::command]
async fn lsp_semantic_tokens(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
) -> Result<Option<LspSemanticTokens>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(None);
    };
    manager
        .semantic_tokens(&document)
        .await
        .map_err(String::from)
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
    apply_workspace_edit(&app, &state, edit).await
}

#[tauri::command]
async fn lsp_completion(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    line: u32,
    column: u32,
) -> Result<LspCompletionList, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(LspCompletionList {
            is_incomplete: false,
            items: Vec::new(),
        });
    };
    manager
        .completion(&document, line, column)
        .await
        .map_err(String::from)
}

#[tauri::command]
async fn lsp_code_actions(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    range: LspRange,
    diagnostics: Vec<LspCodeActionDiagnostic>,
    only: Option<Vec<String>>,
    trigger: LspCodeActionTrigger,
) -> Result<Vec<LspCodeAction>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .code_actions(&document, range, diagnostics, only, trigger)
        .await
        .map_err(String::from)
}

#[tauri::command]
async fn lsp_format_document(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    options: LspFormattingOptions,
) -> Result<Vec<LspTextEdit>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .format_document(&document, options)
        .await
        .map_err(String::from)
}

#[tauri::command]
async fn lsp_format_range(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    range: LspRange,
    options: LspFormattingOptions,
) -> Result<Vec<LspTextEdit>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .format_range(&document, range, options)
        .await
        .map_err(String::from)
}

#[tauri::command]
async fn lsp_signature_help(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    line: u32,
    column: u32,
) -> Result<Option<LspSignatureHelp>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(None);
    };
    manager
        .signature_help(&document, line, column)
        .await
        .map_err(String::from)
}

#[tauri::command]
fn recent_workspaces(state: State<'_, SharedState>) -> Result<Vec<RecentWorkspace>, String> {
    with_settings(&state, |settings| settings.recent_workspaces()).map_err(String::from)
}

#[tauri::command]
fn recent_workspace_forget(
    state: State<'_, SharedState>,
    root: PathBuf,
) -> Result<Vec<RecentWorkspace>, String> {
    with_settings(&state, |settings| settings.forget_recent_workspace(root)).map_err(String::from)
}

#[tauri::command]
fn settings_get(
    state: State<'_, SharedState>,
    scope: SettingsScope,
    key: String,
) -> Result<Option<SettingValue>, String> {
    let settings = state.settings.lock().map_err(lock_error)?;
    Ok(settings.as_ref().and_then(|store| store.get(scope, &key)))
}

#[tauri::command]
fn settings_set(
    app: AppHandle,
    state: State<'_, SharedState>,
    scope: SettingsScope,
    key: String,
    value: Value,
) -> Result<SettingValue, String> {
    let mut settings = state.settings.lock().map_err(lock_error)?;
    let store = settings
        .as_mut()
        .ok_or_else(|| "settings store is not initialized".to_string())?;
    let setting = store.set(scope, key.clone(), value).map_err(String::from)?;
    emit_event(&app, LuxEvent::SettingsChanged { key })?;
    Ok(setting)
}

#[tauri::command]
fn keybindings_get(state: State<'_, SharedState>) -> Result<KeybindingProfile, String> {
    with_settings(&state, |settings| Ok(settings.keybinding_profile())).map_err(String::from)
}

#[tauri::command]
fn keybindings_set(
    app: AppHandle,
    state: State<'_, SharedState>,
    profile: KeybindingProfile,
) -> Result<KeybindingProfile, String> {
    let mut settings = state.settings.lock().map_err(lock_error)?;
    let store = settings
        .as_mut()
        .ok_or_else(|| "settings store is not initialized".to_string())?;
    let profile = store
        .set_keybinding_profile(profile)
        .map_err(String::from)?;
    emit_event(
        &app,
        LuxEvent::SettingsChanged {
            key: "workbench.keybindings".to_string(),
        },
    )?;
    Ok(profile)
}

pub fn run() {
    let state = Arc::new(AppState::default());

    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
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
                .map_err(|error| Box::<dyn std::error::Error>::from(error))?
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
            *state.lsp.blocking_lock() = Some(lux_lsp::LspManager::new(diagnostics_tx));
            let diagnostics_state = state.inner().clone();
            tauri::async_runtime::spawn(async move {
                while let Some(update) = diagnostics_rx.recv().await {
                    let _ = apply_diagnostics_update(&lsp_handle, &diagnostics_state, update);
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
            editor_open_file,
            editor_new_file,
            editor_update_text,
            editor_apply_edits,
            editor_apply_workspace_edit,
            editor_save_file,
            editor_save_file_as,
            search_query,
            ai_chat_completion,
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
            debug_workspace_info,
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

fn emit_event(app: &AppHandle, event: LuxEvent) -> Result<(), String> {
    app.emit("lux://event", event)
        .map_err(|error| error.to_string())
}

fn ai_chat_completion_endpoint(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("AI provider base URL is empty".to_string());
    }
    let url = reqwest::Url::parse(trimmed)
        .map_err(|error| format!("Invalid AI provider URL: {error}"))?;
    match url.scheme() {
        "http" | "https" => {}
        scheme => return Err(format!("Unsupported AI provider URL scheme: {scheme}")),
    }
    let text = url.as_str().trim_end_matches('/');
    if text.ends_with("/chat/completions") {
        Ok(text.to_string())
    } else {
        Ok(format!("{text}/chat/completions"))
    }
}

fn ai_response_error(status: u16, body: &Value) -> String {
    let message = body
        .get("error")
        .and_then(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
        })
        .or_else(|| body.get("message").and_then(Value::as_str))
        .unwrap_or("AI provider returned an error");
    format!("AI provider error {status}: {message}")
}

fn validate_web_fetch_url(url: &str) -> Result<reqwest::Url, String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("WebFetch URL is empty".to_string());
    }
    let parsed =
        reqwest::Url::parse(trimmed).map_err(|error| format!("Invalid WebFetch URL: {error}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => return Err(format!("Unsupported WebFetch URL scheme: {scheme}")),
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "WebFetch URL must include a host".to_string())?;
    if host.trim().is_empty() {
        return Err("WebFetch URL host is empty".to_string());
    }
    Ok(parsed)
}

async fn reject_private_web_fetch_host(url: &reqwest::Url) -> Result<(), String> {
    let host = url
        .host_str()
        .ok_or_else(|| "WebFetch URL must include a host".to_string())?
        .to_string();
    if is_localhost_name(&host) {
        return Err("WebFetch blocks localhost/private hosts by default".to_string());
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "WebFetch URL has no usable port".to_string())?;
    let addresses = tokio::task::spawn_blocking(move || {
        (host.as_str(), port)
            .to_socket_addrs()
            .map(|iter| iter.map(|socket| socket.ip()).collect::<Vec<_>>())
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    if addresses.is_empty() {
        return Err("WebFetch host did not resolve to any address".to_string());
    }
    if addresses.iter().any(|ip| is_private_web_fetch_ip(*ip)) {
        return Err("WebFetch blocks localhost/private network addresses by default".to_string());
    }
    Ok(())
}

fn is_localhost_name(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost")
}

fn is_private_web_fetch_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.segments()[0] & 0xffc0 == 0xfe80
        }
    }
}

fn normalize_web_fetch_text(text: &str, content_type: Option<&str>) -> String {
    let looks_html = content_type
        .map(|value| value.to_ascii_lowercase().contains("html"))
        .unwrap_or_else(|| {
            text.contains("<html") || text.contains("<body") || text.contains("<!DOCTYPE html")
        });
    let normalized = if looks_html {
        html_to_text(text)
    } else {
        text.to_string()
    };
    compact_web_fetch_whitespace(&normalized)
}

fn html_to_text(html: &str) -> String {
    let without_scripts = strip_html_block(html, "script");
    let without_styles = strip_html_block(&without_scripts, "style");
    let with_breaks = without_styles
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</p>", "\n")
        .replace("</div>", "\n")
        .replace("</li>", "\n")
        .replace("</h1>", "\n")
        .replace("</h2>", "\n")
        .replace("</h3>", "\n");
    let mut output = String::with_capacity(with_breaks.len());
    let mut in_tag = false;
    for character in with_breaks.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(character),
            _ => {}
        }
    }
    decode_basic_html_entities(&output)
}

fn strip_html_block(input: &str, tag: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let open_pattern = format!("<{tag}");
    let close_pattern = format!("</{tag}>");
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(relative_start) = lower[cursor..].find(&open_pattern) {
        let start = cursor + relative_start;
        output.push_str(&input[cursor..start]);
        let Some(relative_end) = lower[start..].find(&close_pattern) else {
            return output;
        };
        cursor = start + relative_end + close_pattern.len();
    }
    output.push_str(&input[cursor..]);
    output
}

fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let after_open = lower[start..].find('>')? + start + 1;
    let end = lower[after_open..].find("</title>")? + after_open;
    let title = decode_basic_html_entities(&html[after_open..end]);
    let compact = compact_web_fetch_whitespace(&title);
    (!compact.is_empty()).then_some(compact)
}

fn compact_web_fetch_whitespace(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_basic_html_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

async fn run_ai_chat_completion_stream(
    app: AppHandle,
    state: SharedState,
    stream_id: String,
    request: AiChatCompletionStreamRequest,
    cancel_rx: oneshot::Receiver<()>,
) {
    let result = stream_ai_chat_completion(&app, &stream_id, request, cancel_rx).await;
    let _ = state
        .ai_streams
        .lock()
        .map(|mut streams| streams.remove(&stream_id));

    match result {
        Ok(AiStreamCompletion::Done) => {}
        Ok(AiStreamCompletion::Cancelled) => {
            let _ = emit_ai_stream_event(
                &app,
                AiChatStreamEvent {
                    stream_id,
                    kind: "cancelled".to_string(),
                    data: None,
                    error: None,
                },
            );
        }
        Err(error) => {
            let _ = emit_ai_stream_event(
                &app,
                AiChatStreamEvent {
                    stream_id,
                    kind: "error".to_string(),
                    data: None,
                    error: Some(error),
                },
            );
        }
    }
}

enum AiStreamCompletion {
    Done,
    Cancelled,
}

async fn stream_ai_chat_completion(
    app: &AppHandle,
    stream_id: &str,
    request: AiChatCompletionStreamRequest,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<AiStreamCompletion, String> {
    let endpoint = ai_chat_completion_endpoint(&request.base_url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(AI_CHAT_TIMEOUT_SECS))
        .build()
        .map_err(|error| error.to_string())?;
    let payload = ai_stream_payload(request.payload);

    let mut builder = client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .json(&payload);

    if let Some(api_key) = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        builder = builder.bearer_auth(api_key);
    }

    let response = tokio::select! {
        _ = &mut cancel_rx => return Ok(AiStreamCompletion::Cancelled),
        response = timeout(Duration::from_secs(AI_CHAT_TIMEOUT_SECS + 5), builder.send()) => {
            response
                .map_err(|_| "AI stream request timed out".to_string())?
                .map_err(|error| error.to_string())?
        }
    };

    let status = response.status().as_u16();
    if status >= 400 {
        let text = response.text().await.unwrap_or_default();
        return Err(ai_stream_response_error(status, &text));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    loop {
        let chunk = tokio::select! {
            _ = &mut cancel_rx => return Ok(AiStreamCompletion::Cancelled),
            chunk = stream.next() => chunk,
        };

        let Some(chunk) = chunk else {
            break;
        };
        let bytes = chunk.map_err(|error| error.to_string())?;
        buffer.push_str(&String::from_utf8_lossy(&bytes));
        normalize_sse_buffer_newlines(&mut buffer);
        if emit_ai_stream_sse_events(app, stream_id, &mut buffer)? {
            return Ok(AiStreamCompletion::Done);
        }
    }

    normalize_sse_buffer_newlines(&mut buffer);
    if emit_ai_stream_sse_events(app, stream_id, &mut buffer)? {
        return Ok(AiStreamCompletion::Done);
    }
    if !buffer.trim().is_empty() && emit_ai_stream_sse_event(app, stream_id, buffer.trim())? {
        return Ok(AiStreamCompletion::Done);
    }

    emit_ai_stream_event(
        app,
        AiChatStreamEvent {
            stream_id: stream_id.to_string(),
            kind: "done".to_string(),
            data: None,
            error: None,
        },
    )?;
    Ok(AiStreamCompletion::Done)
}

fn ai_stream_payload(mut payload: Value) -> Value {
    if let Value::Object(object) = &mut payload {
        object.insert("stream".to_string(), Value::Bool(true));
    }
    payload
}

fn ai_stream_response_error(status: u16, text: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return ai_response_error(status, &value);
    }
    let message = text.trim();
    if message.is_empty() {
        format!("AI provider stream error {status}")
    } else {
        format!("AI provider stream error {status}: {message}")
    }
}

fn normalize_sse_buffer_newlines(buffer: &mut String) {
    if buffer.contains('\r') {
        *buffer = buffer.replace("\r\n", "\n").replace('\r', "\n");
    }
}

fn emit_ai_stream_sse_events(
    app: &AppHandle,
    stream_id: &str,
    buffer: &mut String,
) -> Result<bool, String> {
    while let Some(index) = buffer.find("\n\n") {
        let event = buffer[..index].to_string();
        buffer.drain(..index + 2);
        if emit_ai_stream_sse_event(app, stream_id, &event)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn emit_ai_stream_sse_event(app: &AppHandle, stream_id: &str, event: &str) -> Result<bool, String> {
    let Some(data) = sse_event_data(event) else {
        return Ok(false);
    };
    if data.trim() == "[DONE]" {
        emit_ai_stream_event(
            app,
            AiChatStreamEvent {
                stream_id: stream_id.to_string(),
                kind: "done".to_string(),
                data: None,
                error: None,
            },
        )?;
        return Ok(true);
    }

    let value = serde_json::from_str::<Value>(&data)
        .map_err(|error| format!("Invalid AI stream JSON chunk: {error}"))?;
    emit_ai_stream_event(
        app,
        AiChatStreamEvent {
            stream_id: stream_id.to_string(),
            kind: "chunk".to_string(),
            data: Some(value),
            error: None,
        },
    )?;
    Ok(false)
}

fn sse_event_data(event: &str) -> Option<String> {
    let lines = event
        .lines()
        .filter_map(|line| {
            let line = line.trim_end();
            if line.starts_with(':') {
                return None;
            }
            let data = line.strip_prefix("data:")?;
            Some(data.strip_prefix(' ').unwrap_or(data).to_string())
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn emit_ai_stream_event(app: &AppHandle, event: AiChatStreamEvent) -> Result<(), String> {
    app.emit("lux://ai-chat-stream", event)
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

async fn current_text_for_path(
    state: &State<'_, SharedState>,
    path: &Path,
) -> Result<String, String> {
    let open_document = {
        let documents = state.documents.lock().map_err(lock_error)?;
        documents.snapshot_for_path(path).map_err(String::from)?
    };
    if let Some(document) = open_document {
        return Ok(document.text);
    }
    tokio::fs::read_to_string(path)
        .await
        .map_err(|error| error.to_string())
}

async fn prepare_ai_patch_operations(
    state: &State<'_, SharedState>,
    operations: Vec<AiFilePatchOperation>,
) -> Result<Vec<AiPreparedPatchOperation>, String> {
    let mut prepared = Vec::with_capacity(operations.len());
    let mut next_text_by_path = BTreeMap::<PathBuf, Option<String>>::new();

    for operation in operations {
        let action = operation.action.trim().to_ascii_lowercase();
        let kind = match action.as_str() {
            "create" => AiPreparedPatchKind::Create,
            "write" | "rewrite" | "replacefile" | "replace_file" => AiPreparedPatchKind::Rewrite,
            "strreplace" | "str_replace" | "replace" => AiPreparedPatchKind::Replace,
            "delete" | "remove" => AiPreparedPatchKind::Delete,
            _ => return Err(format!("unsupported patch action: {}", operation.action)),
        };
        let path = match kind {
            AiPreparedPatchKind::Create | AiPreparedPatchKind::Rewrite => {
                resolve_workspace_path_for_write(state, &operation.path)?
            }
            AiPreparedPatchKind::Replace | AiPreparedPatchKind::Delete => {
                resolve_workspace_path(state, &operation.path)?
            }
        };
        let before_text = if let Some(previous) = next_text_by_path.get(&path) {
            previous.clone()
        } else if path.exists() {
            Some(current_text_for_path(state, &path).await?)
        } else {
            None
        };

        match kind {
            AiPreparedPatchKind::Create | AiPreparedPatchKind::Rewrite => {
                let text = operation.text.ok_or_else(|| {
                    format!("{} requires text for {}", operation.action, path.display())
                })?;
                if before_text.is_some()
                    && kind == AiPreparedPatchKind::Create
                    && !operation.overwrite.unwrap_or(false)
                {
                    return Err(format!("file already exists: {}", path.display()));
                }
                let stats = diff_stats(
                    before_text.as_deref().unwrap_or(""),
                    &text,
                    before_text.is_none(),
                );
                next_text_by_path.insert(path.clone(), Some(text.clone()));
                prepared.push(AiPreparedPatchOperation {
                    kind,
                    path,
                    after_text: Some(text),
                    stats,
                });
            }
            AiPreparedPatchKind::Replace => {
                let Some(before) = before_text else {
                    return Err(format!(
                        "file does not exist for replacement: {}",
                        path.display()
                    ));
                };
                let old_text = operation
                    .old_text
                    .ok_or_else(|| format!("replace requires oldText for {}", path.display()))?;
                if old_text.is_empty() {
                    return Err(format!("oldText must not be empty for {}", path.display()));
                }
                let new_text = operation.new_text.unwrap_or_default();
                let expected = operation.expected_replacements.unwrap_or(1);
                let replacement_count = before.matches(&old_text).count();
                if replacement_count != expected {
                    return Err(format!(
                        "replacement count mismatch for {}: expected {expected}, found {replacement_count}",
                        path.display()
                    ));
                }
                let after = before.replacen(&old_text, &new_text, expected);
                let stats = diff_stats(&before, &after, false);
                next_text_by_path.insert(path.clone(), Some(after.clone()));
                prepared.push(AiPreparedPatchOperation {
                    kind,
                    path,
                    after_text: Some(after),
                    stats,
                });
            }
            AiPreparedPatchKind::Delete => {
                let Some(before) = before_text else {
                    return Err(format!(
                        "file does not exist for deletion: {}",
                        path.display()
                    ));
                };
                if path.is_dir() {
                    return Err(format!(
                        "PatchEngine deletes files only, not directories: {}",
                        path.display()
                    ));
                }
                let stats = AiFileOperationStats {
                    lines_added: 0,
                    lines_removed: before.lines().count(),
                    files_changed: 0,
                    files_created: 0,
                    files_deleted: 1,
                };
                next_text_by_path.insert(path.clone(), None);
                prepared.push(AiPreparedPatchOperation {
                    kind,
                    path,
                    after_text: None,
                    stats,
                });
            }
        }
    }

    Ok(prepared)
}

async fn apply_ai_patch_to_disk(
    operations: &[AiPreparedPatchOperation],
    save_to_disk: bool,
    rollback: &mut Vec<AiPatchRollbackEntry>,
) -> Result<(), String> {
    if !save_to_disk {
        return Ok(());
    }

    for operation in operations {
        let previous_bytes = if operation.path.exists() {
            Some(
                tokio::fs::read(&operation.path)
                    .await
                    .map_err(|error| error.to_string())?,
            )
        } else {
            None
        };
        rollback.push(AiPatchRollbackEntry {
            path: operation.path.clone(),
            previous_bytes,
        });

        match operation.kind {
            AiPreparedPatchKind::Create
            | AiPreparedPatchKind::Rewrite
            | AiPreparedPatchKind::Replace => {
                let text = operation.after_text.as_deref().unwrap_or_default();
                if let Some(parent) = operation.path.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|error| error.to_string())?;
                }
                tokio::fs::write(&operation.path, text)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            AiPreparedPatchKind::Delete => {
                tokio::fs::remove_file(&operation.path)
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}

async fn rollback_ai_patch(mut rollback: Vec<AiPatchRollbackEntry>) {
    while let Some(entry) = rollback.pop() {
        match entry.previous_bytes {
            Some(bytes) => {
                let _ = tokio::fs::write(&entry.path, bytes).await;
            }
            None => {
                let _ = tokio::fs::remove_file(&entry.path).await;
            }
        }
    }
}

fn combine_patch_stats(operations: &[AiPreparedPatchOperation]) -> AiFileOperationStats {
    operations.iter().fold(
        AiFileOperationStats {
            lines_added: 0,
            lines_removed: 0,
            files_changed: 0,
            files_created: 0,
            files_deleted: 0,
        },
        |mut stats, operation| {
            stats.lines_added += operation.stats.lines_added;
            stats.lines_removed += operation.stats.lines_removed;
            stats.files_changed += operation.stats.files_changed;
            stats.files_created += operation.stats.files_created;
            stats.files_deleted += operation.stats.files_deleted;
            stats
        },
    )
}

fn unique_patch_paths(operations: &[AiPreparedPatchOperation]) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut paths = Vec::new();
    for operation in operations {
        if seen.insert(operation.path.clone()) {
            paths.push(operation.path.clone());
        }
    }
    paths
}

async fn symbol_context_document_for_path(
    state: &State<'_, SharedState>,
    path: &Path,
) -> Result<DocumentSnapshot, String> {
    let open_document = {
        let documents = state.documents.lock().map_err(lock_error)?;
        documents.snapshot_for_path(path).map_err(String::from)?
    };
    if let Some(document) = open_document {
        return Ok(document);
    }

    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err(format!(
            "symbol context path is not a file: {}",
            path.display()
        ));
    }
    let text = tokio::fs::read_to_string(path)
        .await
        .map_err(|error| error.to_string())?;
    Ok(DocumentSnapshot {
        id: BufferId::new(),
        path: Some(path.to_path_buf()),
        title: path
            .file_name()
            .and_then(|value| value.to_str())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        language_id: lux_editor::language_id_for_path(path),
        text,
        version: 1,
        is_dirty: false,
        is_untitled: false,
        opened_at: Utc::now(),
    })
}

fn truncate_document_symbols(symbols: &mut Vec<LspDocumentSymbol>, max_results: usize) {
    let mut remaining = max_results;
    symbols.retain_mut(|symbol| retain_symbol_with_budget(symbol, &mut remaining));
}

fn retain_symbol_with_budget(symbol: &mut LspDocumentSymbol, remaining: &mut usize) -> bool {
    if *remaining == 0 {
        return false;
    }
    *remaining -= 1;
    symbol
        .children
        .retain_mut(|child| retain_symbol_with_budget(child, remaining));
    true
}

fn filter_document_symbols(
    symbols: &[LspDocumentSymbol],
    query: &str,
    max_results: usize,
) -> Vec<LspDocumentSymbol> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        let mut symbols = symbols.to_vec();
        truncate_document_symbols(&mut symbols, max_results);
        return symbols;
    }

    let mut remaining = max_results;
    symbols
        .iter()
        .filter_map(|symbol| filter_symbol_with_budget(symbol, &needle, &mut remaining))
        .collect()
}

fn filter_symbol_with_budget(
    symbol: &LspDocumentSymbol,
    needle: &str,
    remaining: &mut usize,
) -> Option<LspDocumentSymbol> {
    if *remaining == 0 {
        return None;
    }
    let children = symbol
        .children
        .iter()
        .filter_map(|child| filter_symbol_with_budget(child, needle, remaining))
        .collect::<Vec<_>>();
    let matches = symbol.name.to_ascii_lowercase().contains(needle)
        || symbol
            .detail
            .as_deref()
            .is_some_and(|detail| detail.to_ascii_lowercase().contains(needle));

    if !matches && children.is_empty() {
        return None;
    }
    if *remaining == 0 {
        return None;
    }
    *remaining -= 1;
    let mut filtered = symbol.clone();
    filtered.children = children;
    Some(filtered)
}

async fn update_open_document_after_text_change(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    path: &Path,
    text: String,
    dirty: bool,
) -> Result<Option<DocumentSnapshot>, String> {
    let updated = {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        documents
            .replace_text_for_path(path, text, dirty)
            .map_err(String::from)?
    };
    if let Some(document) = &updated {
        emit_event(
            app,
            LuxEvent::EditorDocumentChanged {
                document: document.clone(),
            },
        )?;
        forward_document_update(app, state, document).await?;
    }
    Ok(updated)
}

fn diff_stats(before: &str, after: &str, created: bool) -> AiFileOperationStats {
    let before_lines = before.lines().count();
    let after_lines = after.lines().count();
    AiFileOperationStats {
        lines_added: after_lines.saturating_sub(before_lines),
        lines_removed: before_lines.saturating_sub(after_lines),
        files_changed: if created {
            0
        } else if before != after {
            1
        } else {
            0
        },
        files_created: usize::from(created),
        files_deleted: 0,
    }
}

fn detect_test_health_plans(root: &Path) -> Result<Vec<TestHealthPlan>, String> {
    let mut directories = collect_test_health_scan_dirs(root);
    directories.sort_by(|left, right| {
        relative_depth(root, left)
            .cmp(&relative_depth(root, right))
            .then_with(|| left.cmp(right))
    });

    let root_caps = RootTestHealthCapabilities::from_root(root);
    let mut plans = Vec::new();
    let mut seen = BTreeSet::new();
    for directory in directories {
        add_test_health_plans_for_dir(root, &directory, &root_caps, &mut seen, &mut plans);
    }
    Ok(plans)
}

#[derive(Debug, Clone)]
struct RootTestHealthCapabilities {
    cargo_workspace: bool,
    maven_multi_module: bool,
    gradle_multi_project: bool,
    dotnet: bool,
}

impl RootTestHealthCapabilities {
    fn from_root(root: &Path) -> Self {
        Self {
            cargo_workspace: cargo_manifest_has_workspace(root),
            maven_multi_module: maven_manifest_has_modules(root),
            gradle_multi_project: has_gradle_settings(root),
            dotnet: find_file_with_extension(root, "sln").is_some(),
        }
    }
}

fn add_test_health_plans_for_dir(
    root: &Path,
    directory: &Path,
    root_caps: &RootTestHealthCapabilities,
    seen: &mut BTreeSet<String>,
    plans: &mut Vec<TestHealthPlan>,
) {
    let is_root = same_path(root, directory);

    if has_package_test_script(directory) {
        push_test_health_command(
            seen,
            plans,
            "test",
            "JavaScript/TypeScript",
            "package.json test script",
            directory,
            package_manager_script_command(directory, "test"),
        );
    } else {
        add_package_validation_plans(directory, seen, plans);
    }

    if directory.join("Cargo.toml").is_file() && (is_root || !root_caps.cargo_workspace) {
        let command = if cargo_manifest_has_workspace(directory) {
            "cargo test --workspace"
        } else {
            "cargo test"
        };
        push_test_health_command(seen, plans, "test", "Rust", "Cargo", directory, command);
    }

    if is_python_test_project(directory) {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Python",
            "pytest",
            directory,
            python_test_command(directory),
        );
    }

    if directory.join("go.mod").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Go",
            "go test",
            directory,
            "go test ./...",
        );
    }

    if directory.join("pom.xml").is_file() && (is_root || !root_caps.maven_multi_module) {
        push_test_health_command(seen, plans, "test", "Java", "Maven", directory, "mvn test");
    }

    if is_gradle_project(directory) && (is_root || !root_caps.gradle_multi_project) {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Java/Kotlin",
            "Gradle",
            directory,
            gradle_test_command(directory),
        );
    }

    if (find_file_with_extension(directory, "sln").is_some()
        || find_file_with_extension(directory, "csproj").is_some())
        && (is_root || !root_caps.dotnet)
    {
        push_test_health_command(
            seen,
            plans,
            "test",
            ".NET",
            "dotnet test",
            directory,
            dotnet_test_command(directory),
        );
    }

    if has_composer_test_script(directory) {
        push_test_health_command(
            seen,
            plans,
            "test",
            "PHP",
            "Composer",
            directory,
            "composer test",
        );
    }

    if directory.join("Gemfile").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Ruby",
            "Bundler",
            directory,
            ruby_test_command(directory),
        );
    }

    if directory.join("mix.exs").is_file() {
        push_test_health_command(seen, plans, "test", "Elixir", "Mix", directory, "mix test");
    }

    if directory.join("Package.swift").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Swift",
            "SwiftPM",
            directory,
            "swift test",
        );
    }

    if directory.join("deno.json").is_file() || directory.join("deno.jsonc").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "TypeScript/JavaScript",
            "Deno",
            directory,
            "deno test",
        );
    }

    if directory.join("pubspec.yaml").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Dart/Flutter",
            "Dart test",
            directory,
            dart_test_command(directory),
        );
    }

    if directory.join("build.sbt").is_file() {
        push_test_health_command(seen, plans, "test", "Scala", "sbt", directory, "sbt test");
    }

    if directory.join("stack.yaml").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Haskell",
            "Stack",
            directory,
            "stack test",
        );
    } else if directory.join("cabal.project").is_file()
        || find_file_with_extension(directory, "cabal").is_some()
    {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Haskell",
            "Cabal",
            directory,
            "cabal test all",
        );
    }

    if directory.join("build.zig").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Zig",
            "zig build test",
            directory,
            "zig build test",
        );
    }

    if directory.join("dune-project").is_file() || directory.join("dune").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "OCaml",
            "Dune",
            directory,
            "dune runtest",
        );
    }

    if directory.join("rebar.config").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Erlang",
            "rebar3",
            directory,
            "rebar3 eunit",
        );
    }

    if directory.join("shard.yml").is_file() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Crystal",
            "spec",
            directory,
            "crystal spec",
        );
    }

    if find_file_with_extension(directory, "nimble").is_some() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Nim",
            "nimble",
            directory,
            "nimble test",
        );
    }

    if directory.join("Project.toml").is_file() && directory.join("test").is_dir() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Julia",
            "Pkg.test",
            directory,
            "julia --project -e \"using Pkg; Pkg.test()\"",
        );
    }

    if directory.join("DESCRIPTION").is_file() && directory.join("tests/testthat").is_dir() {
        push_test_health_command(
            seen,
            plans,
            "test",
            "R",
            "testthat",
            directory,
            "Rscript -e \"testthat::test_dir('tests/testthat')\"",
        );
    }

    if directory.join("Makefile.PL").is_file()
        || directory.join("cpanfile").is_file()
        || directory.join("t").is_dir()
    {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Perl",
            "prove",
            directory,
            "prove -lr t",
        );
    }

    if let Some(ctest_dir) = ctest_working_dir(directory) {
        push_test_health_plan(
            seen,
            plans,
            TestHealthPlan {
                kind: "test",
                language: "C/C++",
                framework: "CTest",
                working_dir: ctest_dir,
                command: "ctest --output-on-failure".to_string(),
            },
        );
    }

    if has_make_target(directory, "test") {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Project",
            "Make",
            directory,
            "make test",
        );
    }

    if has_just_recipe(directory, "test") {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Project",
            "just",
            directory,
            "just test",
        );
    }

    if has_taskfile_task(directory, "test") {
        push_test_health_command(
            seen,
            plans,
            "test",
            "Project",
            "Taskfile",
            directory,
            "task test",
        );
    }
}

fn collect_test_health_scan_dirs(root: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    let mut seen = BTreeSet::new();

    while let Some((directory, depth)) = queue.pop_front() {
        let key = normalize_watch_path_for_compare(&directory);
        if !seen.insert(key) {
            continue;
        }
        result.push(directory.clone());
        if depth >= AI_TEST_HEALTH_SCAN_MAX_DEPTH {
            continue;
        }

        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        let mut child_dirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() || is_ignored_test_health_scan_dir(&path) {
                continue;
            }
            child_dirs.push(path);
        }
        child_dirs.sort();
        queue.extend(child_dirs.into_iter().map(|path| (path, depth + 1)));
    }

    result
}

fn push_test_health_plan(
    seen: &mut BTreeSet<String>,
    plans: &mut Vec<TestHealthPlan>,
    plan: TestHealthPlan,
) {
    let key = format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        normalize_watch_path_for_compare(&plan.working_dir),
        plan.kind,
        plan.framework,
        plan.command
    );
    if seen.insert(key) {
        plans.push(plan);
    }
}

fn push_test_health_command(
    seen: &mut BTreeSet<String>,
    plans: &mut Vec<TestHealthPlan>,
    kind: &'static str,
    language: &'static str,
    framework: &'static str,
    working_dir: &Path,
    command: impl Into<String>,
) {
    push_test_health_plan(
        seen,
        plans,
        TestHealthPlan {
            kind,
            language,
            framework,
            working_dir: working_dir.to_path_buf(),
            command: command.into(),
        },
    );
}

fn add_package_validation_plans(
    directory: &Path,
    seen: &mut BTreeSet<String>,
    plans: &mut Vec<TestHealthPlan>,
) {
    let Some(scripts) = package_json_scripts(directory) else {
        return;
    };

    for script in [
        "test:ci",
        "test:unit",
        "test:integration",
        "test:e2e",
        "unit",
        "spec",
    ] {
        if has_valid_package_script(&scripts, script) {
            push_test_health_command(
                seen,
                plans,
                "test",
                "JavaScript/TypeScript",
                "package.json test script",
                directory,
                package_manager_script_command(directory, script),
            );
        }
    }

    for (script, kind, framework) in [
        ("typecheck", "typecheck", "package.json typecheck script"),
        ("check", "check", "package.json check script"),
        ("lint", "lint", "package.json lint script"),
        ("build", "build", "package.json build script"),
    ] {
        if has_valid_package_script(&scripts, script) {
            push_test_health_command(
                seen,
                plans,
                kind,
                "JavaScript/TypeScript",
                framework,
                directory,
                package_manager_script_command(directory, script),
            );
        }
    }
}

fn package_json_scripts(directory: &Path) -> Option<serde_json::Map<String, Value>> {
    read_json_file(&directory.join("package.json"))
        .and_then(|value| value.get("scripts").and_then(Value::as_object).cloned())
}

fn has_valid_package_script(scripts: &serde_json::Map<String, Value>, name: &str) -> bool {
    scripts
        .get(name)
        .and_then(Value::as_str)
        .map(is_meaningful_package_script)
        .unwrap_or(false)
}

fn is_meaningful_package_script(script: &str) -> bool {
    let script = script.trim().to_ascii_lowercase();
    !script.is_empty()
        && !script.contains("no test specified")
        && !script.contains("echo \"error:")
        && !script.contains("exit 1")
        && !is_package_watch_script(&script)
}

fn is_package_watch_script(script: &str) -> bool {
    let is_explicit_false = script.contains("--watch=false")
        || script.contains("--watch false")
        || script.contains("--watchall=false")
        || script.contains("--watchall false");
    !is_explicit_false
        && (script.contains("--watch") || script.contains(" watch") || script.ends_with("watch"))
}

fn has_package_test_script(directory: &Path) -> bool {
    package_json_scripts(directory)
        .map(|scripts| has_valid_package_script(&scripts, "test"))
        .unwrap_or(false)
}

fn has_composer_test_script(directory: &Path) -> bool {
    let Some(value) = read_json_file(&directory.join("composer.json")) else {
        return false;
    };
    value
        .get("scripts")
        .and_then(Value::as_object)
        .and_then(|scripts| scripts.get("test"))
        .is_some()
}

fn read_json_file(path: &Path) -> Option<Value> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn is_python_test_project(directory: &Path) -> bool {
    directory.join("pytest.ini").is_file()
        || directory.join("tox.ini").is_file()
        || directory.join("noxfile.py").is_file()
        || directory.join("pyproject.toml").is_file()
        || directory.join("setup.cfg").is_file()
}

fn python_test_command(directory: &Path) -> String {
    if directory.join("tox.ini").is_file() {
        "python -m tox".to_string()
    } else if directory.join("noxfile.py").is_file() {
        "python -m nox".to_string()
    } else {
        "python -m pytest".to_string()
    }
}

fn cargo_manifest_has_workspace(directory: &Path) -> bool {
    std::fs::read_to_string(directory.join("Cargo.toml"))
        .map(|content| content.lines().any(|line| line.trim() == "[workspace]"))
        .unwrap_or(false)
}

fn is_gradle_project(directory: &Path) -> bool {
    directory.join("build.gradle").is_file()
        || directory.join("build.gradle.kts").is_file()
        || directory.join("settings.gradle").is_file()
        || directory.join("settings.gradle.kts").is_file()
}

fn gradle_test_command(directory: &Path) -> String {
    if cfg!(windows) && directory.join("gradlew.bat").is_file() {
        "gradlew.bat test".to_string()
    } else if directory.join("gradlew").is_file() {
        "./gradlew test".to_string()
    } else {
        "gradle test".to_string()
    }
}

fn package_manager_script_command(directory: &Path, script: &str) -> String {
    if directory.join("pnpm-lock.yaml").is_file()
        || nearest_parent_has_file(directory, "pnpm-lock.yaml")
    {
        format!("pnpm {script}")
    } else if directory.join("yarn.lock").is_file()
        || nearest_parent_has_file(directory, "yarn.lock")
    {
        format!("yarn {script}")
    } else if directory.join("bun.lockb").is_file()
        || directory.join("bun.lock").is_file()
        || nearest_parent_has_file(directory, "bun.lockb")
        || nearest_parent_has_file(directory, "bun.lock")
    {
        if script == "test" {
            "bun test".to_string()
        } else {
            format!("bun run {script}")
        }
    } else if script == "test" {
        "npm test".to_string()
    } else {
        format!("npm run {script}")
    }
}

fn maven_manifest_has_modules(directory: &Path) -> bool {
    std::fs::read_to_string(directory.join("pom.xml"))
        .map(|content| content.contains("<modules>") && content.contains("<module>"))
        .unwrap_or(false)
}

fn has_gradle_settings(directory: &Path) -> bool {
    directory.join("settings.gradle").is_file() || directory.join("settings.gradle.kts").is_file()
}

fn dotnet_test_command(directory: &Path) -> String {
    if let Some(solution) = find_file_with_extension(directory, "sln") {
        format!("dotnet test {}", shell_quote_path(&solution))
    } else if let Some(project) = find_file_with_extension(directory, "csproj") {
        format!("dotnet test {}", shell_quote_path(&project))
    } else {
        "dotnet test".to_string()
    }
}

fn dart_test_command(directory: &Path) -> String {
    std::fs::read_to_string(directory.join("pubspec.yaml"))
        .map(|content| {
            if content.lines().any(|line| line.trim() == "flutter:") {
                "flutter test".to_string()
            } else {
                "dart test".to_string()
            }
        })
        .unwrap_or_else(|_| "dart test".to_string())
}

fn ruby_test_command(directory: &Path) -> &'static str {
    if directory.join("Rakefile").is_file() {
        "bundle exec rake test"
    } else {
        "bundle exec rspec"
    }
}

fn ctest_working_dir(directory: &Path) -> Option<PathBuf> {
    if directory.join("CTestTestfile.cmake").is_file() {
        return Some(directory.to_path_buf());
    }

    for child in [
        "build",
        "cmake-build-debug",
        "cmake-build-release",
        "out/build",
    ] {
        let candidate = directory.join(child);
        if candidate.join("CTestTestfile.cmake").is_file() {
            return Some(candidate);
        }
    }

    None
}

fn has_make_target(directory: &Path, target: &str) -> bool {
    ["Makefile", "makefile", "GNUmakefile"]
        .iter()
        .any(|file_name| file_has_recipe_target(&directory.join(file_name), target))
}

fn has_just_recipe(directory: &Path, target: &str) -> bool {
    ["justfile", "Justfile", ".justfile"]
        .iter()
        .any(|file_name| file_has_recipe_target(&directory.join(file_name), target))
}

fn has_taskfile_task(directory: &Path, target: &str) -> bool {
    [
        "Taskfile.yml",
        "Taskfile.yaml",
        "Taskfile.dist.yml",
        "Taskfile.dist.yaml",
    ]
    .iter()
    .any(|file_name| taskfile_has_task(&directory.join(file_name), target))
}

fn file_has_recipe_target(path: &Path, target: &str) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    content.lines().any(|line| {
        let trimmed = line.trim_start();
        !trimmed.starts_with('#')
            && trimmed.starts_with(target)
            && trimmed[target.len()..].trim_start().starts_with(':')
    })
}

fn taskfile_has_task(path: &Path, target: &str) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let target_line = format!("  {target}:");
    content
        .lines()
        .any(|line| line == target_line || line.trim_start() == format!("{target}:"))
}

fn find_file_with_extension(directory: &Path, extension: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(directory).ok()?;
    let mut matches = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .map(|value| value.to_string_lossy().eq_ignore_ascii_case(extension))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    matches.sort();
    matches.into_iter().next()
}

fn nearest_parent_has_file(directory: &Path, file_name: &str) -> bool {
    directory
        .ancestors()
        .skip(1)
        .take(4)
        .any(|parent| parent.join(file_name).is_file())
}

fn shell_quote_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | '\\' | ':'))
    {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('"', "\\\""))
    }
}

fn is_ignored_test_health_scan_dir(path: &Path) -> bool {
    let Some(name) = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
    else {
        return true;
    };
    WATCH_EXCLUDED_COMPONENTS.contains(&name.as_str())
        || matches!(
            name.as_str(),
            ".cache"
                | ".gradle"
                | ".idea"
                | ".pytest_cache"
                | ".ruff_cache"
                | ".venv"
                | ".vscode"
                | "__pycache__"
                | "build"
                | "out"
                | "venv"
        )
}

fn same_path(left: &Path, right: &Path) -> bool {
    normalize_watch_path_for_compare(left) == normalize_watch_path_for_compare(right)
}

fn relative_depth(root: &Path, path: &Path) -> usize {
    path.strip_prefix(root)
        .ok()
        .map(|relative| {
            relative
                .components()
                .filter(|component| matches!(component, Component::Normal(_)))
                .count()
        })
        .unwrap_or(usize::MAX)
}

async fn run_test_health_plans(
    root: PathBuf,
    plans: Vec<TestHealthPlan>,
) -> Result<TestHealthResponse, String> {
    if plans.is_empty() {
        return Ok(empty_test_health_response(root));
    }

    let started = std::time::Instant::now();
    let skipped = plans.len().saturating_sub(AI_TEST_HEALTH_MAX_RUNNERS);
    let mut runners = Vec::new();
    for plan in plans.into_iter().take(AI_TEST_HEALTH_MAX_RUNNERS) {
        runners.push(run_single_test_health_plan(&root, plan).await);
    }
    let total_duration_ms = started.elapsed().as_millis();
    Ok(test_health_response_from_runners(
        root,
        runners,
        skipped,
        total_duration_ms,
    ))
}

async fn run_single_test_health_plan(root: &Path, plan: TestHealthPlan) -> TestHealthRunnerResult {
    let started = std::time::Instant::now();
    let mut command = shell_command(&plan.command);
    command.current_dir(&plan.working_dir);
    let output_result = timeout(
        Duration::from_secs(AI_TEST_HEALTH_TIMEOUT_SECS),
        command.output(),
    )
    .await;
    let duration_ms = started.elapsed().as_millis();
    let id = test_health_runner_id(root, &plan);
    let workspace_relative_path = workspace_relative_path(root, &plan.working_dir);

    match output_result {
        Ok(Ok(output)) => TestHealthRunnerResult {
            id,
            workspace_relative_path,
            status: if output.status.success() {
                "passed".to_string()
            } else {
                "failed".to_string()
            },
            kind: plan.kind.to_string(),
            language: plan.language.to_string(),
            framework: plan.framework.to_string(),
            command: plan.command,
            exit_code: output.status.code(),
            duration_ms,
            stdout: truncate_output(&String::from_utf8_lossy(&output.stdout)),
            stderr: truncate_output(&String::from_utf8_lossy(&output.stderr)),
            timed_out: false,
        },
        Ok(Err(error)) => TestHealthRunnerResult {
            id,
            workspace_relative_path,
            status: "error".to_string(),
            kind: plan.kind.to_string(),
            language: plan.language.to_string(),
            framework: plan.framework.to_string(),
            command: plan.command,
            exit_code: None,
            duration_ms,
            stdout: String::new(),
            stderr: format!("Failed to start test command: {error}"),
            timed_out: false,
        },
        Err(_) => TestHealthRunnerResult {
            id,
            workspace_relative_path,
            status: "timeout".to_string(),
            kind: plan.kind.to_string(),
            language: plan.language.to_string(),
            framework: plan.framework.to_string(),
            command: plan.command,
            exit_code: None,
            duration_ms,
            stdout: String::new(),
            stderr: format!("Test command timed out after {AI_TEST_HEALTH_TIMEOUT_SECS} seconds"),
            timed_out: true,
        },
    }
}

fn empty_test_health_response(root: PathBuf) -> TestHealthResponse {
    TestHealthResponse {
        workspace_root: root,
        status: "skipped".to_string(),
        summary: TestHealthSummary {
            total: 0,
            passed: 0,
            failed: 0,
            timed_out: 0,
            errored: 0,
            skipped: 0,
            duration_ms: 0,
        },
        runners: Vec::new(),
        language: "Mixed".to_string(),
        framework: "No supported test runner".to_string(),
        command: String::new(),
        exit_code: None,
        duration_ms: 0,
        stdout: String::new(),
        stderr: "No supported test runner was detected in the workspace.".to_string(),
        timed_out: false,
    }
}

fn test_health_response_from_runners(
    root: PathBuf,
    runners: Vec<TestHealthRunnerResult>,
    skipped: usize,
    duration_ms: u128,
) -> TestHealthResponse {
    let summary = TestHealthSummary {
        total: runners.len() + skipped,
        passed: runners
            .iter()
            .filter(|runner| runner.status == "passed")
            .count(),
        failed: runners
            .iter()
            .filter(|runner| runner.status == "failed")
            .count(),
        timed_out: runners
            .iter()
            .filter(|runner| runner.status == "timeout")
            .count(),
        errored: runners
            .iter()
            .filter(|runner| runner.status == "error")
            .count(),
        skipped,
        duration_ms,
    };
    let status = aggregate_test_health_status(&summary);
    let primary = runners
        .iter()
        .find(|runner| runner.status != "passed")
        .or_else(|| runners.first());
    let language = aggregate_test_health_language(&runners);
    let framework = if runners.len() == 1 {
        primary
            .map(|runner| runner.framework.clone())
            .unwrap_or_default()
    } else {
        format!("{} runners", runners.len())
    };
    let command = if runners.len() == 1 {
        primary
            .map(|runner| runner.command.clone())
            .unwrap_or_default()
    } else {
        runners
            .iter()
            .take(4)
            .map(|runner| format!("{}: {}", runner.workspace_relative_path, runner.command))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let exit_code = primary.and_then(|runner| runner.exit_code);
    let timed_out = summary.timed_out > 0;
    let stdout = aggregate_test_stream(&runners, false);
    let stderr = aggregate_test_stream(&runners, true);

    TestHealthResponse {
        workspace_root: root,
        status,
        summary,
        runners,
        language,
        framework,
        command,
        exit_code,
        duration_ms,
        stdout,
        stderr,
        timed_out,
    }
}

fn aggregate_test_health_status(summary: &TestHealthSummary) -> String {
    if summary.failed > 0 {
        "failed".to_string()
    } else if summary.timed_out > 0 {
        "timeout".to_string()
    } else if summary.errored > 0 {
        "error".to_string()
    } else if summary.passed > 0 && summary.skipped == 0 {
        "passed".to_string()
    } else if summary.passed > 0 {
        "partial".to_string()
    } else {
        "skipped".to_string()
    }
}

fn aggregate_test_health_language(runners: &[TestHealthRunnerResult]) -> String {
    let languages = runners
        .iter()
        .map(|runner| runner.language.as_str())
        .collect::<BTreeSet<_>>();
    if languages.len() == 1 {
        languages.into_iter().next().unwrap_or("Mixed").to_string()
    } else {
        "Mixed".to_string()
    }
}

fn aggregate_test_stream(runners: &[TestHealthRunnerResult], stderr: bool) -> String {
    let mut sections = Vec::new();
    for runner in runners {
        let output = if stderr {
            &runner.stderr
        } else {
            &runner.stdout
        };
        if output.trim().is_empty() {
            continue;
        }
        sections.push(format!(
            "## {} [{} / {}]\n{}",
            runner.workspace_relative_path, runner.kind, runner.framework, output
        ));
    }
    truncate_output(&sections.join("\n\n"))
}

fn test_health_runner_id(root: &Path, plan: &TestHealthPlan) -> String {
    format!(
        "{}:{}:{}",
        workspace_relative_path(root, &plan.working_dir),
        plan.kind,
        plan.framework
            .to_ascii_lowercase()
            .replace(|ch: char| !ch.is_ascii_alphanumeric(), "-")
    )
}

fn workspace_relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| ".".to_string())
}

fn truncate_output(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= AI_TEST_HEALTH_MAX_OUTPUT_CHARS {
        return trimmed.to_string();
    }
    let head: String = trimmed
        .chars()
        .take(AI_TEST_HEALTH_MAX_OUTPUT_CHARS)
        .collect();
    format!("{head}\n...[truncated]")
}

fn shell_command(command_line: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let mut command = tokio::process::Command::new("cmd");
        command.arg("/C").arg(command_line);
        command
    }
    #[cfg(not(windows))]
    {
        let mut command = tokio::process::Command::new("sh");
        command.arg("-c").arg(command_line);
        command
    }
}

fn close_all_terminals(state: &State<'_, SharedState>) -> Result<(), String> {
    let service = state
        .terminals
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .cloned()
        .ok_or_else(|| "terminal service is not initialized".to_string())?;
    service.close_all().map_err(String::from)
}

async fn pick_save_path(app: &AppHandle, suggested_name: &str) -> Result<Option<PathBuf>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Save File")
        .set_file_name(suggested_name)
        .save_file(move |file| {
            let _ = sender.send(file);
        });

    let Some(file) = receiver.await.map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    file.into_path()
        .map(Some)
        .map_err(|error| error.to_string())
}

fn start_workspace_watcher(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    root: PathBuf,
) -> Result<(), String> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<PathBuf>();
    let watch_root = root.clone();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let event = match result {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(%error, "workspace file watcher event failed");
                return;
            }
        };

        if !is_mutating_watch_event(&event.kind) {
            return;
        }

        for path in event.paths {
            if is_publishable_watch_path(&watch_root, &path) {
                let _ = tx.send(normalize_watch_event_path(&watch_root, path));
            }
        }
    })
    .map_err(|error| error.to_string())?;

    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|error| error.to_string())?;
    *state.workspace_watcher.lock().map_err(lock_error)? = Some(watcher);

    tauri::async_runtime::spawn(forward_workspace_fs_events(app.clone(), root, rx));
    Ok(())
}

fn stop_workspace_watcher(state: &State<'_, SharedState>) -> Result<(), String> {
    *state.workspace_watcher.lock().map_err(lock_error)? = None;
    Ok(())
}

async fn forward_workspace_fs_events(
    app: AppHandle,
    root: PathBuf,
    mut rx: UnboundedReceiver<PathBuf>,
) {
    while let Some(first_path) = rx.recv().await {
        let mut paths = BTreeSet::new();
        let mut collapsed = false;
        push_watch_path(&root, first_path, &mut paths, &mut collapsed);

        sleep(Duration::from_millis(WATCH_DEBOUNCE_MS)).await;

        while let Ok(path) = rx.try_recv() {
            push_watch_path(&root, path, &mut paths, &mut collapsed);
        }

        if collapsed {
            paths.clear();
            paths.insert(root.clone());
        }

        for path in paths {
            let _ = emit_event(&app, LuxEvent::FsChanged { path });
        }
    }
}

fn push_watch_path(
    root: &Path,
    path: PathBuf,
    paths: &mut BTreeSet<PathBuf>,
    collapsed: &mut bool,
) {
    if !is_publishable_watch_path(root, &path) {
        return;
    }
    if paths.len() >= WATCH_MAX_BATCHED_PATHS {
        *collapsed = true;
        return;
    }
    paths.insert(normalize_watch_event_path(root, path));
}

fn is_mutating_watch_event(kind: &EventKind) -> bool {
    !kind.is_access() && !matches!(kind, EventKind::Other)
}

fn is_publishable_watch_path(root: &Path, path: &Path) -> bool {
    let path = normalize_watch_event_path(root, path.to_path_buf());
    path_is_within_root(root, &path) && !has_excluded_watch_component(root, &path)
}

fn normalize_watch_event_path(root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn path_is_within_root(root: &Path, path: &Path) -> bool {
    let normalized_root = normalize_watch_path_for_compare(root);
    let normalized_path = normalize_watch_path_for_compare(path);
    normalized_path == normalized_root
        || normalized_path.starts_with(&format!("{normalized_root}/"))
}

fn has_excluded_watch_component(root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative.components().any(|component| match component {
        Component::Normal(name) => {
            let name = name.to_string_lossy().to_ascii_lowercase();
            WATCH_EXCLUDED_COMPONENTS.contains(&name.as_str())
        }
        _ => false,
    })
}

fn normalize_watch_path_for_compare(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn replace_diagnostics(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    diagnostics: Vec<WorkspaceDiagnostic>,
) -> Result<(), String> {
    let mut by_path: BTreeMap<PathBuf, Vec<WorkspaceDiagnostic>> = BTreeMap::new();
    for diagnostic in &diagnostics {
        by_path
            .entry(diagnostic.path.clone())
            .or_default()
            .push(diagnostic.clone());
    }

    let previous_paths = {
        let mut current = state.diagnostics.lock().map_err(lock_error)?;
        let paths = current
            .iter()
            .map(|diagnostic| diagnostic.path.clone())
            .collect::<Vec<_>>();
        *current = diagnostics;
        paths
    };

    for path in previous_paths {
        by_path.entry(path).or_default();
    }

    for (path, path_diagnostics) in by_path {
        emit_event(
            app,
            LuxEvent::EditorDiagnosticsChanged {
                path,
                diagnostics: path_diagnostics,
            },
        )?;
    }

    Ok(())
}

fn clear_diagnostics(app: &AppHandle, state: &State<'_, SharedState>) -> Result<(), String> {
    let previous_paths = {
        let mut current = state.diagnostics.lock().map_err(lock_error)?;
        let paths = current
            .iter()
            .map(|diagnostic| diagnostic.path.clone())
            .collect::<Vec<_>>();
        current.clear();
        paths
    };

    for path in previous_paths {
        emit_event(
            app,
            LuxEvent::EditorDiagnosticsChanged {
                path,
                diagnostics: Vec::new(),
            },
        )?;
    }

    Ok(())
}

fn apply_diagnostics_update(
    app: &AppHandle,
    state: &SharedState,
    update: lux_lsp::DiagnosticsUpdate,
) -> Result<(), String> {
    {
        let mut current = state.diagnostics.lock().map_err(lock_error)?;
        current.retain(|diagnostic| diagnostic.path != update.path);
        current.extend(update.diagnostics.clone());
    }

    emit_event(
        app,
        LuxEvent::EditorDiagnosticsChanged {
            path: update.path,
            diagnostics: update.diagnostics,
        },
    )
}

fn record_recent_workspace(
    state: &State<'_, SharedState>,
    workspace: &WorkspaceInfo,
) -> Result<(), String> {
    with_settings(state, |settings| {
        settings.record_recent_workspace(workspace)
    })
    .map(|_| ())
    .map_err(String::from)
}

fn with_settings<T>(
    state: &State<'_, SharedState>,
    action: impl FnOnce(&mut SettingsStore) -> lux_core::AppResult<T>,
) -> lux_core::AppResult<T> {
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| lux_core::AppError::Service("settings lock poisoned".to_string()))?;
    let settings = settings.as_mut().ok_or_else(|| {
        lux_core::AppError::Service("settings store is not initialized".to_string())
    })?;
    action(settings)
}

async fn start_lsp_servers(
    state: &State<'_, SharedState>,
    servers: &[LanguageServerInfo],
) -> Result<Vec<WorkspaceDiagnostic>, String> {
    let mut lsp = state.lsp.lock().await;
    let manager = lsp
        .as_mut()
        .ok_or_else(|| "language service is not initialized".to_string())?;
    Ok(manager.start_available_servers(servers).await)
}

async fn shutdown_lsp(state: &State<'_, SharedState>) {
    let mut lsp = state.lsp.lock().await;
    if let Some(manager) = lsp.as_mut() {
        manager.shutdown_all().await;
    }
}

async fn forward_document_open(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    document: &DocumentSnapshot,
) -> Result<(), String> {
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(());
    };
    if let Err(error) = manager.open_document(document).await {
        publish_lsp_forwarding_error(app, state, document, error)?;
    }
    Ok(())
}

async fn forward_document_update(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    document: &DocumentSnapshot,
) -> Result<(), String> {
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(());
    };
    if let Err(error) = manager.update_document(document).await {
        publish_lsp_forwarding_error(app, state, document, error)?;
    }
    Ok(())
}

async fn forward_document_edits(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    document: &DocumentSnapshot,
    edits: &[TextEdit],
) -> Result<(), String> {
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(());
    };
    if let Err(error) = manager.apply_document_edits(document, edits).await {
        publish_lsp_forwarding_error(app, state, document, error)?;
    }
    Ok(())
}

async fn forward_document_save(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    document: &DocumentSnapshot,
) -> Result<(), String> {
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(());
    };
    if let Err(error) = manager.save_document(document).await {
        publish_lsp_forwarding_error(app, state, document, error)?;
    }
    Ok(())
}

async fn forward_document_close(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    path: &Path,
) -> Result<(), String> {
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(());
    };
    if let Err(error) = manager.close_document(path).await {
        apply_diagnostics_update(
            app,
            state.inner(),
            lux_lsp::DiagnosticsUpdate {
                path: path.to_path_buf(),
                diagnostics: vec![WorkspaceDiagnostic {
                    path: path.to_path_buf(),
                    line: 1,
                    column: 1,
                    severity: lux_core::DiagnosticSeverity::Warning,
                    source: "lux-lsp".to_string(),
                    message: format!("Language server stopped: {error}"),
                }],
            },
        )?;
    }
    Ok(())
}

fn publish_lsp_forwarding_error(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    document: &DocumentSnapshot,
    error: lux_core::AppError,
) -> Result<(), String> {
    let Some(path) = document.path.clone() else {
        return Ok(());
    };
    apply_diagnostics_update(
        app,
        state.inner(),
        lux_lsp::DiagnosticsUpdate {
            path: path.clone(),
            diagnostics: vec![WorkspaceDiagnostic {
                path,
                line: 1,
                column: 1,
                severity: lux_core::DiagnosticSeverity::Warning,
                source: "lux-lsp".to_string(),
                message: format!("Language server stopped: {error}"),
            }],
        },
    )
}

fn local_voice_input_status(
    command: Option<String>,
    model_path: Option<PathBuf>,
) -> VoiceInputProviderStatus {
    let command = command.and_then(non_empty_string).or_else(|| {
        env::var(LOCAL_STT_COMMAND_ENV)
            .ok()
            .and_then(non_empty_string)
    });
    let model_path = model_path.or_else(|| env_path(LOCAL_STT_MODEL_ENV));

    let Some(command_value) = command else {
        return VoiceInputProviderStatus {
            provider: "local".to_string(),
            available: false,
            detail: format!("Set {LOCAL_STT_COMMAND_ENV} or AI settings Local STT command"),
            command: None,
            model_path,
        };
    };

    let Some(executable) = first_command_token(&command_value) else {
        return VoiceInputProviderStatus {
            provider: "local".to_string(),
            available: false,
            detail: "Local STT command is empty".to_string(),
            command: Some(command_value),
            model_path,
        };
    };

    if !command_token_available(&executable) {
        return VoiceInputProviderStatus {
            provider: "local".to_string(),
            available: false,
            detail: format!("Local STT executable not found: {executable}"),
            command: Some(command_value),
            model_path,
        };
    }

    if let Some(model) = &model_path {
        if !model.exists() {
            return VoiceInputProviderStatus {
                provider: "local".to_string(),
                available: false,
                detail: format!("Local STT model path does not exist: {}", model.display()),
                command: Some(command_value),
                model_path,
            };
        }
    }

    VoiceInputProviderStatus {
        provider: "local".to_string(),
        available: true,
        detail: "Local STT command is configured".to_string(),
        command: Some(command_value),
        model_path,
    }
}

async fn run_local_stt_command(
    command_template: &str,
    audio: &[u8],
    mime_type: &str,
    language: Option<&str>,
    model_path: Option<&Path>,
) -> Result<String, String> {
    let audio_path = write_temp_audio(audio, mime_type)?;
    let command_line = render_stt_command(
        command_template,
        &audio_path,
        mime_type,
        language,
        model_path,
    );
    let mut command = local_stt_shell_command(&command_line);
    let output_result = tokio::time::timeout(Duration::from_secs(120), command.output())
        .await
        .map_err(|_| "Local STT command timed out after 120 seconds".to_string());
    let _ = std::fs::remove_file(&audio_path);
    let output =
        output_result?.map_err(|error| format!("Local STT command failed to start: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("Local STT command exited with {}", output.status)
        } else {
            stderr
        });
    }

    let transcript = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if transcript.is_empty() {
        return Err("Local STT command returned no transcript".to_string());
    }
    Ok(transcript)
}

fn render_stt_command(
    command_template: &str,
    audio_path: &Path,
    mime_type: &str,
    language: Option<&str>,
    model_path: Option<&Path>,
) -> String {
    let audio = shell_quote(&audio_path.to_string_lossy());
    let mime = shell_quote(mime_type);
    let language = shell_quote(language.unwrap_or("auto"));
    let model = model_path
        .map(|path| shell_quote(&path.to_string_lossy()))
        .unwrap_or_default();
    let mut command = command_template
        .replace("{audio}", &audio)
        .replace("{mime}", &mime)
        .replace("{language}", &language)
        .replace("{model}", &model);
    if !command_template.contains("{audio}") {
        command.push(' ');
        command.push_str(&audio);
    }
    command
}

fn write_temp_audio(audio: &[u8], mime_type: &str) -> Result<PathBuf, String> {
    let extension = audio_extension_for_mime(mime_type);
    let path = env::temp_dir().join(format!("lux-stt-{}.{}", Uuid::new_v4(), extension));
    std::fs::write(&path, audio)
        .map_err(|error| format!("Failed to write recorded audio: {error}"))?;
    Ok(path)
}

fn audio_extension_for_mime(mime_type: &str) -> &'static str {
    if mime_type.contains("wav") {
        "wav"
    } else if mime_type.contains("ogg") {
        "ogg"
    } else if mime_type.contains("mp4") || mime_type.contains("m4a") {
        "m4a"
    } else {
        "webm"
    }
}

fn local_stt_shell_command(command_line: &str) -> tokio::process::Command {
    shell_command(command_line)
}

fn shell_quote(value: &str) -> String {
    #[cfg(windows)]
    {
        format!("\"{}\"", value.replace('"', "\\\""))
    }
    #[cfg(not(windows))]
    {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn non_empty_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name).and_then(|value| {
        if value.to_string_lossy().trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(value))
        }
    })
}

fn first_command_token(command: &str) -> Option<String> {
    let trimmed = command.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    let mut chars = trimmed.chars();
    let first = chars.next()?;
    if first == '"' || first == '\'' {
        let end = trimmed[1..].find(first)? + 1;
        return Some(trimmed[1..end].to_string());
    }
    Some(trimmed.split_whitespace().next()?.to_string())
}

fn command_token_available(token: &str) -> bool {
    let path = Path::new(token);
    if path.is_absolute() || token.contains('/') || token.contains('\\') {
        return executable_candidate_exists(path);
    }

    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|directory| executable_candidate_exists(&directory.join(token)))
}

fn executable_candidate_exists(path: &Path) -> bool {
    if path.is_file() {
        return true;
    }
    #[cfg(windows)]
    {
        if path.extension().is_none() {
            let extensions = env::var_os("PATHEXT")
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string());
            return extensions
                .split(';')
                .map(|extension| extension.trim().trim_start_matches('.'))
                .filter(|extension| !extension.is_empty())
                .any(|extension| path.with_extension(extension).is_file());
        }
    }
    false
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> String {
    "application state lock poisoned".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lux_core::LspSymbolKind;

    #[test]
    fn watcher_accepts_root_and_nested_workspace_paths() {
        let root = Path::new("C:/work/project");

        assert!(is_publishable_watch_path(
            root,
            Path::new("C:/work/project")
        ));
        assert!(is_publishable_watch_path(
            root,
            Path::new("C:/work/project/src/main.rs")
        ));
    }

    #[test]
    fn watcher_rejects_sibling_paths() {
        let root = Path::new("C:/work/project");

        assert!(!is_publishable_watch_path(
            root,
            Path::new("C:/work/project-old/src/main.rs")
        ));
        assert!(!is_publishable_watch_path(
            root,
            Path::new("C:/work/project2/src/main.rs")
        ));
    }

    #[test]
    fn watcher_rejects_generated_directories() {
        let root = Path::new("C:/work/project");

        assert!(!is_publishable_watch_path(
            root,
            Path::new("C:/work/project/.git/index")
        ));
        assert!(!is_publishable_watch_path(
            root,
            Path::new("C:/work/project/node_modules/pkg/index.js")
        ));
        assert!(!is_publishable_watch_path(
            root,
            Path::new("C:/work/project/target/debug/app.exe")
        ));
    }

    #[test]
    fn local_stt_command_token_supports_quoted_executable() {
        assert_eq!(
            first_command_token("\"C:/Program Files/stt/whisper-cli.exe\" -m model -f {audio}")
                .as_deref(),
            Some("C:/Program Files/stt/whisper-cli.exe")
        );
        assert_eq!(
            first_command_token("whisper-cli -m {model} -f {audio}").as_deref(),
            Some("whisper-cli")
        );
        assert_eq!(first_command_token("   "), None);
    }

    #[test]
    fn local_stt_command_rendering_appends_audio_when_placeholder_is_absent() {
        let rendered = render_stt_command(
            "whisper-cli --json",
            Path::new("C:/tmp/voice.webm"),
            "audio/webm",
            Some("ru-RU"),
            None,
        );
        assert!(rendered.contains("whisper-cli --json"));
        assert!(rendered.contains("voice.webm"));

        let rendered_with_placeholders = render_stt_command(
            "whisper-cli -m {model} -f {audio} -l {language} --mime {mime}",
            Path::new("C:/tmp/voice.webm"),
            "audio/webm",
            Some("ru-RU"),
            Some(Path::new("C:/models/ggml.bin")),
        );
        assert!(rendered_with_placeholders.contains("ggml.bin"));
        assert!(rendered_with_placeholders.contains("ru-RU"));
        assert!(rendered_with_placeholders.contains("audio/webm"));
    }

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

    #[test]
    fn patch_stats_combine_counts_all_operation_types() {
        let root = PathBuf::from("C:/work/project");
        let operations = vec![
            AiPreparedPatchOperation {
                kind: AiPreparedPatchKind::Create,
                path: root.join("a.ts"),
                after_text: Some("one\ntwo".to_string()),
                stats: AiFileOperationStats {
                    lines_added: 2,
                    lines_removed: 0,
                    files_changed: 0,
                    files_created: 1,
                    files_deleted: 0,
                },
            },
            AiPreparedPatchOperation {
                kind: AiPreparedPatchKind::Replace,
                path: root.join("b.ts"),
                after_text: Some("next".to_string()),
                stats: AiFileOperationStats {
                    lines_added: 1,
                    lines_removed: 1,
                    files_changed: 1,
                    files_created: 0,
                    files_deleted: 0,
                },
            },
            AiPreparedPatchOperation {
                kind: AiPreparedPatchKind::Delete,
                path: root.join("b.ts"),
                after_text: None,
                stats: AiFileOperationStats {
                    lines_added: 0,
                    lines_removed: 3,
                    files_changed: 0,
                    files_created: 0,
                    files_deleted: 1,
                },
            },
        ];

        let stats = combine_patch_stats(&operations);
        let paths = unique_patch_paths(&operations);

        assert_eq!(stats.lines_added, 3);
        assert_eq!(stats.lines_removed, 4);
        assert_eq!(stats.files_changed, 1);
        assert_eq!(stats.files_created, 1);
        assert_eq!(stats.files_deleted, 1);
        assert_eq!(paths, vec![root.join("a.ts"), root.join("b.ts")]);
    }

    #[test]
    fn sse_event_data_collects_multiline_data_and_ignores_comments() {
        let event = ": keep-alive\nevent: message\ndata: {\"a\":\ndata: 1}\n";

        assert_eq!(sse_event_data(event).as_deref(), Some("{\"a\":\n1}"));
    }

    #[test]
    fn ai_stream_payload_forces_stream_true() {
        let payload = serde_json::json!({ "model": "gpt-5.5", "stream": false });

        let payload = ai_stream_payload(payload);

        assert_eq!(payload.get("stream").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn web_fetch_url_guard_rejects_unsupported_schemes() {
        assert!(validate_web_fetch_url("file:///C:/Windows/win.ini").is_err());
        assert!(validate_web_fetch_url("ftp://example.com/file.txt").is_err());
        assert!(validate_web_fetch_url("https://example.com/docs").is_ok());
    }

    #[test]
    fn web_fetch_private_ip_guard_detects_local_ranges() {
        assert!(is_private_web_fetch_ip("127.0.0.1".parse().unwrap()));
        assert!(is_private_web_fetch_ip("10.0.0.12".parse().unwrap()));
        assert!(is_private_web_fetch_ip("172.16.0.1".parse().unwrap()));
        assert!(is_private_web_fetch_ip("192.168.1.20".parse().unwrap()));
        assert!(is_private_web_fetch_ip("::1".parse().unwrap()));
        assert!(!is_private_web_fetch_ip("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn web_fetch_html_to_text_removes_scripts_and_extracts_title() {
        let html = r#"<!doctype html><html><head><title>Docs &amp; API</title><style>.x{}</style><script>secret()</script></head><body><h1>Hello</h1><p>World&nbsp;now</p></body></html>"#;

        assert_eq!(extract_html_title(html).as_deref(), Some("Docs & API"));
        let text = normalize_web_fetch_text(html, Some("text/html; charset=utf-8"));

        assert!(text.contains("Hello"));
        assert!(text.contains("World now"));
        assert!(!text.contains("secret()"));
        assert!(!text.contains(".x{}"));
    }

    #[test]
    fn test_health_detects_root_workspace_before_nested_crates() {
        let root = std::env::temp_dir().join(format!("lux-test-health-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("crates/example")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/example\"]\n",
        )
        .unwrap();
        std::fs::write(
            root.join("crates/example/Cargo.toml"),
            "[package]\nname = \"example\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let plans = detect_test_health_plans(&root).unwrap();

        assert_eq!(plans.len(), 1);
        assert!(same_path(&plans[0].working_dir, &root));
        assert_eq!(plans[0].command, "cargo test --workspace");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_health_detects_nested_projects_when_root_has_no_runner() {
        let root = std::env::temp_dir().join(format!("lux-test-health-{}", Uuid::new_v4()));
        let app = root.join("apps/web");
        let api = root.join("services/api");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&api).unwrap();
        std::fs::write(
            app.join("package.json"),
            r#"{"scripts":{"test":"vitest run"}}"#,
        )
        .unwrap();
        std::fs::write(api.join("go.mod"), "module example.com/api\n").unwrap();

        let plans = detect_test_health_plans(&root).unwrap();
        let commands = plans
            .iter()
            .map(|plan| plan.command.as_str())
            .collect::<BTreeSet<_>>();

        assert_eq!(plans.len(), 2);
        assert!(commands.contains("npm test"));
        assert!(commands.contains("go test ./..."));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_health_detects_package_validation_scripts_without_test_script() {
        let root = std::env::temp_dir().join(format!("lux-test-health-{}", Uuid::new_v4()));
        let app = root.join("apps/desktop");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(root.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n").unwrap();
        std::fs::write(
            app.join("package.json"),
            r#"{"scripts":{"typecheck":"tsc --noEmit","build":"vite build"}}"#,
        )
        .unwrap();

        let plans = detect_test_health_plans(&root).unwrap();
        let commands = plans
            .iter()
            .map(|plan| (plan.kind, plan.command.as_str()))
            .collect::<BTreeSet<_>>();

        assert_eq!(plans.len(), 2);
        assert!(commands.contains(&("typecheck", "pnpm typecheck")));
        assert!(commands.contains(&("build", "pnpm build")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_health_keeps_package_validation_next_to_rust_workspace() {
        let root = std::env::temp_dir().join(format!("lux-test-health-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("crates/example")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/example\"]\n",
        )
        .unwrap();
        std::fs::write(
            root.join("crates/example/Cargo.toml"),
            "[package]\nname = \"example\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(root.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n").unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"scripts":{"typecheck":"tsc --noEmit"}}"#,
        )
        .unwrap();

        let plans = detect_test_health_plans(&root).unwrap();
        let commands = plans
            .iter()
            .map(|plan| (plan.kind, plan.command.as_str()))
            .collect::<BTreeSet<_>>();

        assert_eq!(plans.len(), 2);
        assert!(commands.contains(&("test", "cargo test --workspace")));
        assert!(commands.contains(&("typecheck", "pnpm typecheck")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_health_detects_generic_project_test_targets() {
        let root = std::env::temp_dir().join(format!("lux-test-health-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("build")).unwrap();
        std::fs::write(root.join("Makefile"), "test:\n\t./run-tests\n").unwrap();
        std::fs::write(root.join("justfile"), "test:\n    ./run-tests\n").unwrap();
        std::fs::write(
            root.join("Taskfile.yml"),
            "version: '3'\ntasks:\n  test:\n    cmds: ['echo ok']\n",
        )
        .unwrap();
        std::fs::write(
            root.join("build/CTestTestfile.cmake"),
            "# CTest generated file\n",
        )
        .unwrap();

        let plans = detect_test_health_plans(&root).unwrap();
        let commands = plans
            .iter()
            .map(|plan| plan.command.as_str())
            .collect::<BTreeSet<_>>();

        assert!(commands.contains("make test"));
        assert!(commands.contains("just test"));
        assert!(commands.contains("task test"));
        assert!(commands.contains("ctest --output-on-failure"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_health_response_aggregates_failure_before_success() {
        let root = PathBuf::from("C:/work/project");
        let runners = vec![
            TestHealthRunnerResult {
                id: ".:cargo".to_string(),
                workspace_relative_path: ".".to_string(),
                status: "passed".to_string(),
                kind: "test".to_string(),
                language: "Rust".to_string(),
                framework: "Cargo".to_string(),
                command: "cargo test".to_string(),
                exit_code: Some(0),
                duration_ms: 10,
                stdout: "ok".to_string(),
                stderr: String::new(),
                timed_out: false,
            },
            TestHealthRunnerResult {
                id: "apps/web:package".to_string(),
                workspace_relative_path: "apps/web".to_string(),
                status: "failed".to_string(),
                kind: "test".to_string(),
                language: "JavaScript/TypeScript".to_string(),
                framework: "package.json test script".to_string(),
                command: "pnpm test".to_string(),
                exit_code: Some(1),
                duration_ms: 20,
                stdout: String::new(),
                stderr: "failed".to_string(),
                timed_out: false,
            },
        ];

        let response = test_health_response_from_runners(root, runners, 0, 30);

        assert_eq!(response.status, "failed");
        assert_eq!(response.summary.total, 2);
        assert_eq!(response.summary.passed, 1);
        assert_eq!(response.summary.failed, 1);
        assert_eq!(response.exit_code, Some(1));
        assert_eq!(response.language, "Mixed");
    }

    #[test]
    fn symbol_context_filters_document_symbols_with_ancestors() {
        let symbols = vec![test_symbol(
            "App",
            LspSymbolKind::Class,
            vec![test_symbol(
                "renderToolbar",
                LspSymbolKind::Method,
                Vec::new(),
            )],
        )];

        let filtered = filter_document_symbols(&symbols, "toolbar", 10);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "App");
        assert_eq!(filtered[0].children.len(), 1);
        assert_eq!(filtered[0].children[0].name, "renderToolbar");
    }

    #[test]
    fn symbol_context_truncates_nested_document_symbols() {
        let mut symbols = vec![
            test_symbol(
                "one",
                LspSymbolKind::Function,
                vec![test_symbol("two", LspSymbolKind::Function, Vec::new())],
            ),
            test_symbol("three", LspSymbolKind::Function, Vec::new()),
        ];

        truncate_document_symbols(&mut symbols, 2);

        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "one");
        assert_eq!(symbols[0].children.len(), 1);
    }

    fn test_symbol(
        name: &str,
        kind: LspSymbolKind,
        children: Vec<LspDocumentSymbol>,
    ) -> LspDocumentSymbol {
        LspDocumentSymbol {
            name: name.to_string(),
            detail: None,
            kind,
            range: test_range(),
            selection_range: test_range(),
            children,
        }
    }

    fn test_range() -> LspRange {
        LspRange {
            start_line: 1,
            start_column: 1,
            end_line: 1,
            end_column: 1,
        }
    }
}
