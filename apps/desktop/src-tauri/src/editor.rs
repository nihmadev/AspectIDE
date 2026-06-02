use std::path::PathBuf;

use lux_core::{
    file_view_descriptor_for_path, BufferId, DocumentEditResult, DocumentSnapshot, FileOpenMode,
    LspWorkspaceEdit, LuxEvent, TextEdit, WorkspaceEditResult,
};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use super::{emit_event, lock_error, lsp, SharedState};

#[tauri::command]
pub async fn editor_open_file(
    app: AppHandle,
    state: State<'_, SharedState>,
    path: PathBuf,
) -> Result<DocumentSnapshot, String> {
    let canonical = tokio::task::spawn_blocking(move || path.canonicalize())
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;

    let existing = {
        let documents = state.documents.lock().map_err(lock_error)?;
        documents
            .snapshot_for_path(&canonical)
            .map_err(String::from)?
    };
    if let Some(document) = existing {
        emit_event(
            &app,
            LuxEvent::EditorDocumentChanged {
                document: document.clone(),
            },
        )?;
        if matches!(
            document.view.mode,
            FileOpenMode::EditableText | FileOpenMode::ReadOnlyText
        ) {
            lsp::forward_document_open(&app, &state, &document).await?;
        }
        return Ok(document);
    }

    let view = file_view_descriptor_for_path(&canonical);
    let text = if matches!(
        view.mode,
        FileOpenMode::EditableText | FileOpenMode::ReadOnlyText
    ) {
        let read_path = canonical.clone();
        tokio::task::spawn_blocking(move || std::fs::read_to_string(read_path))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?
    } else {
        String::new()
    };

    let document = {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        documents
            .open_loaded_file(&canonical, text)
            .map_err(String::from)?
    };
    emit_event(
        &app,
        LuxEvent::EditorDocumentChanged {
            document: document.clone(),
        },
    )?;
    if matches!(
        view.mode,
        FileOpenMode::EditableText | FileOpenMode::ReadOnlyText
    ) {
        lsp::forward_document_open(&app, &state, &document).await?;
    }
    Ok(document)
}

#[tauri::command]
pub async fn editor_new_file(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<DocumentSnapshot, String> {
    let document = {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        documents.new_untitled()
    };
    emit_event(
        &app,
        LuxEvent::EditorDocumentChanged {
            document: document.clone(),
        },
    )?;
    Ok(document)
}

#[tauri::command]
pub async fn editor_update_text(
    app: AppHandle,
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    text: String,
) -> Result<DocumentSnapshot, String> {
    let document = {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        documents
            .update_text(buffer_id, text)
            .map_err(String::from)?
    };
    emit_event(
        &app,
        LuxEvent::EditorDocumentChanged {
            document: document.clone(),
        },
    )?;
    lsp::forward_document_update(&app, &state, &document).await?;
    Ok(document)
}

#[tauri::command]
pub async fn editor_apply_edits(
    app: AppHandle,
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    edits: Vec<TextEdit>,
) -> Result<DocumentEditResult, String> {
    let document = {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        documents
            .apply_edits(buffer_id, &edits)
            .map_err(String::from)?
    };
    let result = DocumentEditResult::from(&document);
    emit_event(
        &app,
        LuxEvent::EditorDocumentEdited {
            document: result.clone(),
        },
    )?;
    lsp::forward_document_edits(&app, &state, &document, &edits).await?;
    Ok(result)
}

#[tauri::command]
pub async fn editor_apply_workspace_edit(
    app: AppHandle,
    state: State<'_, SharedState>,
    edit: LspWorkspaceEdit,
) -> Result<WorkspaceEditResult, String> {
    apply_workspace_edit(&app, &state, edit).await
}

#[tauri::command]
pub async fn editor_save_file(
    app: AppHandle,
    state: State<'_, SharedState>,
    buffer_id: BufferId,
) -> Result<DocumentSnapshot, String> {
    let payload = {
        let documents = state.documents.lock().map_err(lock_error)?;
        documents.save_payload(buffer_id).map_err(String::from)?
    };

    let save_path = match payload.path.clone() {
        Some(path) => path,
        None => pick_save_path(&app, &payload.suggested_name)
            .await?
            .ok_or_else(|| "save cancelled".to_string())?,
    };
    let attach_path = payload.is_untitled;
    save_document_to_path(app, state, buffer_id, payload, save_path, attach_path).await
}

#[tauri::command]
pub async fn editor_save_file_as(
    app: AppHandle,
    state: State<'_, SharedState>,
    buffer_id: BufferId,
) -> Result<DocumentSnapshot, String> {
    let payload = {
        let documents = state.documents.lock().map_err(lock_error)?;
        documents.save_payload(buffer_id).map_err(String::from)?
    };

    let save_path = pick_save_path(&app, &payload.suggested_name)
        .await?
        .ok_or_else(|| "save cancelled".to_string())?;
    save_document_to_path(app, state, buffer_id, payload, save_path, true).await
}

pub async fn apply_workspace_edit(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    edit: LspWorkspaceEdit,
) -> Result<WorkspaceEditResult, String> {
    let mut edited_documents = Vec::new();
    let mut changed_paths = Vec::new();

    for file_edit in edit.files {
        let path = file_edit.path;
        let text_edits = file_edit
            .edits
            .into_iter()
            .map(|edit| TextEdit {
                start_line: edit.range.start_line,
                start_column: edit.range.start_column,
                end_line: edit.range.end_line,
                end_column: edit.range.end_column,
                text: edit.text,
            })
            .collect::<Vec<_>>();
        if text_edits.is_empty() {
            continue;
        }

        let edited_open_document = {
            let mut documents = state.documents.lock().map_err(lock_error)?;
            documents
                .apply_edits_for_path(&path, &text_edits)
                .map_err(String::from)?
        };

        if let Some(document) = edited_open_document {
            lsp::forward_document_update(app, state, &document).await?;
            edited_documents.push(document);
            changed_paths.push(path);
            continue;
        }

        let write_path = path.clone();
        let text_edits_for_file = text_edits.clone();
        tokio::task::spawn_blocking(move || -> lux_core::AppResult<()> {
            let mut text = std::fs::read_to_string(&write_path)?;
            lux_editor::apply_text_edits(&mut text, &text_edits_for_file)?;
            std::fs::write(&write_path, text)?;
            Ok(())
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)?;
        emit_event(app, LuxEvent::FsChanged { path: path.clone() })?;
        changed_paths.push(path);
    }

    if !edited_documents.is_empty() {
        emit_event(
            app,
            LuxEvent::EditorDocumentsChanged {
                documents: edited_documents.clone(),
            },
        )?;
    }

    Ok(WorkspaceEditResult {
        edited_documents,
        changed_paths,
    })
}

async fn save_document_to_path(
    app: AppHandle,
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    payload: lux_editor::DocumentSavePayload,
    save_path: PathBuf,
    attach_path: bool,
) -> Result<DocumentSnapshot, String> {
    let saved_version = payload.version;
    let save_path = if attach_path {
        let documents = state.documents.lock().map_err(lock_error)?;
        documents
            .validate_attach_path(buffer_id, &save_path)
            .map_err(String::from)?
    } else {
        save_path
    };

    let write_path = save_path.clone();
    tokio::task::spawn_blocking(move || std::fs::write(write_path, payload.text))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;

    let path_attachment = if attach_path {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        Some(
            documents
                .attach_path_with_previous(buffer_id, save_path.clone())
                .map_err(String::from)?,
        )
    } else {
        None
    };

    let (document, saved_current_version) = {
        let mut documents = state.documents.lock().map_err(lock_error)?;
        documents
            .finish_save(buffer_id, saved_version)
            .map_err(String::from)?
    };
    emit_event(
        &app,
        LuxEvent::EditorDocumentChanged {
            document: document.clone(),
        },
    )?;
    if let Some(attachment) = &path_attachment {
        if let Some(previous_path) = &attachment.previous_path {
            lsp::forward_document_close(&app, &state, previous_path).await?;
            lsp::apply_diagnostics_update(
                &app,
                state.inner(),
                lux_lsp::DiagnosticsUpdate {
                    path: previous_path.clone(),
                    diagnostics: Vec::new(),
                },
            )?;
            emit_event(
                &app,
                LuxEvent::FsChanged {
                    path: previous_path.clone(),
                },
            )?;
        }
        emit_event(
            &app,
            LuxEvent::FsChanged {
                path: save_path.clone(),
            },
        )?;
    }
    if saved_current_version {
        if path_attachment.is_some() {
            lsp::forward_document_open(&app, &state, &document).await?;
        }
        lsp::forward_document_save(&app, &state, &document).await?;
    }
    Ok(document)
}

async fn pick_save_path(app: &AppHandle, suggested_name: &str) -> Result<Option<PathBuf>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Save File")
        .set_file_name(suggested_name)
        .save_file(move |file| {
            let _ = sender.send(file);
        });

    let Some(file) = receiver.await.map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    file.into_path()
        .map(Some)
        .map_err(|error| error.to_string())
}
