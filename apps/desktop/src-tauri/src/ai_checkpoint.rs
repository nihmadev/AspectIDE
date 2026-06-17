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
    pub created_at_ms: i64,
    #[serde(skip)]
    pub files: Vec<CheckpointFileSnapshot>,
    pub max_bytes_per_file: u64,
}

fn store() -> &'static Mutex<HashMap<String, Vec<Checkpoint>>> {
    static STORE: OnceLock<Mutex<HashMap<String, Vec<Checkpoint>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn ws_key(root: &str) -> String {
    ai_semantic::normalize_slashes_pub(root.trim_end_matches('/')).to_lowercase()
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
#[allow(clippy::too_many_arguments)] // Tauri command signature is fixed by the JS invoke contract.
pub async fn ai_checkpoint(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    action: String,
    id: Option<String>,
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
                // Distinguish the two empty cases so the model never mistakes a silent
                // no-op for a working safety net (the "illusion of protection" trap):
                //   - explicit paths given but none resolved → the request FAILED
                //     (bad/outside-root paths); surface it as an error so the agent
                //     fixes the paths instead of trusting a checkpoint that wasn't made.
                //   - no paths and nothing open → genuine nothing-to-snapshot no-op.
                let explicit_count = paths
                    .as_ref()
                    .map_or(0, |p| p.iter().filter(|s| !s.trim().is_empty()).count());
                if explicit_count > 0 {
                    return Err(format!(
                        "Checkpoint not created: none of the {explicit_count} requested path(s) resolved inside the workspace. Pass workspace-relative or in-root absolute file paths."
                    ));
                }
                return Ok(serde_json::json!({
                    "status": "skipped",
                    "reason": "No file paths were available to snapshot (no paths given and no open editor files). Pass an explicit `paths` array.",
                }));
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
                created_at_ms: now_ms,
                files,
                max_bytes_per_file: max_bytes,
            };
            let summary = summarize(&cp);
            let file_views: Vec<serde_json::Value> = cp.files.iter().map(compact_file).collect();
            let mut guard = store().lock().map_err(lock_error)?;
            let list = guard.entry(ws_key(&root_str)).or_default();
            list.insert(0, cp);
            list.truncate(MAX_CHECKPOINTS);
            Ok(
                serde_json::json!({ "status": "created", "checkpoint": summary, "files": file_views }),
            )
        }
        "list" => {
            let guard = store().lock().map_err(lock_error)?;
            let list = guard.get(&ws_key(&root_str)).cloned().unwrap_or_default();
            let summaries: Vec<CheckpointSummary> = list.iter().map(summarize).collect();
            Ok(
                serde_json::json!({ "workspaceRoot": root_str, "count": summaries.len(), "checkpoints": summaries }),
            )
        }
        "diff" => {
            let cp = select_checkpoint(&root_str, id.as_ref())?;
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
            let cp = select_checkpoint(&root_str, id.as_ref())?;
            let mut guard = store().lock().map_err(lock_error)?;
            if let Some(list) = guard.get_mut(&ws_key(&root_str)) {
                list.retain(|c| c.id != cp.id);
            }
            Ok(serde_json::json!({ "status": "deleted", "checkpoint": summarize(&cp) }))
        }
        "restore" => {
            let cp = select_checkpoint(&root_str, id.as_ref())?;
            let save = save_to_disk.unwrap_or(true);
            let dry = dry_run.unwrap_or(false);
            if cp.files.iter().any(|f| f.truncated || f.error.is_some()) {
                return Ok(serde_json::json!({
                    "status": "blocked",
                    "checkpoint": summarize(&cp),
                    "reason": "Restore refused because snapshot files were truncated or unreadable.",
                }));
            }
            // Build restore operations by diffing each file against current state.
            let mut operations: Vec<crate::ai_tools::AiFilePatchOperation> = Vec::new();
            for file in &cp.files {
                let current =
                    read_current(&state, &root_str, &file.path, cp.max_bytes_per_file).await;
                let changed = match (file.existed, current.existed) {
                    (true, true) => file.text != current.text,
                    (true, false) | (false, true) => true, // restore re-creates or deletes
                    (false, false) => false,
                };
                if !changed {
                    continue;
                }
                if file.existed {
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
            if operations.is_empty() {
                return Ok(
                    serde_json::json!({ "status": "unchanged", "checkpoint": summarize(&cp) }),
                );
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
            Ok(
                serde_json::json!({ "status": if dry { "preview" } else { "restored" }, "operations": op_count, "result": result }),
            )
        }
        "augment" => {
            // Capture pre-edit snapshots for files the model is about to create/edit that were not
            // open at turn start and are not already in this checkpoint, so a later restore can
            // revert the edit or delete the newly created file. snapshot_file is async, so we read
            // the checkpoint's byte budget + existing paths under the lock, snapshot without holding
            // it, then re-acquire to push (re-checking each path in case of a concurrent augment).
            let want_id = id
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "Checkpoint augment requires an id.".to_string())?;
            let requested = paths.unwrap_or_default();
            let (max_bytes, existing) = {
                let guard = store().lock().map_err(lock_error)?;
                let cp = guard
                    .get(&ws_key(&root_str))
                    .and_then(|list| list.iter().find(|c| c.id == want_id))
                    .ok_or_else(|| format!("Checkpoint not found: {want_id}"))?;
                let existing: std::collections::HashSet<String> = cp
                    .files
                    .iter()
                    .map(|f| ai_semantic::normalize_slashes_pub(&f.path).to_lowercase())
                    .collect();
                (cp.max_bytes_per_file, existing)
            };
            let mut missing: Vec<String> = Vec::new();
            for raw in &requested {
                if let Some(resolved) = resolve_in_workspace(raw, &root_str) {
                    let key = resolved.to_lowercase();
                    if !existing.contains(&key)
                        && !missing.iter().any(|m: &String| m.to_lowercase() == key)
                    {
                        missing.push(resolved);
                    }
                }
            }
            let mut snapshots = Vec::with_capacity(missing.len());
            for path in &missing {
                snapshots.push(snapshot_file(&state, &root_str, path, max_bytes).await);
            }
            let mut guard = store().lock().map_err(lock_error)?;
            let cp = guard
                .get_mut(&ws_key(&root_str))
                .and_then(|list| list.iter_mut().find(|c| c.id == want_id))
                .ok_or_else(|| format!("Checkpoint not found: {want_id}"))?;
            let mut added = 0usize;
            for snap in snapshots {
                let key = ai_semantic::normalize_slashes_pub(&snap.path).to_lowercase();
                if !cp
                    .files
                    .iter()
                    .any(|f| ai_semantic::normalize_slashes_pub(&f.path).to_lowercase() == key)
                {
                    cp.files.push(snap);
                    added += 1;
                }
            }
            let summary = summarize(cp);
            Ok(serde_json::json!({ "status": "augmented", "added": added, "checkpoint": summary }))
        }
        _ => unreachable!(),
    }
}

fn select_checkpoint(root: &str, id: Option<&String>) -> Result<Checkpoint, String> {
    let guard = store().lock().map_err(lock_error)?;
    let list = guard
        .get(&ws_key(root))
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

fn resolve_in_workspace(path: &str, root: &str) -> Option<String> {
    let normalized = ai_semantic::normalize_slashes_pub(path.trim());
    let resolved = if normalized.starts_with('/') || normalized.chars().nth(1) == Some(':') {
        normalized
    } else {
        format!(
            "{}/{}",
            root.trim_end_matches('/'),
            normalized.trim_start_matches('/')
        )
    };
    let root_lower = root.to_lowercase();
    if resolved.to_lowercase() == root_lower
        || resolved
            .to_lowercase()
            .starts_with(&format!("{root_lower}/"))
    {
        Some(resolved)
    } else {
        None
    }
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

async fn snapshot_file(
    state: &State<'_, SharedState>,
    root: &str,
    path: &str,
    max_bytes: u64,
) -> CheckpointFileSnapshot {
    let relative_path = relative_to(path, root);
    // Editor buffer first.
    if let Some(snap) = editor_snapshot(state, path) {
        let truncated = snap.text.chars().count() as u64 > max_bytes;
        let text: String = snap
            .text
            .chars()
            .take(usize::try_from(max_bytes).unwrap_or(usize::MAX))
            .collect();
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
    match tokio::fs::read_to_string(path).await {
        Ok(text) => {
            let truncated = text.chars().count() as u64 > max_bytes;
            let clamped: String = text
                .chars()
                .take(usize::try_from(max_bytes).unwrap_or(usize::MAX))
                .collect();
            CheckpointFileSnapshot {
                path: path.to_string(),
                relative_path,
                existed: true,
                size: text.len() as u64,
                truncated,
                text: clamped,
                source: "disk".into(),
                error: None,
            }
        }
        Err(err) => CheckpointFileSnapshot {
            path: path.to_string(),
            relative_path,
            existed: false,
            size: 0,
            truncated: false,
            text: String::new(),
            source: "missing".into(),
            error: Some(err.to_string()),
        },
    }
}

struct CurrentFile {
    existed: bool,
    disk_exists: bool,
    text: String,
    truncated: bool,
    source: String,
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
        let truncated = snap.text.chars().count() as u64 > max_bytes;
        return CurrentFile {
            existed: true,
            disk_exists,
            text: snap
                .text
                .chars()
                .take(usize::try_from(max_bytes).unwrap_or(usize::MAX))
                .collect(),
            truncated,
            source: "editor".into(),
        };
    }
    tokio::fs::read_to_string(path).await.map_or_else(
        |_| CurrentFile {
            existed: false,
            disk_exists: false,
            text: String::new(),
            truncated: false,
            source: "missing".into(),
        },
        |text| {
            let truncated = text.chars().count() as u64 > max_bytes;
            CurrentFile {
                existed: true,
                disk_exists: true,
                text: text
                    .chars()
                    .take(usize::try_from(max_bytes).unwrap_or(usize::MAX))
                    .collect(),
                truncated,
                source: "disk".into(),
            }
        },
    )
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
    serde_json::json!({
        "path": file.path,
        "relativePath": file.relative_path,
        "status": status,
        "existedAtCheckpoint": file.existed,
        "currentExists": current.existed,
        "diskExists": current.disk_exists,
        "snapshotSource": file.source,
        "currentSource": current.source,
        "lineDelta": line_delta,
    })
}

fn diff_summary(diffs: &[serde_json::Value]) -> serde_json::Value {
    let count = |s: &str| {
        diffs
            .iter()
            .filter(|d| d.get("status").and_then(|v| v.as_str()) == Some(s))
            .count()
    };
    serde_json::json!({
        "total": diffs.len(),
        "unchanged": count("unchanged"),
        "modified": count("modified"),
        "missing": count("missing"),
        "created": count("created"),
        "truncated": count("truncated"),
        "errored": count("error"),
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
}
