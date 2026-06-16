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
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize)]
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
                .open_loaded_file(&path, text)
                .map_err(String::from)?
        };
        emit_event(
            &app,
            LuxEvent::EditorDocumentChanged {
                document: document.clone(),
            },
        )?;
        if exists {
            lsp::forward_document_update(&app, &state, &document).await?;
        } else {
            lsp::forward_document_open(&app, &state, &document).await?;
        }
        Some(document)
    } else {
        let mut newly_opened = false;
        let existing = {
            let mut documents = state.documents.lock().map_err(lock_error)?;
            let existing = documents.snapshot_for_path(&path).map_err(String::from)?;
            match existing {
                Some(document) => Some(
                    documents
                        .update_text(document.id, text)
                        .map_err(String::from)?,
                ),
                // Staged (save_to_disk=false) write to a not-yet-open path: open an
                // in-memory doc and dirty-mark it so the new content is preserved
                // instead of being silently discarded while we report "file created".
                None => {
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
            }
        };
        if let Some(document) = &existing {
            emit_event(
                &app,
                LuxEvent::EditorDocumentChanged {
                    document: document.clone(),
                },
            )?;
            // A never-before-opened staged doc needs a didOpen first; sending
            // didChange (forward_document_update) for an unknown document would
            // be dropped or rejected by the language server. Mirror ai_file_patch.
            if newly_opened {
                lsp::forward_document_open(&app, &state, document).await?;
            } else {
                lsp::forward_document_update(&app, &state, document).await?;
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
        emit_event(
            &app,
            LuxEvent::EditorDocumentClosed {
                document: document.clone(),
            },
        )?;
        if let Some(path) = &document.path {
            lsp::forward_document_close(&app, &state, path).await?;
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
            lsp::forward_document_open(&app, &state, document).await?;
        } else {
            lsp::forward_document_update(&app, &state, document).await?;
        }
    }
    for path in &changed_paths {
        if save_to_disk {
            emit_event(&app, LuxEvent::FsChanged { path: path.clone() })?;
        }
        if prepared.iter().any(|operation| {
            operation.path == *path && operation.kind == AiPreparedPatchKind::Delete
        }) {
            lsp::apply_diagnostics_update(
                &app,
                state.inner(),
                lux_lsp::DiagnosticsUpdate {
                    path: path.clone(),
                    diagnostics: Vec::new(),
                },
            )?;
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
    let closed_documents = {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        if metadata.is_dir() {
            // remove_dir_all wiped a whole subtree: close every open document that
            // lived under the deleted directory, otherwise those tabs linger with
            // stale content and their LSP sessions / diagnostics are never cleared.
            let descendants: Vec<PathBuf> = documents
                .snapshots()
                .into_iter()
                .filter_map(|document| {
                    document
                        .path
                        .filter(|doc_path| doc_path.starts_with(&path))
                })
                .collect();
            let mut closed = Vec::new();
            for descendant in descendants {
                if let Some(document) = documents.close_path(&descendant).map_err(String::from)? {
                    closed.push(document);
                }
            }
            closed
        } else {
            documents
                .close_path(&path)
                .map_err(String::from)?
                .into_iter()
                .collect()
        }
    };
    for document in &closed_documents {
        emit_event(
            &app,
            LuxEvent::EditorDocumentClosed {
                document: document.clone(),
            },
        )?;
    }
    if metadata.is_dir() {
        for document in &closed_documents {
            if let Some(doc_path) = &document.path {
                lsp::forward_document_close(&app, &state, doc_path).await?;
                lsp::apply_diagnostics_update(
                    &app,
                    state.inner(),
                    lux_lsp::DiagnosticsUpdate {
                        path: doc_path.clone(),
                        diagnostics: Vec::new(),
                    },
                )?;
            }
        }
    } else {
        lsp::forward_document_close(&app, &state, &path).await?;
        lsp::apply_diagnostics_update(
            &app,
            state.inner(),
            lux_lsp::DiagnosticsUpdate {
                path: path.clone(),
                diagnostics: Vec::new(),
            },
        )?;
    }
    emit_event(&app, LuxEvent::FsChanged { path: path.clone() })?;
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
    process.current_dir(&cwd);
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

    // Own the child explicitly (rather than `process.output()`, which hides the
    // PID): drain both pipes concurrently to avoid a full-buffer deadlock, then
    // reap. Borrowing `child` here means a timeout drops only this future's
    // borrow, leaving the child alive so the tree-kill below can target it.
    let collect = async {
        use tokio::io::AsyncReadExt;
        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        let read_stdout = async {
            if let Some(pipe) = stdout_pipe.as_mut() {
                let _ = pipe.read_to_end(&mut stdout_buf).await;
            }
        };
        let read_stderr = async {
            if let Some(pipe) = stderr_pipe.as_mut() {
                let _ = pipe.read_to_end(&mut stderr_buf).await;
            }
        };
        tokio::join!(read_stdout, read_stderr);
        let status = child.wait().await;
        (status, stdout_buf, stderr_buf)
    };

    let output_result = timeout(Duration::from_secs(timeout_secs), collect).await;
    let duration_ms = started.elapsed().as_millis();

    match output_result {
        Ok((Ok(status), stdout_buf, stderr_buf)) => Ok(AiShellResponse {
            workspace_root: root,
            cwd,
            command,
            exit_code: status.code(),
            duration_ms,
            stdout: truncate_shell_output(&String::from_utf8_lossy(&stdout_buf)),
            stderr: truncate_shell_output(&String::from_utf8_lossy(&stderr_buf)),
            timed_out: false,
            warnings: safety.warnings,
            read_only: safety.read_only,
        }),
        Ok((Err(error), _, _)) => Err(format!("Failed to run shell command: {error}")),
        Err(_) => {
            // Timed out: `child` is still alive (only the borrow held by `collect`
            // was dropped). Kill the whole process tree before returning so no
            // grandchild keeps running orphaned; start_kill backstops the shell.
            kill_process_tree(child_pid).await;
            let _ = child.start_kill();
            Ok(AiShellResponse {
                workspace_root: root,
                cwd,
                command,
                exit_code: None,
                duration_ms,
                stdout: String::new(),
                stderr: format!("Shell command timed out after {timeout_secs} seconds"),
                timed_out: true,
                warnings: safety.warnings,
                read_only: safety.read_only,
            })
        }
    }
}

/// Best-effort kill of a timed-out shell command's entire process tree.
/// `kill_on_drop` only terminates the immediate `cmd.exe`/`sh` child, so any
/// grandchildren it spawned would otherwise keep running orphaned.
async fn kill_process_tree(pid: Option<u32>) {
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
) -> Result<AiReadFileResult, String> {
    let max_bytes = max_bytes.unwrap_or(120_000).max(1);
    let path = resolve_workspace_path(&state, &path)?;
    tokio::task::spawn_blocking(move || -> Result<AiReadFileResult, String> {
        use std::io::Read;
        let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("path is not a file".to_string());
        }
        let size = metadata.len();
        // Bound the read with Take + read_to_end so a short read() can't silently
        // truncate the content; read_to_end loops until the capped EOF.
        let limit = max_bytes.min(size);
        let mut buffer = Vec::new();
        std::fs::File::open(&path)
            .map_err(|e| e.to_string())?
            .take(limit)
            .read_to_end(&mut buffer)
            .map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&buffer).into_owned();
        Ok(AiReadFileResult {
            path,
            text,
            truncated: size > max_bytes,
            size,
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
}

/// List workspace files matching a pattern — public wrapper for the turn-loop.
pub async fn ai_glob(
    state: State<'_, SharedState>,
    pattern: String,
    max_results: Option<usize>,
) -> Result<AiGlobResult, String> {
    let root = workspace_root(&state)?;
    let max = max_results.unwrap_or(80).clamp(1, 500);
    let pattern = pattern.trim().to_lowercase().replace('\\', "/");
    // Push the substring filter into the walk and stop once `max` files match.
    // This finds matches anywhere in the tree (no pre-filter file-count cap that
    // would drop late-sorting matches as a misleading "file not found") while
    // never materializing the entire workspace — only the matched subset.
    let scan_pattern = pattern.clone();
    let files: Vec<PathBuf> = tokio::task::spawn_blocking(move || {
        lux_fs::list_files_matching(
            root,
            move |path| {
                path.to_string_lossy()
                    .to_lowercase()
                    .replace('\\', "/")
                    .contains(&scan_pattern)
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
        pattern: pattern.clone(),
        count: files.len(),
        files,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiGlobResult {
    pub pattern: String,
    pub count: usize,
    pub files: Vec<PathBuf>,
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
        lsp::forward_document_update(app, state, document).await?;
    }
    Ok(updated)
}

fn diff_stats(before: &str, after: &str, created: bool) -> AiFileOperationStats {
    let before_lines = before.lines().count();
    let after_lines = after.lines().count();
    AiFileOperationStats {
        lines_added: after_lines.saturating_sub(before_lines),
        lines_removed: before_lines.saturating_sub(after_lines),
        files_changed: usize::from(!created && before != after),
        files_created: usize::from(created),
        files_deleted: 0,
    }
}

fn shell_command(command_line: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let mut command = tokio::process::Command::new("cmd");
        command.arg("/C").arg(command_line);
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

fn truncate_shell_output(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= AI_SHELL_MAX_OUTPUT_CHARS {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(AI_SHELL_MAX_OUTPUT_CHARS).collect();
    format!("{head}\n...[truncated]")
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
}
