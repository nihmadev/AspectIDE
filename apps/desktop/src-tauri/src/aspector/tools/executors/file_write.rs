use std::path::PathBuf;

use aspect_core::AspectEvent;
use tauri::{AppHandle, State};

use crate::{emit_event, lock_error, lsp, resolve_workspace_path_for_write, SharedState};
use aspect_agent_tools::{
    atomic_write::ai_atomic_write,
    diff_stats::diff_stats,
    types::AiFileOperationResult,
};

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
        if let Err(e) = emit_event(&app, AspectEvent::FsChanged { path: path.clone() }) {
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
        if let Err(e) = emit_event(
            &app,
            AspectEvent::EditorDocumentChanged {
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
            if let Err(e) = emit_event(
                &app,
                AspectEvent::EditorDocumentChanged {
                    document: document.clone(),
                },
            ) {
                tracing::warn!(%e, "ai_file_write(staged): emit_event failed (non-fatal)");
            }
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
