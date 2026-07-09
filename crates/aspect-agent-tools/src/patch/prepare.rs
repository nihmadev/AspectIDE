use std::path::PathBuf;
use std::collections::HashSet;

use crate::types::file_patch::*;
use crate::types::file_result::AiFileOperationStats;
use crate::diff_stats::diff_stats;
use crate::eol::{detect_eol, normalize_eol};

pub fn classify_patch_action(action: &str) -> Result<AiPreparedPatchKind, String> {
    match action {
        "create" => Ok(AiPreparedPatchKind::Create),
        "write" | "rewrite" | "replacefile" | "replace_file" => Ok(AiPreparedPatchKind::Rewrite),
        "strreplace" | "str_replace" | "replace" => Ok(AiPreparedPatchKind::Replace),
        "delete" | "remove" => Ok(AiPreparedPatchKind::Delete),
        _ => Err(format!("unsupported patch action: {action}")),
    }
}

pub fn prepare_patch_operation(
    op: &AiFilePatchOperation,
    before: Option<&str>,
    overwrite: Option<bool>,
) -> Result<AiPreparedPatchOperation, String> {
    let action = op.action.trim().to_ascii_lowercase();
    let kind = classify_patch_action(&action)?;

    match kind {
        AiPreparedPatchKind::Create | AiPreparedPatchKind::Rewrite => {
            let text = op.text.clone().ok_or_else(|| {
                format!("{} requires text for {}", op.action, op.path.display())
            })?;
            if before.is_some() && kind == AiPreparedPatchKind::Create && !overwrite.unwrap_or(false)
            {
                return Err(format!("file already exists: {}", op.path.display()));
            }
            if kind == AiPreparedPatchKind::Rewrite && before.is_none() {
                return Err(format!(
                    "rewrite target does not exist (use create): {}",
                    op.path.display()
                ));
            }
            let stats = diff_stats(before.unwrap_or(""), &text, before.is_none());
            Ok(AiPreparedPatchOperation {
                kind,
                path: op.path.clone(),
                after_text: Some(text),
                stats,
            })
        }
        AiPreparedPatchKind::Replace => {
            let Some(src) = before else {
                return Err(format!(
                    "file does not exist for replacement: {}",
                    op.path.display()
                ));
            };
            let old_text = op.old_text.as_deref().ok_or_else(|| {
                format!("replace requires oldText for {}", op.path.display())
            })?;
            if old_text.is_empty() {
                return Err(format!("oldText must not be empty for {}", op.path.display()));
            }
            let new_text = op.new_text.as_deref().ok_or_else(|| {
                format!("replace requires newText for {}", op.path.display())
            })?;

            let eol = detect_eol(src);
            let old_text = normalize_eol(old_text, eol);
            let new_text = normalize_eol(new_text, eol);

            let replacement_count = src.matches(&old_text).count();
            let expected = op.expected_replacements.unwrap_or(1);
            if replacement_count != expected {
                let new_already = src.matches(&new_text).count();
                let hint = if new_already > 0 && !new_text.is_empty() {
                    format!(
                        " (newText already present {new_already} time(s) - may already be applied)"
                    )
                } else {
                    String::new()
                };
                return Err(format!(
                    "replacement count mismatch for {}: expected {expected}, found {replacement_count}{hint}",
                    op.path.display()
                ));
            }
            let after = src.replacen(&old_text, &new_text, expected);
            let stats = diff_stats(src, &after, false);
            Ok(AiPreparedPatchOperation {
                kind,
                path: op.path.clone(),
                after_text: Some(after),
                stats,
            })
        }
        AiPreparedPatchKind::Delete => {
            if before.is_none() {
                return Err(format!(
                    "file does not exist for deletion: {}",
                    op.path.display()
                ));
            }
            let lines = before.map(|s| s.lines().count()).unwrap_or(0);
            let stats = AiFileOperationStats {
                lines_added: 0,
                lines_removed: lines,
                files_changed: 0,
                files_created: 0,
                files_deleted: 1,
            };
            Ok(AiPreparedPatchOperation {
                kind,
                path: op.path.clone(),
                after_text: None,
                stats,
            })
        }
    }
}

pub fn unique_patch_paths(operations: &[AiPreparedPatchOperation]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    operations
        .iter()
        .filter(|op| seen.insert(op.path.clone()))
        .map(|op| op.path.clone())
        .collect()
}
