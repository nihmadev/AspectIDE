use std::path::PathBuf;

use aspect_core::AspectEvent;
use tauri::{AppHandle, State};

use crate::{emit_event, resolve_workspace_path_for_write, SharedState};
use aspect_agent_tools::{
    atomic_write::ai_atomic_write,
    diff_stats::diff_stats,
    eol::{detect_eol, normalize_eol},
    types::{AiFileOperationResult, AiFileOperationStats},
};

use super::common::update_open_document_after_text_change;

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
    let path = resolve_workspace_path_for_write(&state, &path)?;
    let before = super::common::current_text_for_path(&state, &path).await?;
    let eol = detect_eol(&before);
    let old_text = normalize_eol(&old_text, eol);
    let new_text = normalize_eol(&new_text, eol);
    let replacement_count = before.matches(&old_text).count();
    let expected = expected_replacements.unwrap_or(1);
    if replacement_count != expected {
        let new_already = before.matches(&new_text).count();
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
                    "no change: newText already present {new_already} time(s) - edit appears already applied"
                ),
            });
        }
        let remedy = if replacement_count > expected {
            format!(" - oldText matched {replacement_count} places; pass expectedReplacements:{replacement_count} to replace all, or add surrounding lines to oldText to target one")
        } else if replacement_count == 0 && new_already > 0 && !new_text.is_empty() {
            format!(" - newText is already present {new_already} time(s); it may already be applied")
        } else if replacement_count == 0 {
            " - oldText not found; check exact whitespace/indentation (matched literally, though CRLF/LF differences are tolerated)".to_string()
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
        if let Err(e) = emit_event(&app, AspectEvent::FsChanged { path: path.clone() }) {
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
