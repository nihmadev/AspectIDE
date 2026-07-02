use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Stdio,
};

use chrono::Utc;
use lux_core::{
    file_view_descriptor_for_path, BufferId, DocumentSnapshot, LspDocumentSymbol, LspHover,
    LspLocation, LspSignatureHelp, LspWorkspaceSymbol, LuxEvent, WorkspaceDiagnostic,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tokio::time::{timeout, Duration};

use super::{
    emit_event, lock_error, lsp, resolve_workspace_path, resolve_workspace_path_for_write,
    resolve_workspace_path_from_root, workspace_root, SharedState,
};

const AI_SHELL_DEFAULT_TIMEOUT_SECS: u64 = 120;
const AI_SHELL_MAX_TIMEOUT_SECS: u64 = 600;
const AI_SHELL_MAX_OUTPUT_CHARS: usize = 24_000;
/// Head+tail split: first 12k and last 12k characters when truncating.
const AI_SHELL_TRUNCATE_HEAD_CHARS: usize = 12_000;
const AI_SHELL_TRUNCATE_TAIL_CHARS: usize = 12_000;
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiFileOperationStats {
    lines_added: usize,
    lines_removed: usize,
    files_changed: usize,
    files_created: usize,
    files_deleted: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiFileOperationResult {
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
pub struct AiFilePatchOperation {
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
pub struct AiShellResponse {
    workspace_root: PathBuf,
    cwd: PathBuf,
    command: String,
    exit_code: Option<i32>,
    duration_ms: u128,
    stdout: String,
    stderr: String,
    timed_out: bool,
    /// Non-fatal safety notices from `ai_shell_safety` (e.g. force-push, sudo).
    #[serde(default)]
    warnings: Vec<String>,
    /// True when the command is a known read-only inspection command.
    #[serde(default)]
    read_only: bool,
    /// True when stdout exceeded the output cap and was head+tail truncated, so the
    /// model knows output was clipped without scraping the inline marker (E18).
    #[serde(default)]
    stdout_truncated: bool,
    /// True when stderr exceeded the output cap and was head+tail truncated.
    #[serde(default)]
    stderr_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSymbolContextResponse {
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

/// F15: durable, crash-safe write for AI file operations — write a sibling temp
/// file + fsync + atomic rename instead of truncating the target in place, so a
/// disk-full error, crash, or AV lock can never leave the file empty or
/// half-written. Delegates to `lux_editor::atomic_write` on a blocking thread
/// (it does synchronous fs I/O). The `JoinError` from `spawn_blocking` and the
/// inner `AppResult` are both flattened into the tool's `String` error channel.
async fn ai_atomic_write(path: &Path, bytes: Vec<u8>) -> Result<(), String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || lux_editor::atomic_write(&path, &bytes))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// The dominant end-of-line style of `text`, from the first terminator seen:
/// `"\r\n"` (Windows), `"\r"` (classic Mac), or `"\n"` (Unix / no newline).
fn detect_eol(text: &str) -> &'static str {
    match text.find(['\n', '\r']) {
        Some(idx) if text.as_bytes()[idx] == b'\r' => {
            if text.as_bytes().get(idx + 1) == Some(&b'\n') {
                "\r\n"
            } else {
                "\r"
            }
        }
        _ => "\n",
    }
}

/// Re-encode `text`'s line endings to `eol`, first collapsing any `\r\n`/lone `\r`
/// to `\n` so the result is uniform. This lets StrReplace/PatchEngine match a file
/// saved with `\r\n` (or a classic-Mac `\r`) even though the model emits `\n`-only
/// search/replacement text — the recurring "replacement count mismatch: expected 1,
/// found 0" trap on Windows-authored files. A no-op when `text` already uses `eol`.
fn normalize_eol(text: &str, eol: &str) -> String {
    let lf = text.replace("\r\n", "\n").replace('\r', "\n");
    match eol {
        "\r\n" => lf.replace('\n', "\r\n"),
        "\r" => lf.replace('\n', "\r"),
        _ => lf,
    }
}

#[tauri::command]
pub async fn ai_file_write(
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
    let stats = diff_stats(previous_text.as_deref().unwrap_or(""), &text, !exists);
    let save_to_disk = save_to_disk.unwrap_or(true);
    if save_to_disk {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| error.to_string())?;
        }
        ai_atomic_write(&path, text.clone().into_bytes()).await?;
        // Finding 2: disk is already committed, so a notification failure must be a
        // non-fatal warning — returning Err here would make a retry double-apply.
        if let Err(e) = emit_event(&app, LuxEvent::FsChanged { path: path.clone() }) {
            tracing::warn!(%e, "ai_file_write: emit_event(FsChanged) failed (non-fatal)");
        }
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
                .open_loaded_file(&path, text)
                .map_err(String::from)?
        };
        // Finding 2: post-mutation event/LSP failures are non-fatal warnings —
        // disk state has already been committed, so returning Err would cause
        // retries to double-apply on a correct disk payload.
        if let Err(e) = emit_event(
            &app,
            LuxEvent::EditorDocumentChanged {
                document: document.clone(),
            },
        ) {
            tracing::warn!(%e, "ai_file_write: emit_event failed (non-fatal)");
        }
        if exists {
            let _ = lsp::forward_document_update(&app, &state, &document).await;
        } else {
            let _ = lsp::forward_document_open(&app, &state, &document).await;
        }
        Some(document)
    } else {
        let mut newly_opened = false;
        let existing = {
            let mut documents = state.documents.lock().map_err(lock_error)?;
            let existing = documents.snapshot_for_path(&path).map_err(String::from)?;
            if let Some(document) = existing {
                Some(
                    documents
                        .update_text(document.id, text)
                        .map_err(String::from)?,
                )
            } else {
                // Staged (save_to_disk=false) write to a not-yet-open path: open an
                // in-memory doc and dirty-mark it so the new content is preserved
                // instead of being silently discarded while we report "file created".
                newly_opened = true;
                let opened = documents
                    .open_loaded_file(&path, String::new())
                    .map_err(String::from)?;
                Some(
                    documents
                        .update_text(opened.id, text)
                        .map_err(String::from)?,
                )
            }
        };
        if let Some(document) = &existing {
            // Finding 2: non-fatal warnings on post-mutation notification failure.
            if let Err(e) = emit_event(
                &app,
                LuxEvent::EditorDocumentChanged {
                    document: document.clone(),
                },
            ) {
                tracing::warn!(%e, "ai_file_write(staged): emit_event failed (non-fatal)");
            }
            // A never-before-opened staged doc needs a didOpen first; sending
            // didChange (forward_document_update) for an unknown document would
            // be dropped or rejected by the language server. Mirror ai_file_patch.
            if newly_opened {
                let _ = lsp::forward_document_open(&app, &state, document).await;
            } else {
                let _ = lsp::forward_document_update(&app, &state, document).await;
            }
        }
        existing
    };

    Ok(AiFileOperationResult {
        operation: "write".to_string(),
        path: path.clone(),
        saved_to_disk: save_to_disk,
        changed_paths: vec![path],
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
pub async fn ai_file_str_replace(
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
    // Finding 1: use write-path resolution for mutating operations (same as Write/Create).
    let path = resolve_workspace_path_for_write(&state, &path)?;
    let before = current_text_for_path(&state, &path).await?;
    // CRLF tolerance: re-encode the model's (usually `\n`-only) search + replacement
    // text to the file's own EOL so a match isn't silently missed on a `\r\n` file.
    let eol = detect_eol(&before);
    let old_text = normalize_eol(&old_text, eol);
    let new_text = normalize_eol(&new_text, eol);
    let replacement_count = before.matches(&old_text).count();
    let expected = expected_replacements.unwrap_or(1);
    if replacement_count != expected {
        let new_already = before.matches(&new_text).count();
        // Idempotency aid: a repeated "insert around" edit (newText contains oldText)
        // that was already applied leaves 0 matches of oldText but the newText already
        // present. Report a no-op success so re-issuing the same StrReplace can't
        // compound duplicate insertions, rather than a confusing count mismatch.
        if replacement_count == 0
            && !new_text.is_empty()
            && new_text.contains(&old_text)
            && new_already >= expected
        {
            return Ok(AiFileOperationResult {
                operation: "strReplace".to_string(),
                path: path.clone(),
                saved_to_disk: false,
                changed_paths: Vec::new(),
                edited_documents: Vec::new(),
                stats: AiFileOperationStats::default(),
                message: format!(
                    "no change: newText already present {new_already} time(s) — edit appears already applied"
                ),
            });
        }
        // E10: give the model an in-band remedy instead of a bare count.
        let remedy = if replacement_count > expected {
            format!(" — oldText matched {replacement_count} places; pass expectedReplacements:{replacement_count} to replace all, or add surrounding lines to oldText to target one")
        } else if replacement_count == 0 && new_already > 0 && !new_text.is_empty() {
            format!(
                " — newText is already present {new_already} time(s); it may already be applied"
            )
        } else if replacement_count == 0 {
            " — oldText not found; check exact whitespace/indentation (matched literally, though CRLF/LF differences are tolerated)".to_string()
        } else {
            String::new()
        };
        return Err(format!(
            "replacement count mismatch for {}: expected {expected}, found {replacement_count}{remedy}",
            path.display()
        ));
    }
    let after = before.replacen(&old_text, &new_text, expected);
    let stats = diff_stats(&before, &after, false);
    let save_to_disk = save_to_disk.unwrap_or(true);
    if save_to_disk {
        ai_atomic_write(&path, after.clone().into_bytes()).await?;
        // Finding 2: non-fatal on notification failure (disk is already committed).
        if let Err(e) = emit_event(&app, LuxEvent::FsChanged { path: path.clone() }) {
            tracing::warn!(%e, "ai_file_str_replace: emit_event failed (non-fatal)");
        }
    }

    let edited_document =
        update_open_document_after_text_change(&app, &state, &path, after, !save_to_disk).await?;
    Ok(AiFileOperationResult {
        operation: "strReplace".to_string(),
        path: path.clone(),
        saved_to_disk: save_to_disk,
        changed_paths: vec![path],
        edited_documents: edited_document.into_iter().collect(),
        stats,
        message: format!("replaced {replacement_count} occurrence(s)"),
    })
}

#[tauri::command]
pub async fn ai_file_patch(
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
                                .open_loaded_file(&operation.path, after_text)
                                .map_err(String::from)?,
                            true,
                        ),
                        // Staged (save_to_disk=false) Create/Rewrite on a not-yet-open
                        // path: open an in-memory doc and dirty-mark it so the content
                        // takes effect instead of being skipped while reported applied.
                        None => {
                            let opened = documents
                                .open_loaded_file(&operation.path, String::new())
                                .map_err(String::from)?;
                            (
                                documents
                                    .update_text(opened.id, after_text)
                                    .map_err(String::from)?,
                                true,
                            )
                        }
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
        // Finding 2: non-fatal warnings on post-mutation notification failure.
        if let Err(e) = emit_event(
            &app,
            LuxEvent::EditorDocumentClosed {
                document: document.clone(),
            },
        ) {
            tracing::warn!(%e, "ai_file_patch: emit_event(closed) failed (non-fatal)");
        }
        if let Some(path) = &document.path {
            let _ = lsp::forward_document_close(&app, &state, path).await;
        }
    }
    for (document, is_new_document) in &document_events {
        if let Err(e) = emit_event(
            &app,
            LuxEvent::EditorDocumentChanged {
                document: document.clone(),
            },
        ) {
            tracing::warn!(%e, "ai_file_patch: emit_event(changed) failed (non-fatal)");
        }
        if *is_new_document {
            let _ = lsp::forward_document_open(&app, &state, document).await;
        } else {
            let _ = lsp::forward_document_update(&app, &state, document).await;
        }
    }
    for path in &changed_paths {
        if save_to_disk {
            if let Err(e) = emit_event(&app, LuxEvent::FsChanged { path: path.clone() }) {
                tracing::warn!(%e, "ai_file_patch: emit_event(FsChanged) failed (non-fatal)");
            }
        }
        if prepared.iter().any(|operation| {
            operation.path == *path && operation.kind == AiPreparedPatchKind::Delete
        }) {
            let _ = lsp::apply_diagnostics_update(
                &app,
                state.inner(),
                lux_lsp::DiagnosticsUpdate {
                    path: path.clone(),
                    diagnostics: Vec::new(),
                },
            );
        }
    }

    let operation_count = prepared.len();
    let path_count = changed_paths.len();
    Ok(AiFileOperationResult {
        operation: "patch".to_string(),
        path: changed_paths.first().cloned().unwrap_or_default(),
        saved_to_disk: save_to_disk,
        changed_paths,
        edited_documents,
        stats,
        message: format!("patch applied: {operation_count} operation(s), {path_count} path(s)"),
    })
}

#[tauri::command]
pub async fn ai_file_delete(
    app: AppHandle,
    state: State<'_, SharedState>,
    path: PathBuf,
) -> Result<AiFileOperationResult, String> {
    // Finding 1 + 8: use write-path resolution; refuse directories (file-only contract).
    let path = resolve_workspace_path_for_write(&state, &path)?;
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|error| error.to_string())?;
    if metadata.is_dir() {
        return Err(format!(
            "Delete tool only removes files, not directories: {}. \
             Use the integrated terminal for recursive directory deletion.",
            path.display()
        ));
    }
    let previous_text = tokio::fs::read_to_string(&path).await.unwrap_or_default();
    tokio::fs::remove_file(&path)
        .await
        .map_err(|error| error.to_string())?;
    let stats = AiFileOperationStats {
        lines_added: 0,
        lines_removed: previous_text.lines().count(),
        files_changed: 0,
        files_created: 0,
        files_deleted: 1,
    };
    let closed_documents = {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        documents
            .close_path(&path)
            .map_err(String::from)?
            .into_iter()
            .collect::<Vec<_>>()
    };
    // Finding 2: disk is already committed; notification failures are non-fatal.
    for document in &closed_documents {
        if let Err(e) = emit_event(
            &app,
            LuxEvent::EditorDocumentClosed {
                document: document.clone(),
            },
        ) {
            tracing::warn!(%e, "ai_file_delete: emit_event(closed) failed (non-fatal)");
        }
    }
    let _ = lsp::forward_document_close(&app, &state, &path).await;
    let _ = lsp::apply_diagnostics_update(
        &app,
        state.inner(),
        lux_lsp::DiagnosticsUpdate {
            path: path.clone(),
            diagnostics: Vec::new(),
        },
    );
    if let Err(e) = emit_event(&app, LuxEvent::FsChanged { path: path.clone() }) {
        tracing::warn!(%e, "ai_file_delete: emit_event(FsChanged) failed (non-fatal)");
    }
    Ok(AiFileOperationResult {
        operation: "delete".to_string(),
        path: path.clone(),
        saved_to_disk: true,
        changed_paths: vec![path],
        edited_documents: Vec::new(),
        stats,
        message: "file deleted".to_string(),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiShellClassification {
    /// Catastrophic reason when the command would be refused by `ai_shell`.
    blocked: Option<String>,
    /// Non-fatal risk notices.
    warnings: Vec<String>,
    /// True when the command is a known read-only inspection command.
    read_only: bool,
}

/// Classify a shell command's safety without executing it. Lets the TypeScript
/// approval gate auto-approve read-only inspections and reject catastrophic
/// commands before prompting, while the authoritative checks stay in Rust.
#[tauri::command]
pub fn ai_shell_classify(command: String) -> AiShellClassification {
    let report = crate::ai_shell_safety::classify_shell_command(command.trim());
    AiShellClassification {
        blocked: report.blocked,
        warnings: report.warnings,
        read_only: report.read_only,
    }
}

#[tauri::command]
pub async fn ai_shell(
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

    // Safety boundary: refuse catastrophic commands outright; surface risk notices.
    let safety = crate::ai_shell_safety::classify_shell_command(&command);
    if let Some(reason) = safety.blocked {
        return Err(format!(
            "Lux blocked this command for safety ({reason}). If this is genuinely intended, run it manually in the integrated terminal."
        ));
    }

    let timeout_secs = timeout_secs
        .unwrap_or(AI_SHELL_DEFAULT_TIMEOUT_SECS)
        .clamp(1, AI_SHELL_MAX_TIMEOUT_SECS);

    let started = std::time::Instant::now();
    let mut process = shell_command(&command);
    // Strip any verbatim `\\?\` prefix: cmd.exe refuses verbatim/UNC working
    // directories and silently falls back to C:\Windows, so the shell would run
    // in the wrong place (the recurring "cmd stuck in C:\Windows" bug).
    process.current_dir(dunce::simplified(&cwd));
    process.stdin(Stdio::null());
    process.stdout(Stdio::piped());
    process.stderr(Stdio::piped());
    // Backstop only: if this future is dropped (cancel/panic) the immediate shell
    // child is killed. On timeout we additionally tree-kill below, because
    // kill_on_drop issues a single-PID TerminateProcess/SIGKILL that leaves any
    // grandchildren spawned by `cmd /C` / `sh -c` running orphaned.
    process.kill_on_drop(true);
    // Make the child its own process-group leader (pgid == child pid) so the
    // timeout group-kill on Unix takes down the whole subtree at once.
    #[cfg(unix)]
    process.process_group(0);

    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => return Err(format!("Failed to start shell command: {error}")),
    };
    let child_pid = child.id();
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();

    // Finding 5: keep bounded stdout/stderr buffers outside the timed-out future so
    // partial output is returned on timeout instead of being discarded. The collect
    // future fills them; on timeout we read what arrived so far via the Arc.
    let shared_stdout: std::sync::Arc<tokio::sync::Mutex<Vec<u8>>> =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let shared_stderr: std::sync::Arc<tokio::sync::Mutex<Vec<u8>>> =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));

    // Own the child explicitly (rather than `process.output()`, which hides the
    // PID): drain both pipes concurrently to avoid a full-buffer deadlock, then
    // reap. Borrowing `child` here means a timeout drops only this future's
    // borrow, leaving the child alive so the tree-kill below can target it.
    let collect_stdout = std::sync::Arc::clone(&shared_stdout);
    let collect_stderr = std::sync::Arc::clone(&shared_stderr);
    let collect = async {
        use tokio::io::AsyncReadExt;
        // Cap each pipe so a runaway producer (`yes`, `cat /dev/urandom`) can't
        // balloon memory before the timeout fires — the output is truncated for
        // display anyway. Bounded read here, not "read everything then cap".
        const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;
        // After the command's own process exits, a backgrounded grandchild it
        // spawned (`cmd /C foo &`, `serve &`, a daemon) can keep the stdout/stderr
        // write end open, so `read_to_end` would block on an EOF that never comes —
        // making the tool wait the FULL timeout for a command that already finished.
        // So we reap the child first, then give the pipes only a short grace window
        // to flush trailing output before returning. The real command's output is
        // already captured; a lingering daemon must not hold the turn hostage.
        const PIPE_DRAIN_GRACE_SECS: u64 = 2;
        // Finding 5: read each pipe in chunks and append into the SHARED buffers as
        // bytes arrive, rather than into a local buffer that is only published after
        // EOF. A `read_to_end` that publishes on completion loses everything when the
        // outer timeout fires mid-read, so the timeout branch would still see empty
        // output. Streaming into the shared (mutex-guarded) buffer means whatever was
        // captured before the timeout is preserved.
        async fn stream_into(
            pipe: Option<&mut tokio::process::ChildStdout>,
            sink: &std::sync::Arc<tokio::sync::Mutex<Vec<u8>>>,
            cap: usize,
        ) {
            let Some(pipe) = pipe else { return };
            let mut chunk = [0u8; 16 * 1024];
            loop {
                // Finding 4: keep draining to EOF so the writer never blocks on a full
                // pipe (which would stall a producer that out-runs the cap and make the
                // command appear hung until the timeout fires). Only the APPEND is
                // gated on `buffer.len() < cap`; reads continue and are discarded once
                // the 8 MiB bound is reached.
                match pipe.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut buffer = sink.lock().await;
                        if buffer.len() < cap {
                            let room = cap - buffer.len();
                            buffer.extend_from_slice(&chunk[..n.min(room)]);
                        }
                    }
                }
            }
        }
        // `ChildStdout`/`ChildStderr` are distinct types, so read stderr inline with
        // the same chunked-append logic instead of reusing the stdout-typed helper.
        let read_stdout = stream_into(stdout_pipe.as_mut(), &collect_stdout, MAX_CAPTURE_BYTES);
        let read_stderr = async {
            let Some(pipe) = stderr_pipe.as_mut() else {
                return;
            };
            let mut chunk = [0u8; 16 * 1024];
            loop {
                // Finding 4: drain to EOF; gate only the append on the cap (see
                // `stream_into`). Reading past the cap keeps the pipe from filling and
                // blocking the child's stderr writer.
                match pipe.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut buffer = collect_stderr.lock().await;
                        if buffer.len() < MAX_CAPTURE_BYTES {
                            let room = MAX_CAPTURE_BYTES - buffer.len();
                            buffer.extend_from_slice(&chunk[..n.min(room)]);
                        }
                    }
                }
            }
        };
        let drain = async { tokio::join!(read_stdout, read_stderr) };
        let mut drain = Box::pin(drain);
        // Race the pipe drain against the child exiting. If the pipes hit EOF first
        // (normal case), `status` is taken right after. If the child exits while a
        // grandchild still holds a pipe, the drain stalls — cap it at the grace
        // window so we return promptly instead of stalling to the outer timeout.
        let status = tokio::select! {
            _ = drain.as_mut() => child.wait().await,
            status = child.wait() => {
                let _ = timeout(Duration::from_secs(PIPE_DRAIN_GRACE_SECS), drain.as_mut()).await;
                status
            }
        };
        status
    };

    let output_result = timeout(Duration::from_secs(timeout_secs), collect).await;
    let duration_ms = started.elapsed().as_millis();

    match output_result {
        Ok(Ok(status)) => {
            // The `collect` future (and its buffer clones) is dropped once `timeout`
            // resolves, so read the captured bytes back through the shared handles.
            let stdout_buf = shared_stdout.lock().await.clone();
            let stderr_buf = shared_stderr.lock().await.clone();
            let (stdout, stdout_truncated) =
                truncate_shell_output_flagged(&String::from_utf8_lossy(&stdout_buf));
            let (stderr, stderr_truncated) =
                truncate_shell_output_flagged(&String::from_utf8_lossy(&stderr_buf));
            Ok(AiShellResponse {
                workspace_root: root,
                cwd,
                command,
                exit_code: status.code(),
                duration_ms,
                stdout,
                stderr,
                timed_out: false,
                warnings: safety.warnings,
                read_only: safety.read_only,
                stdout_truncated,
                stderr_truncated,
            })
        }
        Ok(Err(error)) => Err(format!("Failed to run shell command: {error}")),
        Err(_) => {
            // Timed out: `child` is still alive (only the borrow held by `collect`
            // was dropped). Kill the whole process tree before returning so no
            // grandchild keeps running orphaned; start_kill backstops the shell.
            kill_process_tree(child_pid).await;
            let _ = child.start_kill();
            // Finding 5: preserve partial stdout/stderr captured before timeout.
            let partial_stdout = {
                let buf = shared_stdout.lock().await;
                String::from_utf8_lossy(&buf).to_string()
            };
            let partial_stderr = {
                let buf = shared_stderr.lock().await;
                String::from_utf8_lossy(&buf).to_string()
            };
            let (stdout, stdout_truncated) = truncate_shell_output_flagged(&partial_stdout);
            let (stderr_body, stderr_truncated) = truncate_shell_output_flagged(&partial_stderr);
            Ok(AiShellResponse {
                workspace_root: root,
                cwd,
                command,
                exit_code: None,
                duration_ms,
                stdout,
                stderr: if partial_stderr.is_empty() {
                    format!("Shell command timed out after {timeout_secs} seconds")
                } else {
                    format!(
                        "{stderr_body}\n---\nShell command timed out after {timeout_secs} seconds"
                    )
                },
                timed_out: true,
                warnings: safety.warnings,
                read_only: safety.read_only,
                stdout_truncated,
                stderr_truncated,
            })
        }
    }
}

/// Best-effort kill of a timed-out shell command's entire process tree.
/// `kill_on_drop` only terminates the immediate `cmd.exe`/`sh` child, so any
/// grandchildren it spawned would otherwise keep running orphaned.
pub async fn kill_process_tree(pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    #[cfg(windows)]
    {
        let pid_str = pid.to_string();
        let mut command = tokio::process::Command::new("taskkill");
        command
            .args(["/T", "/F", "/PID", &pid_str])
            .creation_flags(CREATE_NO_WINDOW);
        let _ = command.output().await;
    }
    #[cfg(not(windows))]
    {
        // The child is its own process-group leader (process_group(0)), so its
        // pgid equals its pid; the leading '-' targets the whole group.
        let _ = tokio::process::Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{pid}"))
            .output()
            .await;
    }
}

/// Read a text file — public wrapper for the turn-loop dispatcher.
pub async fn ai_read_file(
    state: State<'_, SharedState>,
    path: PathBuf,
    max_bytes: Option<u64>,
    start_line: Option<u32>,
    max_lines: Option<u32>,
) -> Result<AiReadFileResult, String> {
    // Hard upper ceiling so a caller/model can't request an arbitrarily large
    // read (e.g. `maxBytes: 9_999_999_999`) and balloon memory; the default
    // stays small and the request is clamped into [1, 10 MiB].
    const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;
    // A line-window read (startLine/maxLines) may need to reach deep into a big
    // file, so give it a roomier default byte budget than a plain head read when
    // the caller didn't pin `maxBytes` explicitly — lets the model page a 2000-line
    // file instead of re-reading a truncated head over and over.
    let windowed = start_line.is_some() || max_lines.is_some();
    let default_bytes: u64 = if windowed { 2 * 1024 * 1024 } else { 120_000 };
    let max_bytes = max_bytes.unwrap_or(default_bytes).clamp(1, MAX_READ_BYTES);
    let path = resolve_workspace_path(&state, &path)?;
    tokio::task::spawn_blocking(move || -> Result<AiReadFileResult, String> {
        use std::io::{BufRead, BufReader, Read};
        let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("path is not a file".to_string());
        }
        let size = metadata.len();

        // Line-window read (startLine/maxLines): stream the file line-by-line so a
        // deep startLine is reachable regardless of byte size, count EVERY line for
        // an exact total_lines, and bound only the COLLECTED output so we never OOM.
        // (A byte-capped slice would under-report total_lines and make lines past the
        // cap unreachable — silently defeating the paging contract this promises.)
        if windowed {
            let start = start_line.map_or(1usize, |s| usize::try_from(s.max(1)).unwrap_or(1));
            let take = max_lines.map_or(usize::MAX, |c| usize::try_from(c).unwrap_or(usize::MAX));
            let out_cap = usize::try_from(max_bytes).unwrap_or(usize::MAX);
            let mut reader = BufReader::new(std::fs::File::open(&path).map_err(|e| e.to_string())?);
            let mut line_buf: Vec<u8> = Vec::new();
            let mut total_lines = 0usize;
            let mut text = String::new();
            let mut truncated = false;
            loop {
                line_buf.clear();
                let read = reader
                    .read_until(b'\n', &mut line_buf)
                    .map_err(|e| e.to_string())?;
                if read == 0 {
                    break;
                }
                total_lines += 1;
                if total_lines >= start && total_lines - start < take {
                    // Trim the line terminator (`\n` + an optional preceding `\r`) to
                    // match `str::lines()` semantics.
                    let mut end = line_buf.len();
                    if end > 0 && line_buf[end - 1] == b'\n' {
                        end -= 1;
                    }
                    if end > 0 && line_buf[end - 1] == b'\r' {
                        end -= 1;
                    }
                    let line = String::from_utf8_lossy(&line_buf[..end]);
                    // Bound collected output; keep counting lines for an exact total.
                    if text.len() + line.len() + 1 > out_cap {
                        truncated = true;
                    } else {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(&line);
                    }
                }
            }
            return Ok(AiReadFileResult {
                path,
                text,
                truncated,
                size,
                total_lines,
                start_line: Some(start),
            });
        }

        // Non-windowed head read: bounded byte read + UTF-8 boundary trim.
        let limit = max_bytes.min(size);
        let mut buffer = Vec::new();
        std::fs::File::open(&path)
            .map_err(|e| e.to_string())?
            .take(limit)
            .read_to_end(&mut buffer)
            .map_err(|e| e.to_string())?;
        // Finding 20: a byte cap can slice through the middle of a multi-byte UTF-8
        // codepoint, which `from_utf8_lossy` would render as a spurious replacement
        // char (U+FFFD) at the very end of an otherwise-valid file. Only when the read
        // was actually truncated, drop a single trailing *incomplete* sequence
        // (`error_len().is_none()` == "needs more bytes") so the visible content ends
        // cleanly. A genuinely invalid byte (`error_len() == Some(_)`) is left in place
        // and still falls through to the lossy conversion below.
        if limit < size {
            if let Err(error) = std::str::from_utf8(&buffer) {
                if error.error_len().is_none() {
                    let valid = error.valid_up_to();
                    buffer.truncate(valid);
                }
            }
        }
        let text = String::from_utf8_lossy(&buffer).into_owned();
        // E8: when the head read was truncated, `text` holds only the first `limit`
        // bytes, so counting its lines would UNDER-report the whole-file total the
        // Read contract promises for paging. Stream the remainder counting lines so
        // total_lines is exact (cheap `read_until` loop; no full in-memory load).
        let total_lines = if limit < size {
            let mut reader = BufReader::new(std::fs::File::open(&path).map_err(|e| e.to_string())?);
            let mut scratch: Vec<u8> = Vec::new();
            let mut count = 0usize;
            loop {
                scratch.clear();
                let read = reader
                    .read_until(b'\n', &mut scratch)
                    .map_err(|e| e.to_string())?;
                if read == 0 {
                    break;
                }
                count += 1;
            }
            count
        } else {
            text.lines().count()
        };
        Ok(AiReadFileResult {
            path,
            text,
            truncated: size > max_bytes,
            size,
            total_lines,
            start_line: None,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiReadFileResult {
    pub path: PathBuf,
    pub text: String,
    pub truncated: bool,
    pub size: u64,
    /// Total line count of the file before any line-window slicing, so the caller
    /// can page through it with `startLine`/`maxLines`.
    pub total_lines: usize,
    /// 1-based first line of the returned window, present only when a line range
    /// was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
}

/// List workspace files matching a pattern — public wrapper for the turn-loop.
pub async fn ai_glob(
    state: State<'_, SharedState>,
    pattern: String,
    max_results: Option<usize>,
) -> Result<AiGlobResult, String> {
    let root = workspace_root(&state)?;
    let max = max_results.unwrap_or(80).clamp(1, 500);
    let pattern = pattern.trim().replace('\\', "/");
    if pattern.is_empty() {
        return Ok(AiGlobResult {
            pattern,
            count: 0,
            files: Vec::new(),
            truncated: false,
        });
    }

    // Build a matcher that understands glob wildcards (`*`, `**`, `?`, `[...]`,
    // `{a,b}`) when present, and falls back to a case-insensitive substring match
    // for plain text (so `foo.ts` and `src/foo` keep working). A bare pattern like
    // `*.ts` previously matched nothing because it was compared as a literal
    // substring of the path — the documented Glob bug. Glob matching runs against
    // the path RELATIVE to the workspace root so `*.ts` and `**/*.tsx` anchor the
    // way users expect. The walk stops once `max` files match (no full-tree
    // materialization, no misleading pre-filter cap).
    let has_glob_meta = pattern.contains(['*', '?', '[', ']', '{', '}']);
    let glob_matcher = if has_glob_meta {
        // Bare `*.ts` should match at any depth, so also accept a `**/` prefix form.
        let mut builder = globset::GlobSetBuilder::new();
        let add = |b: &mut globset::GlobSetBuilder, raw: &str| -> Result<(), String> {
            let glob = globset::GlobBuilder::new(raw)
                .case_insensitive(true)
                .literal_separator(false)
                .build()
                .map_err(|e| format!("Invalid glob pattern `{raw}`: {e}"))?;
            b.add(glob);
            Ok(())
        };
        add(&mut builder, &pattern)?;
        if !pattern.contains('/') {
            add(&mut builder, &format!("**/{pattern}"))?;
        }
        Some(
            builder
                .build()
                .map_err(|e| format!("Invalid glob pattern: {e}"))?,
        )
    } else {
        None
    };

    let root_for_walk = root.clone();
    let substring_pattern = pattern.to_lowercase();
    let files: Vec<PathBuf> = tokio::task::spawn_blocking(move || {
        lux_fs::list_files_matching(
            root_for_walk.clone(),
            move |path| {
                // Compare against the workspace-relative path (forward slashes) so
                // glob anchoring matches user intent; fall back to the absolute path
                // if stripping fails (path outside root — shouldn't happen here).
                let relative = path
                    .strip_prefix(&root_for_walk)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', "/");
                glob_matcher.as_ref().map_or_else(
                    || relative.to_lowercase().contains(&substring_pattern),
                    |matcher| matcher.is_match(relative.as_str()),
                )
            },
            max,
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?
    .into_iter()
    .map(|e| e.path)
    .collect();
    Ok(AiGlobResult {
        count: files.len(),
        truncated: files.len() >= max,
        files,
        pattern,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiGlobResult {
    pub pattern: String,
    pub count: usize,
    pub files: Vec<PathBuf>,
    /// True when the match count hit `maxResults`, so the listing may be clipped
    /// and the model should narrow the pattern rather than assume it is complete.
    #[serde(default)]
    pub truncated: bool,
}

#[tauri::command]
pub async fn ai_symbol_context(
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
        let total = workspace_symbols.len();
        workspace_symbols.truncate(max_results);
        if total > workspace_symbols.len() {
            notes.push(format!(
                "workspace symbols truncated to {} of {total}; raise maxResults to see more",
                workspace_symbols.len()
            ));
        }
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
        let total = count_document_symbols(&document_symbols);
        truncate_document_symbols(&mut document_symbols, max_results);
        let shown = count_document_symbols(&document_symbols);
        if total > shown {
            notes.push(format!(
                "document symbols truncated to {shown} of {total}; raise maxResults to see more"
            ));
        }

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
            let definitions_total = definitions.len();
            let references_total = references.len();
            definitions.truncate(max_results);
            references.truncate(max_results);
            if definitions_total > definitions.len() {
                notes.push(format!(
                    "definitions truncated to {} of {definitions_total}; raise maxResults to see more",
                    definitions.len()
                ));
            }
            if references_total > references.len() {
                notes.push(format!(
                    "references truncated to {} of {references_total}; raise maxResults to see more",
                    references.len()
                ));
            }
        } else if !query.is_empty() {
            document_symbols = filter_document_symbols(&document_symbols, &query, max_results);
        }

        // Never return a bare empty result: name the likely reason so the model
        // (and the user) can tell "no symbols" from "server not ready/running".
        if document_symbols.is_empty() {
            notes.push("no document symbols returned for this file; the language server may still be indexing, may not be running for this language (check Settings → Language Servers), or may not support document symbols".to_string());
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
            AiPreparedPatchKind::Create
            | AiPreparedPatchKind::Rewrite
            | AiPreparedPatchKind::Replace
            | AiPreparedPatchKind::Delete => {
                // Finding 1: route ALL mutating operations through the write resolver.
                resolve_workspace_path_for_write(state, &operation.path)?
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
                // `rewrite` replaces an EXISTING file's contents; it must not be a
                // back door for silently creating a file (that is `create`'s job,
                // which is gated by the overwrite check above). Requiring existence
                // keeps the create/rewrite contract explicit; the turn-loop's
                // read-before-edit guard already prevents clobbering unread content.
                if kind == AiPreparedPatchKind::Rewrite && before_text.is_none() {
                    return Err(format!(
                        "rewrite target does not exist (use create): {}",
                        path.display()
                    ));
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
                // F18: an omitted `newText` is a caller error, not a silent
                // delete-to-empty. `ai_file_str_replace` takes `new_text: String`
                // (non-optional), so the patch path must be equally strict; a
                // deliberate deletion still works by passing an explicit empty string.
                let new_text = operation
                    .new_text
                    .ok_or_else(|| format!("replace requires newText for {}", path.display()))?;
                let expected = operation.expected_replacements.unwrap_or(1);
                // CRLF tolerance (parity with `ai_file_str_replace`): match against the
                // file's own EOL so a `\n`-only patch still applies to a `\r\n` file.
                let eol = detect_eol(&before);
                let old_text = normalize_eol(&old_text, eol);
                let new_text = normalize_eol(&new_text, eol);
                let replacement_count = before.matches(&old_text).count();
                if replacement_count != expected {
                    let new_already = before.matches(&new_text).count();
                    let hint = if new_already > 0 && !new_text.is_empty() {
                        format!(" (newText already present {new_already} time(s) — may already be applied)")
                    } else {
                        String::new()
                    };
                    return Err(format!(
                        "replacement count mismatch for {}: expected {expected}, found {replacement_count}{hint}",
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
                ai_atomic_write(&operation.path, text.as_bytes().to_vec()).await?;
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
                // F15: restore previous content durably (sibling temp + atomic
                // rename) just like the forward write, so a crash mid-rollback can't
                // leave a half-written file. Best-effort: rollback is itself the
                // error path, so a failure here is logged-by-absence, not propagated.
                let _ = ai_atomic_write(&entry.path, bytes).await;
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
            .map_or_else(|| path.to_string_lossy().into_owned(), ToOwned::to_owned),
        language_id: lux_editor::language_id_for_path(path),
        text,
        view: file_view_descriptor_for_path(path),
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

/// Count document symbols the same way the budget is spent (one per node,
/// recursing into children), so a truncation note's reported total matches the
/// truncation accounting rather than only the top-level count (A22).
fn count_document_symbols(symbols: &[LspDocumentSymbol]) -> usize {
    symbols
        .iter()
        .map(|symbol| 1 + count_document_symbols(&symbol.children))
        .sum()
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
    let matches = symbol.name.to_ascii_lowercase().contains(needle)
        || symbol
            .detail
            .as_deref()
            .is_some_and(|detail| detail.to_ascii_lowercase().contains(needle));
    // Reserve this node's slot up-front so matched children cannot consume the
    // budget that retains their parent (which would discard the parent and the
    // already-matched descendants, and starve later siblings).
    *remaining -= 1;
    let children = symbol
        .children
        .iter()
        .filter_map(|child| filter_symbol_with_budget(child, needle, remaining))
        .collect::<Vec<_>>();
    if !matches && children.is_empty() {
        // Node is not emitted; return its reserved slot to the pool.
        *remaining += 1;
        return None;
    }
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
    let (updated, newly_opened) = {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        let existing = documents
            .replace_text_for_path(path, text.clone(), dirty)
            .map_err(String::from)?;
        if let Some(document) = existing {
            (Some(document), false)
        } else if dirty {
            // Finding 7: staged (save_to_disk=false) operation on a not-yet-open
            // path: open an in-memory doc so the edit is visible in the editor
            // instead of being silently discarded while we report success.
            let opened = documents
                .open_loaded_file(path, String::new())
                .map_err(String::from)?;
            let updated = documents
                .update_text(opened.id, text)
                .map_err(String::from)?;
            (Some(updated), true)
        } else {
            (None, false)
        }
    };
    if let Some(document) = &updated {
        // Finding 2: non-fatal warnings on post-mutation notification failure.
        if let Err(e) = emit_event(
            app,
            LuxEvent::EditorDocumentChanged {
                document: document.clone(),
            },
        ) {
            tracing::warn!(%e, "update_open_document: emit_event failed (non-fatal)");
        }
        if newly_opened {
            let _ = lsp::forward_document_open(app, state, document).await;
        } else {
            let _ = lsp::forward_document_update(app, state, document).await;
        }
    }
    Ok(updated)
}

/// Added / removed / changed line counts between two file contents.
///
/// F23: counts via a longest-common-subsequence of lines rather than the net
/// line-count delta. The old `after.lines().count() - before.lines().count()`
/// reported 0 added / 0 removed for an N-for-N content replacement (same line
/// count, different lines), so any in-place content edit looked like a no-op in
/// the stats. LCS makes `lines_added` / `lines_removed` reflect the lines that
/// actually changed. (A trailing-newline-only edit still shows only in
/// `files_changed`, since `lines()` discards a trailing empty line.)
fn diff_stats(before: &str, after: &str, created: bool) -> AiFileOperationStats {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let common = lcs_len(&before_lines, &after_lines);
    AiFileOperationStats {
        lines_added: after_lines.len() - common,
        lines_removed: before_lines.len() - common,
        files_changed: usize::from(!created && before != after),
        files_created: usize::from(created),
        files_deleted: 0,
    }
}

/// Length of the longest common subsequence of two line slices, via a rolling
/// two-row DP in O(min(m, n)) space (the shorter slice drives the inner row).
/// Returns 0 when either slice is empty.
fn lcs_len(a: &[&str], b: &[&str]) -> usize {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if short.is_empty() {
        return 0;
    }
    let mut prev = vec![0usize; short.len() + 1];
    let mut curr = vec![0usize; short.len() + 1];
    for &long_line in long {
        for (j, &short_line) in short.iter().enumerate() {
            curr[j + 1] = if long_line == short_line {
                prev[j] + 1
            } else {
                curr[j].max(prev[j + 1])
            };
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[short.len()]
}

fn shell_command(command_line: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let mut command = tokio::process::Command::new("cmd");
        // BUG(F1): cmd /C parses the rest of the line with its own verbatim-quote
        // rule — `cmd /C "<command_line>"` strips ONLY the outer wrapping quotes and
        // passes everything between them through untouched. Building the line via two
        // `.arg()` calls instead routed it through the MSVCRT argv quoter, which
        // backslash-escapes inner quotes (`"` -> `\"`), so `python -c "print('x')"`
        // reached the interpreter mangled. Emit the `/C "<line>"` form verbatim with
        // raw_arg so inner quotes survive (empirically verified:
        // `cmd /C "python -c "print(2+2)""` prints 4). raw_arg is tokio's inherent
        // Windows method, same family as creation_flags below — no extra `use`.
        command.raw_arg(format!("/C \"{command_line}\""));
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(windows))]
    {
        let mut command = tokio::process::Command::new("sh");
        command.arg("-c").arg(command_line);
        command
    }
}

pub fn truncate_shell_output(value: &str) -> String {
    truncate_shell_output_flagged(value).0
}

/// Like [`truncate_shell_output`] but also reports whether truncation occurred, so
/// callers can surface a machine-readable flag (parallel to `ai_read_file`'s
/// `truncated`) instead of forcing the model to scrape the inline marker (E18).
/// The returned string is byte-for-byte identical to `truncate_shell_output`.
fn truncate_shell_output_flagged(value: &str) -> (String, bool) {
    let trimmed = value.trim();
    let total_chars = trimmed.chars().count();
    if total_chars <= AI_SHELL_MAX_OUTPUT_CHARS {
        return (trimmed.to_string(), false);
    }
    // Finding 6: head+tail truncation (first 12k + last 12k chars) with
    // omitted-character count, preserving both early context and final errors.
    //
    // Reserve room for the marker inside the cap so the reconstruction is always
    // strictly shorter than the input — for inputs only barely over the cap the
    // marker text would otherwise make the result LONGER than the original.
    // Reserve using the widest possible marker (omitted == total_chars) so the
    // reserved space is never smaller than the actual marker, guaranteeing
    // head + tail + marker <= AI_SHELL_MAX_OUTPUT_CHARS < total_chars.
    let widest_marker = format!("\n... [{total_chars} characters omitted] ...\n");
    let content_budget = AI_SHELL_MAX_OUTPUT_CHARS.saturating_sub(widest_marker.chars().count());
    let head_chars = AI_SHELL_TRUNCATE_HEAD_CHARS.min(content_budget);
    let tail_chars = AI_SHELL_TRUNCATE_TAIL_CHARS.min(content_budget - head_chars);
    let omitted = total_chars - head_chars - tail_chars;
    let head: String = trimmed.chars().take(head_chars).collect();
    let tail: String = trimmed.chars().skip(total_chars - tail_chars).collect();
    (
        format!("{head}\n... [{omitted} characters omitted] ...\n{tail}"),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lux_core::{LspRange, LspSymbolKind};

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

    // ── Finding 6: truncation tests ──

    #[test]
    fn truncate_shell_output_short_string_passes_through() {
        let result = truncate_shell_output("hello world");
        assert_eq!(result, "hello world");
    }

    #[test]
    fn truncate_shell_output_head_tail_preserved() {
        // Build a string barely over 24k chars.
        let prefix = "AAABBB";
        let middle = "X".repeat(AI_SHELL_MAX_OUTPUT_CHARS);
        let suffix = "CCCDDD";
        let long = format!("{prefix}{middle}{suffix}");
        let result = truncate_shell_output(&long);
        // Head is first 12k chars starting with AAA; tail is last 12k ending with DDD.
        assert!(
            result.starts_with("AAABBB"),
            "head should be preserved, got: {result:.50}..."
        );
        assert!(
            result.ends_with("CCCDDD"),
            "tail should be preserved, got: ...{}",
            &result[result.len().saturating_sub(50)..]
        );
        assert!(
            result.contains("characters omitted"),
            "omitted count marker missing"
        );
    }

    #[test]
    fn truncate_shell_output_exact_limit_unchanged() {
        let exact = "a".repeat(AI_SHELL_MAX_OUTPUT_CHARS);
        let result = truncate_shell_output(&exact);
        assert_eq!(result.len(), AI_SHELL_MAX_OUTPUT_CHARS);
    }

    #[test]
    fn truncate_shell_output_one_over_truncates() {
        let one_over = "a".repeat(AI_SHELL_MAX_OUTPUT_CHARS + 1);
        let result = truncate_shell_output(&one_over);
        assert!(result.len() < one_over.len());
        assert!(result.contains("characters omitted"));
    }

    // ── Finding 5: edge cases for shell output ──

    #[test]
    fn truncate_shell_output_whitespace_trimmed() {
        let result = truncate_shell_output("  \n  hello  \n  ");
        assert_eq!(result, "hello");
    }

    #[test]
    fn diff_stats_new_file_counts_created() {
        let stats = diff_stats("", "hello\nworld", true);
        assert_eq!(stats.lines_added, 2);
        assert_eq!(stats.lines_removed, 0);
        assert_eq!(stats.files_created, 1);
        assert_eq!(stats.files_changed, 0);
    }

    #[test]
    fn diff_stats_no_change_no_change_flag() {
        let stats = diff_stats("same", "same", false);
        assert_eq!(stats.lines_added, 0);
        assert_eq!(stats.files_changed, 0);
    }

    #[test]
    fn diff_stats_removal_only() {
        let stats = diff_stats("a\nb\nc", "", false);
        assert_eq!(stats.lines_removed, 3);
        assert_eq!(stats.lines_added, 0);
        assert_eq!(stats.files_changed, 1);
    }

    #[test]
    fn diff_stats_content_replacement_counts_changed_lines() {
        // F23: N-for-N replacement (same line count, all lines differ) must report
        // N added / N removed, not the net delta of 0/0.
        let stats = diff_stats("a\nb\nc", "x\ny\nz", false);
        assert_eq!(stats.lines_added, 3);
        assert_eq!(stats.lines_removed, 3);
        assert_eq!(stats.files_changed, 1);
        // A 1-line edit inside an otherwise-identical file changes exactly 1 line.
        let stats = diff_stats("a\nb\nc", "a\nB\nc", false);
        assert_eq!(stats.lines_added, 1);
        assert_eq!(stats.lines_removed, 1);
    }

    #[test]
    fn eol_detection_and_normalization() {
        assert_eq!(detect_eol("a\r\nb"), "\r\n");
        assert_eq!(detect_eol("a\nb"), "\n");
        assert_eq!(detect_eol("a\rb"), "\r");
        assert_eq!(detect_eol("no newline"), "\n");
        // LF-only model text is re-encoded to a CRLF file's ending so it matches.
        assert_eq!(normalize_eol("x\ny", "\r\n"), "x\r\ny");
        // Already-CRLF text is not doubled when targeting CRLF.
        assert_eq!(normalize_eol("x\r\ny", "\r\n"), "x\r\ny");
        // Targeting LF collapses CRLF and lone CR back to LF.
        assert_eq!(normalize_eol("x\r\ny", "\n"), "x\ny");
        assert_eq!(normalize_eol("x\ry", "\n"), "x\ny");
        // Classic-Mac target re-encodes LF to lone CR.
        assert_eq!(normalize_eol("x\ny", "\r"), "x\ry");
    }

    // ── Finding 4: tool def integer bounds (schema-level test) ──
    // See ai_tool_defs.rs `integer_params_have_bounds` for the JSON schema test.

    // ── Finding 7: staged StrReplace baseline ──
    // `update_open_document_after_text_change` is async and needs State —
    // testing its staged-branch logic fully requires an integration test with
    // a real SharedState / Documents instance. Unit-verify the helper fn.

    #[test]
    fn combine_patch_stats_empty() {
        assert_eq!(
            combine_patch_stats(&[]),
            AiFileOperationStats {
                lines_added: 0,
                lines_removed: 0,
                files_changed: 0,
                files_created: 0,
                files_deleted: 0,
            }
        );
    }

    #[test]
    fn unique_patch_paths_deduplicates() {
        let root = PathBuf::from("C:/work");
        let ops = vec![
            AiPreparedPatchOperation {
                kind: AiPreparedPatchKind::Rewrite,
                path: root.join("a.ts"),
                after_text: None,
                stats: AiFileOperationStats {
                    lines_added: 0,
                    lines_removed: 0,
                    files_changed: 0,
                    files_created: 0,
                    files_deleted: 0,
                },
            },
            AiPreparedPatchOperation {
                kind: AiPreparedPatchKind::Replace,
                path: root.join("a.ts"),
                after_text: None,
                stats: AiFileOperationStats {
                    lines_added: 0,
                    lines_removed: 0,
                    files_changed: 0,
                    files_created: 0,
                    files_deleted: 0,
                },
            },
        ];
        let paths = unique_patch_paths(&ops);
        assert_eq!(paths, vec![root.join("a.ts")]);
    }
}
