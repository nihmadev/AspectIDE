use std::path::PathBuf;

use aspect_core::AspectEvent;
use aspect_lsp::DiagnosticsUpdate;
use tauri::{AppHandle, State};

use crate::{emit_event, lock_error, lsp, resolve_workspace_path_for_write, SharedState};
use aspect_agent_tools::types::{AiFileOperationResult, AiFileOperationStats};

#[tauri::command]
pub async fn ai_file_delete(
    app: AppHandle,
    state: State<'_, SharedState>,
    path: PathBuf,
) -> Result<AiFileOperationResult, String> {
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
    for document in &closed_documents {
        if let Err(e) = emit_event(
            &app,
            AspectEvent::EditorDocumentClosed {
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
        DiagnosticsUpdate {
            path: path.clone(),
            diagnostics: Vec::new(),
        },
    );
    if let Err(e) = emit_event(&app, AspectEvent::FsChanged { path: path.clone() }) {
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
