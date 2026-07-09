use std::{
    collections::BTreeMap,
    path::Path,
};

use chrono::Utc;
use aspect_core::{file_view_descriptor_for_path, BufferId, DocumentSnapshot, AspectEvent};
use tauri::{AppHandle, State};

use crate::{emit_event, lock_error, lsp, resolve_workspace_path_for_write, SharedState};
use aspect_agent_tools::patch::prepare::classify_patch_action;

use super::AiFilePatchOperation;
pub use aspect_agent_tools::types::AiPreparedPatchOperation;

pub async fn current_text_for_path(
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

pub async fn prepare_ai_patch_operations(
    state: &State<'_, SharedState>,
    operations: Vec<AiFilePatchOperation>,
) -> Result<Vec<AiPreparedPatchOperation>, String> {
    let mut prepared = Vec::with_capacity(operations.len());
    let mut next_text_by_path = BTreeMap::<std::path::PathBuf, Option<String>>::new();

    for (op_index, operation) in operations.into_iter().enumerate() {
        let action_label = operation.action.trim().to_ascii_lowercase();
        let prepared_op = prepare_one_patch_operation(state, operation, &mut next_text_by_path)
            .await
            .map_err(|error| patch_operation_error(op_index, &action_label, &error))?;
        prepared.push(prepared_op);
    }

    Ok(prepared)
}

pub fn patch_operation_error(op_index: usize, action: &str, error: &str) -> String {
    format!(
        "operation[{op_index}] ({action}): {error} - nothing was applied (PatchEngine is all-or-nothing); fix this operation and re-send, or make independent edits with StrReplace"
    )
}

pub async fn prepare_one_patch_operation(
    state: &State<'_, SharedState>,
    operation: AiFilePatchOperation,
    next_text_by_path: &mut BTreeMap<std::path::PathBuf, Option<String>>,
) -> Result<AiPreparedPatchOperation, String> {
    let action = operation.action.trim().to_ascii_lowercase();
    let _kind = classify_patch_action(&action)?;
    let path = resolve_workspace_path_for_write(state, &operation.path)?;

    let before_text = if let Some(previous) = next_text_by_path.get(&path) {
        previous.clone()
    } else if path.exists() {
        Some(current_text_for_path(state, &path).await?)
    } else {
        None
    };

    let op = aspect_agent_tools::patch::prepare::prepare_patch_operation(
        &AiFilePatchOperation {
            action: operation.action.clone(),
            path: path.clone(),
            text: operation.text.clone(),
            old_text: operation.old_text.clone(),
            new_text: operation.new_text.clone(),
            expected_replacements: operation.expected_replacements,
            overwrite: operation.overwrite,
        },
        before_text.as_deref(),
        operation.overwrite,
    )?;

    next_text_by_path.insert(path.clone(), op.after_text.clone());
    Ok(op)
}

pub async fn symbol_context_document_for_path(
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
        language_id: aspect_editor::language_id_for_path(path),
        text,
        view: file_view_descriptor_for_path(path),
        version: 1,
        is_dirty: false,
        is_untitled: false,
        opened_at: Utc::now(),
    })
}

pub async fn update_open_document_after_text_change(
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
        if let Err(e) = emit_event(
            app,
            AspectEvent::EditorDocumentChanged {
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
