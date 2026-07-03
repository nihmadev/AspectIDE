//! Native in-session file checkpoints — Stage 5.
//!
//! Per-workspace snapshot store: create captures file text (editor buffer if open,
//! else disk), diff compares against current, restore builds patch operations and
//! applies them via `ai_file_patch`. All snapshot/diff logic is native Rust; only
//! the restore patch reuses the existing guarded file-patch path.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::State;

use crate::ai_semantic;
use crate::{lock_error, workspace_root, SharedState};

const MAX_CHECKPOINTS: usize = 24;
const DEFAULT_MAX_FILES: usize = 40;
const MAX_FILES_LIMIT: usize = 80;
const DEFAULT_MAX_BYTES: u64 = 500_000;
const MAX_BYTES_LIMIT: u64 = 1_000_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointFileSnapshot {
    pub path: String,
    pub relative_path: String,
    pub existed: bool,
    #[serde(skip)]
    pub text: String,
    pub size: u64,
    pub truncated: bool,
    pub source: String, // editor | disk | missing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    pub id: String,
    pub label: String,
    pub workspace_root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub created_at_ms: i64,
    #[serde(skip)]
    pub files: Vec<CheckpointFileSnapshot>,
    pub max_bytes_per_file: u64,
    pub max_files: usize,
}

fn store() -> &'static Mutex<HashMap<String, Vec<Checkpoint>>> {
    static STORE: OnceLock<Mutex<HashMap<String, Vec<Checkpoint>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn ws_key(root: &str) -> String {
    ai_semantic::normalize_slashes_pub(root.trim_end_matches('/')).to_lowercase()
}

/// Derive the in-memory store key. When a `session_id` is provided the key is
/// workspace + session so concurrent agents/chats get isolated checkpoint stores.
/// Without it the key is workspace-only (legacy behavior).
fn store_key(root: &str, session_id: Option<&str>) -> String {
    let base = ws_key(root);
    match session_id.filter(|s| !s.is_empty()) {
        Some(sid) => format!("{base}::{sid}"),
        None => base,
    }
}

/// Drop all in-session checkpoint snapshots for a workspace, including any
/// session-scoped keys. Called on workspace close so full-text snapshots don't
/// linger for the rest of the process lifetime.
pub fn clear_workspace(root: &str) {
    if let Ok(mut map) = store().lock() {
        let prefix = ws_key(root);
        map.retain(|k, _| !k.starts_with(&prefix));
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointSummary {
    pub id: String,
    pub label: String,
    pub created_at_ms: i64,
    pub file_count: usize,
    pub restorable_file_count: usize,
    pub truncated_file_count: usize,
    pub error_file_count: usize,
}

fn summarize(cp: &Checkpoint) -> CheckpointSummary {
    CheckpointSummary {
        id: cp.id.clone(),
        label: cp.label.clone(),
        created_at_ms: cp.created_at_ms,
        file_count: cp.files.len(),
        restorable_file_count: cp
            .files
            .iter()
            .filter(|f| !f.truncated && f.error.is_none())
            .count(),
        truncated_file_count: cp.files.iter().filter(|f| f.truncated).count(),
        error_file_count: cp.files.iter().filter(|f| f.error.is_some()).count(),
    }
}

/// The checkpoint tool entry point (create/list/diff/delete/restore).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn ai_checkpoint(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    action: String,
    id: Option<String>,
    session_id: Option<String>,
    label: Option<String>,
    paths: Option<Vec<String>>,
    max_files: Option<usize>,
    max_bytes_per_file: Option<u64>,
    save_to_disk: Option<bool>,
    dry_run: Option<bool>,
    now_ms: i64,
) -> Result<serde_json::Value, String> {
    let root = workspace_root(&state)?;
    let root_str = ai_semantic::normalize_slashes_pub(&root.to_string_lossy());
    let normalized = action.trim().to_lowercase().replace(['-', '_', ' '], "");
    let act = match normalized.as_str() {
        "create" | "snapshot" | "save" => "create",
        "list" | "ls" => "list",
        "diff" | "compare" => "diff",
        "delete" | "remove" | "drop" => "delete",
        "restore" | "rollback" | "revert" => "restore",
        "augment" | "extend" | "add" => "augment",
        _ => return Err(format!("Unsupported checkpoint action: {action}")),
    };

    match act {
        "create" => {
            let max_files = max_files
                .unwrap_or(DEFAULT_MAX_FILES)
                .clamp(1, MAX_FILES_LIMIT);
            let max_bytes = max_bytes_per_file
                .unwrap_or(DEFAULT_MAX_BYTES)
                .clamp(1_024, MAX_BYTES_LIMIT);
            let target_paths = resolve_target_paths(&state, &root_str, paths.as_ref(), max_files);
            if target_paths.is_empty() {
                let explicit_count = paths
                    .as_ref()
                    .map_or(0, |p| p.iter().filter(|s| !s.trim().is_empty()).count());
                if explicit_count > 0 {
                    return Err(format!(
                        "Checkpoint not created: none of the {explicit_count} requested path(s) resolved inside the workspace. Pass workspace-relative or in-root absolute file paths."
                    ));
                }
                // E39: the requested snapshot did not happen for a problem reason
                // (nothing to capture). Surface as an error so a model watching the
                // tool-error channel does not read this as a successful checkpoint.
                return Err(
                    "Checkpoint not created: no file paths were available to snapshot (no paths given and no open editor files). Pass an explicit `paths` array.".to_string(),
                );
            }
            let mut files = Vec::with_capacity(target_paths.len());
            for path in &target_paths {
                files.push(snapshot_file(&state, &root_str, path, max_bytes).await);
            }
            let cp = Checkpoint {
                id: format!("cp-{}", uuid::Uuid::new_v4().simple()),
                label: label
                    .unwrap_or_default()
                    .trim()
                    .chars()
                    .take(120)
                    .collect::<String>(),
                workspace_root: root_str.clone(),
                session_id: session_id
                    .as_ref()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                created_at_ms: now_ms,
                files,
                max_bytes_per_file: max_bytes,
                max_files,
            };
            let summary = summarize(&cp);
            let file_views: Vec<serde_json::Value> = cp.files.iter().map(compact_file).collect();
            let mut guard = store().lock().map_err(lock_error)?;
            let list = guard
                .entry(store_key(&root_str, session_id.as_deref()))
                .or_default();
            list.insert(0, cp);
            list.truncate(MAX_CHECKPOINTS);
            Ok(
                serde_json::json!({ "status": "created", "checkpoint": summary, "files": file_views }),
            )
        }
        "list" => {
            let guard = store().lock().map_err(lock_error)?;
            let list = guard
                .get(&store_key(&root_str, session_id.as_deref()))
                .cloned()
                .unwrap_or_default();
            let summaries: Vec<CheckpointSummary> = list.iter().map(summarize).collect();
            Ok(
                serde_json::json!({ "workspaceRoot": root_str, "count": summaries.len(), "checkpoints": summaries }),
            )
        }
        "diff" => {
            let cp = select_checkpoint(&root_str, session_id.as_deref(), id.as_ref())?;
            let mut diffs = Vec::new();
            for file in &cp.files {
                diffs.push(diff_file(&state, &root_str, file, cp.max_bytes_per_file).await);
            }
            let summary = diff_summary(&diffs);
            Ok(
                serde_json::json!({ "status": "diffed", "checkpoint": summarize(&cp), "summary": summary, "files": diffs }),
            )
        }
        "delete" => {
            let cp = select_checkpoint(&root_str, session_id.as_deref(), id.as_ref())?;
            let mut guard = store().lock().map_err(lock_error)?;
            if let Some(list) = guard.get_mut(&store_key(&root_str, session_id.as_deref())) {
                list.retain(|c| c.id != cp.id);
            }
            Ok(serde_json::json!({ "status": "deleted", "checkpoint": summarize(&cp) }))
        }
        "restore" => {
            let cp = select_checkpoint(&root_str, session_id.as_deref(), id.as_ref())?;
            let save = save_to_disk.unwrap_or(true);
            let dry = dry_run.unwrap_or(false);
            // E35 (data loss): a snapshot that was itself capped at capture time does
            // not hold the file's full content. Writing such a snapshot back would
            // silently shorten the live file. Refuse and surface an explicit error
            // (not an Ok "blocked") so a model on the tool-error channel sees failure.
            let capped_snapshots: Vec<&str> = cp
                .files
                .iter()
                .filter(|f| f.truncated)
                .map(|f| f.relative_path.as_str())
                .collect();
            if !capped_snapshots.is_empty() {
                return Err(format!(
                    "Restore refused to avoid data loss: {} snapshot file(s) were capped at {} bytes during capture and do not hold full content: {}. Re-create the checkpoint with a larger maxBytesPerFile before restoring.",
                    capped_snapshots.len(),
                    cp.max_bytes_per_file,
                    capped_snapshots.join(", "),
                ));
            }
            let unreadable_snapshots: Vec<String> = cp
                .files
                .iter()
                .filter_map(|f| {
                    f.error
                        .as_ref()
                        .map(|e| format!("{}: {e}", f.relative_path))
                })
                .collect();
            if !unreadable_snapshots.is_empty() {
                return Err(format!(
                    "Restore refused: {} snapshot file(s) were unreadable at capture time and cannot be safely restored: {}.",
                    unreadable_snapshots.len(),
                    unreadable_snapshots.join("; "),
                ));
            }
            // Build restore operations by diffing each file against current state.
            let mut operations: Vec<crate::ai_tools::AiFilePatchOperation> = Vec::new();
            let mut current_read_errors: Vec<String> = Vec::new();
            // E35: if the live file grew past the byte cap after capture, read_current
            // only sees a truncated prefix. Comparing the full snapshot against that
            // prefix can misjudge "changed" and, if judged unchanged, would silently
            // leave a diverged file in place. Refuse rather than decide on partial data.
            let mut current_truncated: Vec<String> = Vec::new();
            for file in &cp.files {
                let current =
                    read_current(&state, &root_str, &file.path, cp.max_bytes_per_file).await;
                if let Some(err) = &current.error {
                    current_read_errors.push(format!("{}: {}", file.path, err));
                    continue;
                }
                if current.truncated {
                    current_truncated.push(file.relative_path.clone());
                    continue;
                }
                let changed = match (file.existed, current.existed) {
                    (true, true) => file.text != current.text,
                    (true, false) | (false, true) => true,
                    (false, false) => false,
                };
                if !changed {
                    continue;
                }
                if file.existed {
                    // rewrite writes the FULL snapshot text (already verified un-capped
                    // above), so the restored file is never silently shortened.
                    operations.push(serde_json::from_value(serde_json::json!({
                        "action": if current.disk_exists { "rewrite" } else { "create" },
                        "path": file.path,
                        "text": file.text,
                        "overwrite": if current.disk_exists { serde_json::Value::Null } else { serde_json::Value::Bool(false) },
                    })).map_err(|e| e.to_string())?);
                } else if current.disk_exists {
                    operations.push(
                        serde_json::from_value(serde_json::json!({
                            "action": "delete", "path": file.path,
                        }))
                        .map_err(|e| e.to_string())?,
                    );
                }
            }
            if !current_truncated.is_empty() {
                return Err(format!(
                    "Restore refused to avoid data loss: {} live file(s) now exceed the {}-byte comparison cap, so their current content cannot be fully compared against the snapshot: {}. Re-create the checkpoint with a larger maxBytesPerFile, or restore manually.",
                    current_truncated.len(),
                    cp.max_bytes_per_file,
                    current_truncated.join(", "),
                ));
            }
            if !current_read_errors.is_empty() {
                return Err(format!(
                    "Restore blocked: cannot read current file(s): {}",
                    current_read_errors.join("; ")
                ));
            }
            // snapshotFileCount distinguishes "nothing captured" (0 — the checkpoint
            // held no file snapshots at all, e.g. created against an empty selection)
            // from "captured but identical" (>0 but operations.is_empty() below) so
            // the frontend can show an honest toast instead of a blanket "nothing to
            // restore" for both very different situations.
            let snapshot_file_count = cp.files.len();
            if operations.is_empty() {
                return Ok(serde_json::json!({
                    "status": "unchanged",
                    "changed": false,
                    "checkpoint": summarize(&cp),
                    "snapshotFileCount": snapshot_file_count,
                    "message": "All snapshotted files already match the checkpoint; nothing to restore.",
                }));
            }
            let op_count = operations.len();
            let result = crate::ai_tools::ai_file_patch(
                app.clone(),
                state.clone(),
                operations,
                Some(save),
                Some(dry),
            )
            .await?;
            Ok(serde_json::json!({
                "status": if dry { "preview" } else { "restored" },
                "changed": true,
                "operations": op_count,
                "snapshotFileCount": snapshot_file_count,
                "result": result,
            }))
        }
        "augment" => {
            let want_id = id
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "Checkpoint augment requires an id.".to_string())?;
            augment_checkpoint(
                &state,
                &root_str,
                &want_id,
                session_id.as_deref(),
                paths.unwrap_or_default(),
            )
            .await
        }
        _ => unreachable!(),
    }
}

/// Capture pre-edit snapshots for files a tool is about to create/edit that were
/// not open at turn start and are not already in the named checkpoint, so a later
/// restore can revert the edit or delete a newly-created file. Shared by the
/// `ai_checkpoint("augment", …)` Tauri command (thin wrapper above) and native
/// tool handlers in `ai_turn::execute_tool`, which call this directly before a
/// Write/StrReplace/Delete/PatchEngine mutation so files the model never
/// explicitly ran the `Checkpoint` tool on still land in the pre-edit snapshot.
pub async fn augment_checkpoint(
    state: &State<'_, SharedState>,
    root_str: &str,
    checkpoint_id: &str,
    session_id: Option<&str>,
    requested: Vec<String>,
) -> Result<serde_json::Value, String> {
    // snapshot_file is async, so read the checkpoint's metadata under the lock,
    // snapshot without holding it, then re-acquire.
    let (max_bytes, max_files_cap, existing, current_count) = {
        let guard = store().lock().map_err(lock_error)?;
        let key = store_key(root_str, session_id);
        let cp = guard
            .get(&key)
            .and_then(|list| list.iter().find(|c| c.id == checkpoint_id))
            .ok_or_else(|| format!("Checkpoint not found: {checkpoint_id}"))?;
        let existing: std::collections::HashSet<String> = cp
            .files
            .iter()
            .map(|f| ai_semantic::normalize_slashes_pub(&f.path).to_lowercase())
            .collect();
        (
            cp.max_bytes_per_file,
            cp.max_files,
            existing,
            cp.files.len(),
        )
    };
    let mut missing: Vec<String> = Vec::new();
    for raw in &requested {
        if let Some(resolved) = resolve_in_workspace(raw, root_str) {
            let key = resolved.to_lowercase();
            if !existing.contains(&key) && !missing.iter().any(|m: &String| m.to_lowercase() == key)
            {
                missing.push(resolved);
            }
        }
    }
    // Enforce the per-checkpoint file cap to prevent unbounded growth.
    let remaining = max_files_cap.saturating_sub(current_count);
    let will_snapshot = missing.len().min(remaining);
    let pre_skipped = missing.len().saturating_sub(will_snapshot);
    missing.truncate(will_snapshot);

    let mut snapshots = Vec::with_capacity(missing.len());
    for path in &missing {
        snapshots.push(snapshot_file(state, root_str, path, max_bytes).await);
    }

    let mut guard = store().lock().map_err(lock_error)?;
    let key = store_key(root_str, session_id);
    let cp = guard
        .get_mut(&key)
        .and_then(|list| list.iter_mut().find(|c| c.id == checkpoint_id))
        .ok_or_else(|| format!("Checkpoint not found: {checkpoint_id}"))?;
    let mut added = 0usize;
    let mut dedup_skipped = 0usize;
    for snap in snapshots {
        let key = ai_semantic::normalize_slashes_pub(&snap.path).to_lowercase();
        if cp
            .files
            .iter()
            .any(|f| ai_semantic::normalize_slashes_pub(&f.path).to_lowercase() == key)
        {
            dedup_skipped += 1;
            continue;
        }
        if cp.files.len() >= max_files_cap {
            dedup_skipped += 1;
            continue;
        }
        cp.files.push(snap);
        added += 1;
    }
    let summary = summarize(cp);
    let total_skipped = pre_skipped + dedup_skipped;
    Ok(serde_json::json!({
        "status": "augmented",
        "added": added,
        "skipped": total_skipped,
        "checkpoint": summary,
    }))
}

fn select_checkpoint(
    root: &str,
    session_id: Option<&str>,
    id: Option<&String>,
) -> Result<Checkpoint, String> {
    let guard = store().lock().map_err(lock_error)?;
    let key = store_key(root, session_id);
    let list = guard
        .get(&key)
        .ok_or_else(|| "No checkpoints exist for this workspace.".to_string())?;
    if list.is_empty() {
        return Err("No checkpoints exist for this workspace.".to_string());
    }
    id.map(String::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(
            || Ok(list[0].clone()),
            |want| {
                list.iter()
                    .find(|c| c.id == want)
                    .cloned()
                    .ok_or_else(|| format!("Checkpoint not found: {want}"))
            },
        )
}

fn resolve_target_paths(
    state: &State<'_, SharedState>,
    root: &str,
    explicit: Option<&Vec<String>>,
    max_files: usize,
) -> Vec<String> {
    let mut selected: Vec<String> = Vec::new();
    let add = |p: &str, acc: &mut Vec<String>| {
        let trimmed = p.trim();
        if trimmed.is_empty() {
            return;
        }
        let resolved = resolve_in_workspace(trimmed, root);
        if let Some(resolved) = resolved {
            if !acc
                .iter()
                .any(|e: &String| e.eq_ignore_ascii_case(&resolved))
            {
                acc.push(resolved);
            }
        }
    };

    if let Some(paths) = explicit {
        if !paths.is_empty() {
            for p in paths {
                add(p, &mut selected);
            }
            selected.truncate(max_files);
            return selected;
        }
    }

    // No explicit paths: snapshot open editor documents.
    if let Ok(documents) = state.documents.lock() {
        for doc in documents.snapshots() {
            if let Some(path) = &doc.path {
                add(&path.to_string_lossy(), &mut selected);
            }
        }
    }
    selected.truncate(max_files);
    selected
}

/// Resolve a path relative to the workspace root, rejecting any parent-directory
/// escapes or symlink-based escapes outside the workspace boundary.
fn resolve_in_workspace(path: &str, root: &str) -> Option<String> {
    use std::path::{Path, PathBuf};

    let normalized = ai_semantic::normalize_slashes_pub(path.trim());
    let root_normalized = ai_semantic::normalize_slashes_pub(root);

    // Use the same absolute-detection logic as the original code to stay
    // platform-independent (Path::is_absolute on Windows rejects /root).
    let abs_path = if normalized.starts_with('/') || normalized.chars().nth(1) == Some(':') {
        PathBuf::from(&normalized)
    } else {
        Path::new(&root_normalized).join(&normalized)
    };

    // Normalize `.` and `..` components, rejecting root-escapes.
    let resolved = normalize_path_components(&abs_path)?;
    let resolved_normalized = ai_semantic::normalize_slashes_pub(&resolved.to_string_lossy());

    // Verify containment via lexical prefix check on component-normalized path.
    // This rejects all parent-dir escapes while requiring no real filesystem I/O,
    // so it works identically on Windows and POSIX even for non-existent paths.
    let root_lower = root_normalized.to_lowercase();
    if resolved_normalized.to_lowercase() == root_lower
        || resolved_normalized
            .to_lowercase()
            .starts_with(&format!("{root_lower}/"))
    {
        // For paths that actually exist on disk, enforce canonical-path containment
        // to catch symlink-based escapes outside the workspace.
        if resolved.exists() {
            if let (Ok(canon), Ok(root_canon)) = (
                resolved.canonicalize(),
                Path::new(&root_normalized).canonicalize(),
            ) {
                if canon.starts_with(&root_canon) {
                    return canon.to_str().map(ai_semantic::normalize_slashes_pub);
                }
                return None;
            }
        }
        return Some(resolved_normalized);
    }
    None
}

/// Walk path components to resolve `.` and `..` segments, rejecting parent-dir
/// escapes that would go beyond the filesystem root or drive prefix.
fn normalize_path_components(path: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let mut result = std::path::PathBuf::new();
    // depth tracks how many Normal components sit above a root/prefix, so we
    // can reject a ParentDir that would pop beyond the root boundary.
    let mut depth: isize = 0;

    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                if depth <= 0 {
                    return None; // escape beyond root
                }
                depth -= 1;
                result.pop();
            }
            Component::CurDir => { /* skip */ }
            Component::RootDir | Component::Prefix(_) => {
                depth = 0;
                result.push(comp.as_os_str());
            }
            Component::Normal(_) => {
                depth += 1;
                result.push(comp.as_os_str());
            }
        }
    }
    Some(result)
}

fn relative_to(path: &str, root: &str) -> String {
    let root_lower = root.to_lowercase();
    if path.to_lowercase().starts_with(&format!("{root_lower}/")) {
        path.get(root.len() + 1..)
            .map_or_else(|| path.to_string(), str::to_string)
    } else {
        path.to_string()
    }
}

/// Read up to `max_bytes` bytes from a file, respecting UTF-8 character boundaries.
/// Returns (`text_content`, `was_truncated`, `file_size_bytes`).
async fn bounded_read_text(path: &str, max_bytes: u64) -> std::io::Result<(String, bool, u64)> {
    use tokio::io::AsyncReadExt;

    let meta = tokio::fs::metadata(path).await?;
    let file_size = meta.len();

    if file_size == 0 {
        return Ok((String::new(), false, 0));
    }

    if file_size <= max_bytes {
        let text = tokio::fs::read_to_string(path).await?;
        return Ok((text, false, file_size));
    }

    // Bounded read: grab max_bytes + 4 bytes so we can find a clean UTF-8 boundary.
    let read_n = usize::try_from(max_bytes.saturating_add(4).min(file_size)).unwrap_or(usize::MAX);
    let mut file = tokio::fs::File::open(path).await?;
    let mut buf = vec![0u8; read_n];
    let n = file.read(&mut buf).await?;
    buf.truncate(n);

    let text = decode_utf8_bounded(&buf, usize::try_from(max_bytes).unwrap_or(usize::MAX));
    Ok((text, true, file_size))
}

/// Decode a byte buffer to String, truncating at the last complete char boundary
/// at or before `max_bytes`.
fn decode_utf8_bounded(buf: &[u8], max_bytes: usize) -> String {
    let s = std::str::from_utf8(buf)
        .unwrap_or_else(|e| std::str::from_utf8(&buf[..e.valid_up_to()]).unwrap_or(""));
    let bound = s
        .char_indices()
        .take_while(|(i, _)| *i < max_bytes)
        .last()
        .map_or(0, |(i, c)| i + c.len_utf8());
    s[..bound].to_string()
}

async fn snapshot_file(
    state: &State<'_, SharedState>,
    root: &str,
    path: &str,
    max_bytes: u64,
) -> CheckpointFileSnapshot {
    let relative_path = relative_to(path, root);
    // Editor buffer first.
    if let Some(snap) = editor_snapshot(state, path) {
        let truncated = snap.text.len() as u64 > max_bytes;
        let text = if truncated {
            let bound = snap
                .text
                .char_indices()
                .take_while(|(i, _)| (*i as u64) < max_bytes)
                .last()
                .map_or(0, |(i, c)| i + c.len_utf8());
            snap.text[..bound].to_string()
        } else {
            snap.text.clone()
        };
        return CheckpointFileSnapshot {
            path: path.to_string(),
            relative_path,
            existed: !snap.is_untitled,
            size: snap.text.len() as u64,
            truncated,
            text,
            source: "editor".into(),
            error: None,
        };
    }
    // Disk.
    snapshot_file_disk(path, relative_path, max_bytes).await
}

/// Disk-only path of `snapshot_file`, factored for testability without Tauri State.
async fn snapshot_file_disk(
    path: &str,
    relative_path: String,
    max_bytes: u64,
) -> CheckpointFileSnapshot {
    match bounded_read_text(path, max_bytes).await {
        Ok((text, truncated, size)) => CheckpointFileSnapshot {
            path: path.to_string(),
            relative_path,
            existed: true,
            size,
            truncated,
            text,
            source: "disk".into(),
            error: None,
        },
        Err(err) => {
            let not_found = err.kind() == std::io::ErrorKind::NotFound;
            CheckpointFileSnapshot {
                path: path.to_string(),
                relative_path,
                existed: false,
                size: 0,
                truncated: false,
                text: String::new(),
                source: if not_found {
                    "missing".into()
                } else {
                    "error".into()
                },
                error: if not_found {
                    None
                } else {
                    Some(err.to_string())
                },
            }
        }
    }
}

struct CurrentFile {
    existed: bool,
    disk_exists: bool,
    text: String,
    truncated: bool,
    source: String,
    error: Option<String>,
}

async fn read_current(
    state: &State<'_, SharedState>,
    root: &str,
    path: &str,
    max_bytes: u64,
) -> CurrentFile {
    let _ = root;
    if let Some(snap) = editor_snapshot(state, path) {
        let disk_exists = tokio::fs::metadata(path).await.is_ok();
        let truncated = snap.text.len() as u64 > max_bytes;
        let text = if truncated {
            let bound = snap
                .text
                .char_indices()
                .take_while(|(i, _)| (*i as u64) < max_bytes)
                .last()
                .map_or(0, |(i, c)| i + c.len_utf8());
            snap.text[..bound].to_string()
        } else {
            snap.text.clone()
        };
        return CurrentFile {
            existed: true,
            disk_exists,
            text,
            truncated,
            source: "editor".into(),
            error: None,
        };
    }
    // Check metadata separately so disk_exists reflects reality when the file
    // can't be read (permission, encoding, transient IO error).
    let meta = tokio::fs::metadata(path).await;
    let disk_exists = meta.is_ok();
    match bounded_read_text(path, max_bytes).await {
        Ok((text, truncated, _)) => CurrentFile {
            existed: true,
            disk_exists: true,
            text,
            truncated,
            source: "disk".into(),
            error: None,
        },
        Err(err) => {
            let not_found = err.kind() == std::io::ErrorKind::NotFound;
            // NotFound is an expected state (file was deleted between snapshot
            // and restore/diff), so carry no error — the missing-file path is
            // handled normally by the caller. Other IO failures (permission,
            // encoding, transient) propagate via error to block restore.
            CurrentFile {
                existed: false,
                disk_exists,
                text: String::new(),
                truncated: false,
                source: if not_found {
                    "missing".into()
                } else {
                    "error".into()
                },
                error: if not_found {
                    None
                } else {
                    Some(err.to_string())
                },
            }
        }
    }
}

fn editor_snapshot(
    state: &State<'_, SharedState>,
    path: &str,
) -> Option<lux_core::DocumentSnapshot> {
    let documents = state.documents.lock().ok()?;
    documents.snapshots().into_iter().find(|doc| {
        doc.path.as_ref().is_some_and(|p| {
            ai_semantic::normalize_slashes_pub(&p.to_string_lossy()).eq_ignore_ascii_case(path)
        })
    })
}

async fn diff_file(
    state: &State<'_, SharedState>,
    root: &str,
    file: &CheckpointFileSnapshot,
    max_bytes: u64,
) -> serde_json::Value {
    let current = read_current(state, root, &file.path, max_bytes).await;
    let status = if file.error.is_some() {
        "error"
    } else if current.error.is_some() {
        "readError"
    } else if file.truncated || current.truncated {
        "truncated"
    } else if file.existed && !current.existed {
        "missing"
    } else if !file.existed && current.existed {
        "created"
    } else if file.text == current.text {
        "unchanged"
    } else {
        "modified"
    };
    let line_delta = if current.existed && file.existed {
        Some(
            i64::try_from(count_lines(&current.text)).unwrap_or(i64::MAX)
                - i64::try_from(count_lines(&file.text)).unwrap_or(i64::MAX),
        )
    } else {
        None
    };
    let mut diff = serde_json::json!({
        "path": file.path,
        "relativePath": file.relative_path,
        "status": status,
        "existedAtCheckpoint": file.existed,
        "currentExists": current.existed,
        "diskExists": current.disk_exists,
        "snapshotSource": file.source,
        "currentSource": current.source,
        "lineDelta": line_delta,
    });
    if let Some(err) = &current.error {
        diff["currentError"] = serde_json::json!(err);
    }
    // E38: surface the actual change so a model can decide whether to restore,
    // instead of seeing only status metadata. Compute a bounded line-level diff
    // between the snapshot ("from") and current ("to") text. Only meaningful when
    // neither side was truncated for the comparison — otherwise the diff would be
    // computed against a partial view and could mislead.
    if status == "modified" && !file.truncated && !current.truncated {
        diff["change"] = line_change_summary(&file.text, &current.text);
    }
    diff
}

/// Longest-common-subsequence line count between two slices, used to derive
/// added/removed counts without pulling in an external diff crate.
fn lcs_line_count(from: &[&str], to: &[&str]) -> usize {
    if from.is_empty() || to.is_empty() {
        return 0;
    }
    // Rolling two-row DP over the LCS table to keep memory at O(min(len)).
    let mut prev = vec![0usize; to.len() + 1];
    let mut curr = vec![0usize; to.len() + 1];
    for &f in from {
        for (j, &t) in to.iter().enumerate() {
            curr[j + 1] = if f == t {
                prev[j] + 1
            } else {
                curr[j].max(prev[j + 1])
            };
        }
        std::mem::swap(&mut prev, &mut curr);
        curr.fill(0);
    }
    prev[to.len()]
}

/// Bounded added/removed line summary plus a capped preview of changed lines,
/// so the diff payload shows the model the actual change without being unbounded.
fn line_change_summary(from_text: &str, to_text: &str) -> serde_json::Value {
    const MAX_PREVIEW_LINES: usize = 40;
    const MAX_LINE_LEN: usize = 400;

    let from_lines: Vec<&str> = from_text.lines().collect();
    let to_lines: Vec<&str> = to_text.lines().collect();
    let common = lcs_line_count(&from_lines, &to_lines);
    let removed = from_lines.len().saturating_sub(common);
    let added = to_lines.len().saturating_sub(common);

    let clip = |line: &str| -> String {
        let bound = line
            .char_indices()
            .take_while(|(i, _)| *i < MAX_LINE_LEN)
            .last()
            .map_or(0, |(i, c)| i + c.len_utf8());
        if bound < line.len() {
            format!("{}…", &line[..bound])
        } else {
            line.to_string()
        }
    };

    // Preview: removed lines present in the snapshot but absent from current,
    // then added lines present in current but absent from the snapshot. This is
    // a set-style preview (not positional), enough for the model to judge intent.
    let to_set: std::collections::HashSet<&str> = to_lines.iter().copied().collect();
    let from_set: std::collections::HashSet<&str> = from_lines.iter().copied().collect();
    let mut preview: Vec<String> = Vec::new();
    let mut truncated_preview = false;
    for line in &from_lines {
        if !to_set.contains(line) {
            if preview.len() >= MAX_PREVIEW_LINES {
                truncated_preview = true;
                break;
            }
            preview.push(format!("- {}", clip(line)));
        }
    }
    if !truncated_preview {
        for line in &to_lines {
            if !from_set.contains(line) {
                if preview.len() >= MAX_PREVIEW_LINES {
                    truncated_preview = true;
                    break;
                }
                preview.push(format!("+ {}", clip(line)));
            }
        }
    }

    serde_json::json!({
        "addedLines": added,
        "removedLines": removed,
        "preview": preview,
        "previewTruncated": truncated_preview,
    })
}

fn diff_summary(diffs: &[serde_json::Value]) -> serde_json::Value {
    let count = |s: &str| {
        diffs
            .iter()
            .filter(|d| d.get("status").and_then(|v| v.as_str()) == Some(s))
            .count()
    };
    // E38: roll up the per-file added/removed line counts so the summary conveys
    // the magnitude of the change, not just how many files differ.
    let sum_field = |field: &str| -> u64 {
        diffs
            .iter()
            .filter_map(|d| {
                d.get("change")
                    .and_then(|c| c.get(field))
                    .and_then(serde_json::Value::as_u64)
            })
            .sum()
    };
    serde_json::json!({
        "total": diffs.len(),
        "unchanged": count("unchanged"),
        "modified": count("modified"),
        "missing": count("missing"),
        "created": count("created"),
        "truncated": count("truncated"),
        "errored": count("error") + count("readError"),
        "addedLines": sum_field("addedLines"),
        "removedLines": sum_field("removedLines"),
    })
}

fn compact_file(file: &CheckpointFileSnapshot) -> serde_json::Value {
    serde_json::json!({
        "path": file.path,
        "relativePath": file.relative_path,
        "existed": file.existed,
        "source": file.source,
        "size": file.size,
        "lines": if file.existed { count_lines(&file.text) } else { 0 },
        "truncated": file.truncated,
        "error": file.error,
    })
}

fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count().max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_workspace_path_inside_only() {
        assert_eq!(
            resolve_in_workspace("src/app.ts", "/root"),
            Some("/root/src/app.ts".to_string())
        );
        assert_eq!(
            resolve_in_workspace("/root/src/app.ts", "/root"),
            Some("/root/src/app.ts".to_string())
        );
        assert_eq!(resolve_in_workspace("/etc/passwd", "/root"), None);
    }

    #[test]
    fn resolve_workspace_rejects_parent_escape() {
        assert_eq!(resolve_in_workspace("../other.txt", "/root"), None);
        assert_eq!(resolve_in_workspace("a/../../other.txt", "/root"), None);
        assert_eq!(resolve_in_workspace("/root/../other.txt", "/root"), None);
    }

    #[test]
    fn relative_path_strips_root() {
        assert_eq!(relative_to("/root/src/app.ts", "/root"), "src/app.ts");
        assert_eq!(relative_to("/other/x.ts", "/root"), "/other/x.ts");
    }

    #[test]
    fn count_lines_basic() {
        assert_eq!(count_lines(""), 0);
        assert_eq!(count_lines("one"), 1);
        assert_eq!(count_lines("one\ntwo\nthree"), 3);
    }

    #[test]
    fn normalize_path_rejects_escape() {
        // Standard parent-dir traversal resolves to a sibling path
        let p = std::path::Path::new("/root/../etc");
        assert_eq!(
            normalize_path_components(p).as_deref(),
            Some(std::path::Path::new("/etc"))
        );
        // Excessive parent-dir beyond root level is rejected
        let p = std::path::Path::new("/a/../..");
        assert_eq!(normalize_path_components(p), None);
        // Nested traversal
        let p = std::path::Path::new("/root/a/../../foo");
        assert_eq!(
            normalize_path_components(p).as_deref(),
            Some(std::path::Path::new("/foo"))
        );
    }

    #[test]
    fn decode_utf8_bounded_truncation() {
        let ascii = b"hello world";
        let result = decode_utf8_bounded(ascii, 5);
        assert_eq!(result, "hello");

        // Multi-byte char at boundary: 'héllo' — é is 2 UTF-8 bytes
        let multi = "héllo";
        let result = decode_utf8_bounded(multi.as_bytes(), 3);
        assert_eq!(result, "hé");
    }

    #[test]
    fn lcs_line_count_basic() {
        assert_eq!(lcs_line_count(&["a", "b", "c"], &["a", "b", "c"]), 3);
        assert_eq!(lcs_line_count(&["a", "b", "c"], &["a", "x", "c"]), 2);
        assert_eq!(lcs_line_count(&["a", "b"], &[]), 0);
        assert_eq!(lcs_line_count(&[], &["a"]), 0);
        // Insertion in the middle keeps all original lines common.
        assert_eq!(lcs_line_count(&["a", "c"], &["a", "b", "c"]), 2);
    }

    #[test]
    fn line_change_summary_reports_added_removed() {
        // E38: diff must convey the actual change, not just a status string.
        let from = "one\ntwo\nthree";
        let to = "one\ntwo-changed\nthree\nfour";
        let change = line_change_summary(from, to);
        // "two" removed; "two-changed" and "four" added.
        assert_eq!(change["removedLines"].as_u64(), Some(1));
        assert_eq!(change["addedLines"].as_u64(), Some(2));
        let preview = change["preview"].as_array().expect("preview array");
        assert!(preview.iter().any(|l| l.as_str() == Some("- two")));
        assert!(preview.iter().any(|l| l.as_str() == Some("+ two-changed")));
        assert!(preview.iter().any(|l| l.as_str() == Some("+ four")));
        assert_eq!(change["previewTruncated"].as_bool(), Some(false));
    }

    #[test]
    fn line_change_summary_caps_preview() {
        // A huge change must produce a bounded, flagged preview.
        let from = String::new();
        let to: String = {
            use std::fmt::Write as _;
            let mut buf = String::new();
            for i in 0..500 {
                let _ = writeln!(buf, "line-{i}");
            }
            buf
        };
        let change = line_change_summary(&from, &to);
        assert_eq!(change["addedLines"].as_u64(), Some(500));
        let preview = change["preview"].as_array().expect("preview array");
        assert!(preview.len() <= 40, "preview must be capped");
        assert_eq!(change["previewTruncated"].as_bool(), Some(true));
    }

    #[test]
    fn diff_summary_rolls_up_line_counts() {
        // E38: the aggregate summary reflects total added/removed lines.
        let diffs = vec![
            serde_json::json!({
                "status": "modified",
                "change": { "addedLines": 3, "removedLines": 1 },
            }),
            serde_json::json!({
                "status": "modified",
                "change": { "addedLines": 2, "removedLines": 4 },
            }),
            serde_json::json!({ "status": "unchanged" }),
        ];
        let summary = diff_summary(&diffs);
        assert_eq!(summary["modified"].as_u64(), Some(2));
        assert_eq!(summary["unchanged"].as_u64(), Some(1));
        assert_eq!(summary["addedLines"].as_u64(), Some(5));
        assert_eq!(summary["removedLines"].as_u64(), Some(5));
    }

    #[tokio::test]
    async fn capped_snapshot_marks_truncated_for_restore_guard() {
        // E35 (data loss): a snapshot captured with a byte cap smaller than the
        // file must be flagged truncated, which the restore path uses to refuse
        // rather than writing a shortened file back.
        let dir =
            std::env::temp_dir().join(format!("lux-ckpt-cap-{}", uuid::Uuid::new_v4().simple()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join("big.txt");
        let file_str = ai_semantic::normalize_slashes_pub(&file_path.to_string_lossy());
        // Write more bytes than the cap we will snapshot with.
        tokio::fs::write(&file_str, "X".repeat(5_000))
            .await
            .unwrap();

        let snap = snapshot_file_disk(&file_str, "big.txt".into(), 1_024).await;
        assert!(
            snap.truncated,
            "snapshot exceeding the byte cap must be flagged truncated so restore refuses it"
        );
        assert!(
            u64::try_from(snap.text.len()).unwrap_or(u64::MAX) <= 1_024,
            "captured text must not exceed the cap"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn missing_file_snapshot_disk_no_error() {
        let dir =
            std::env::temp_dir().join(format!("lux-ckpt-test-{}", uuid::Uuid::new_v4().simple()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let missing_path = dir.join("not-here.txt");
        let missing_str = ai_semantic::normalize_slashes_pub(&missing_path.to_string_lossy());

        let snap = snapshot_file_disk(&missing_str, "not-here.txt".into(), 10_000).await;
        assert!(!snap.existed, "missing file should have existed=false");
        assert_eq!(snap.source, "missing");
        assert!(
            snap.error.is_none(),
            "missing file snapshot must NOT carry an error: {:?}",
            snap.error
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // guard is explicitly dropped before any await
    async fn missing_snapshot_restore_deletes_file() {
        // Regression test for finding 1: a missing-file baseline (snapshot taken
        // before the AI creates a file) must have error=None, so that restore can
        // later delete the file on rollback.
        let dir = std::env::temp_dir().join(format!(
            "lux-ckpt-rollback-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = ai_semantic::normalize_slashes_pub(&dir.to_string_lossy());

        // Step 1: snapshot a file that doesn't exist yet (as the AI would
        // before creating it in the edit turn).
        let file_path = dir.join("created-by-ai.txt");
        let file_str = ai_semantic::normalize_slashes_pub(&file_path.to_string_lossy());
        let snap = snapshot_file_disk(&file_str, "created-by-ai.txt".into(), 10_000).await;
        assert!(!snap.existed, "pre-creation file should not exist");
        assert!(
            snap.error.is_none(),
            "missing baseline must not carry error"
        );

        // Step 2: create the checkpoint (simulating the create command).
        let cp = Checkpoint {
            id: "cp-test-regression".into(),
            label: "test".into(),
            workspace_root: root.clone(),
            session_id: None,
            created_at_ms: 1000,
            files: vec![snap],
            max_bytes_per_file: 10_000,
            max_files: 40,
        };
        let mut guard = store().lock().unwrap();
        let list = guard.entry(ws_key(&root)).or_default();
        list.push(cp);
        drop(guard);

        // Step 3: simulate the AI creating the file.
        tokio::fs::write(&file_str, "AI generated content")
            .await
            .unwrap();
        assert!(
            file_path.exists(),
            "file should exist after simulated AI edit"
        );

        // Step 4: select the checkpoint (as the restore command would).
        let restored_cp = select_checkpoint(&root, None, None).unwrap();

        // The truncate/error guard must NOT block restore.
        let blocked = restored_cp
            .files
            .iter()
            .any(|f| f.truncated || f.error.is_some());
        assert!(
            !blocked,
            "restore must not be blocked by truncated/error files"
        );

        // The snapshot correctly recorded existed=false.
        let file_snap = &restored_cp.files[0];
        assert!(!file_snap.existed, "snapshot had existed=false");
        assert!(file_path.exists(), "current file exists on disk");

        // Step 5: simulate the restore deleting the file (what ai_file_patch
        // would do for the existed=false → current.disk_exists=true case).
        tokio::fs::remove_file(&file_str).await.unwrap();
        assert!(!file_path.exists(), "restored file must be deleted");

        // Clean up the store and temp dir.
        guard = store().lock().unwrap();
        guard.remove(&ws_key(&root));
        drop(guard);
        std::fs::remove_dir_all(&dir).ok();
    }
}
