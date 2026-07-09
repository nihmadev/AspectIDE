use aspect_core::{AspectEvent, DocumentSnapshot};
use aspect_lsp::DiagnosticsUpdate;
use tauri::{AppHandle, State};

use crate::{emit_event, lock_error, lsp, SharedState};
use aspect_agent_tools::{
    diff_stats::combine_patch_stats,
    patch::{apply_patch_to_disk, rollback_patch, prepare::unique_patch_paths},
    types::{AiFileOperationResult, AiPreparedPatchKind},
};

use super::common::prepare_ai_patch_operations;

#[tauri::command]
pub async fn ai_file_patch(
    app: AppHandle,
    state: State<'_, SharedState>,
    operations: Vec<super::AiFilePatchOperation>,
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
    let stats = combine_patch_stats(
        &prepared.iter().map(|op| op.stats.clone()).collect::<Vec<_>>(),
    );
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
    let write_result = apply_patch_to_disk(&prepared, save_to_disk, &mut rollback).await;
    if let Err(error) = write_result {
        rollback_patch(rollback).await;
        return Err(format!(
            "{error} - all changes were rolled back, no files modified"
        ));
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
        rollback_patch(rollback).await;
        return Err(error);
    }

    for document in &closed_documents {
        if let Err(e) = emit_event(
            &app,
            AspectEvent::EditorDocumentClosed {
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
            AspectEvent::EditorDocumentChanged {
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
            if let Err(e) = emit_event(&app, AspectEvent::FsChanged { path: path.clone() }) {
                tracing::warn!(%e, "ai_file_patch: emit_event(FsChanged) failed (non-fatal)");
            }
        }
        if prepared.iter().any(|operation| {
            operation.path == *path && operation.kind == AiPreparedPatchKind::Delete
        }) {
            let _ = lsp::apply_diagnostics_update(
                &app,
                state.inner(),
                DiagnosticsUpdate {
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


