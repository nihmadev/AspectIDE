use std::{collections::BTreeMap, path::Path};

use lux_core::{
    BufferId, DocumentSnapshot, LanguageServerInfo, LspCodeAction, LspCodeActionDiagnostic,
    LspCodeActionTrigger, LspCompletionList, LspDocumentSymbol, LspFoldingRange,
    LspFormattingOptions, LspHover, LspInlayHint, LspLocation, LspRange, LspSemanticTokens,
    LspSignatureHelp, LspTextEdit, LuxEvent, TextEdit, WorkspaceDiagnostic,
};
use tauri::{AppHandle, State};

use super::{emit_event, lock_error, SharedState};

#[tauri::command]
pub async fn lsp_servers(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<Vec<LanguageServerInfo>, String> {
    let root = state
        .workspace
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .map(|workspace| workspace.root.clone())
        .ok_or_else(|| "no workspace is open".to_string())?;

    // Discovery is fast and bounded; do it inline so the UI gets the server list
    // (available/missing) immediately and can clear its "loading" state.
    let servers = tokio::task::spawn_blocking(move || lux_lsp::workspace_language_servers(root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)?;

    // Publish missing-server warnings right away.
    let missing_diagnostics = lux_lsp::language_server_diagnostics(&servers);
    replace_diagnostics(&app, &state, missing_diagnostics)?;

    // Actually starting servers means running each `initialize` handshake (up to
    // a 10s timeout apiece). Do it OFF the command path so the frontend never
    // blocks on it — startup runs in the background and streams diagnostics +
    // forwards open documents as servers come online. This is what keeps the
    // language service from "hanging on load".
    let background_state = state.inner().clone();
    let background_app = app.clone();
    let background_servers = servers.clone();
    tokio::spawn(async move {
        start_servers_background(&background_app, &background_state, &background_servers).await;
    });

    Ok(servers)
}

/// Background startup: start available servers in parallel, merge any startup
/// failures into the published diagnostics, then forward already-open documents
/// to the freshly started servers. Runs detached from the `lsp_servers` command
/// so the UI is never blocked on LSP `initialize` handshakes.
async fn start_servers_background(
    app: &AppHandle,
    state: &SharedState,
    servers: &[LanguageServerInfo],
) {
    let startup_diagnostics = {
        let mut lsp = state.lsp.lock().await;
        match lsp.as_mut() {
            Some(manager) => manager.start_available_servers(servers).await,
            None => return,
        }
    };

    // Re-publish: keep the missing-server warnings and add any startup failures.
    let mut diagnostics = lux_lsp::language_server_diagnostics(servers);
    diagnostics.extend(startup_diagnostics);
    let _ = replace_diagnostics_owned(app, state, diagnostics);

    let open_documents = {
        let Ok(documents) = state.documents.lock() else {
            return;
        };
        documents.snapshots()
    };
    for document in open_documents {
        forward_document_open_owned(app, state, &document).await;
    }
}

#[tauri::command]
pub fn diagnostics_snapshot(
    state: State<'_, SharedState>,
) -> Result<Vec<WorkspaceDiagnostic>, String> {
    Ok(state.diagnostics.lock().map_err(lock_error)?.clone())
}

#[tauri::command]
pub async fn lsp_hover(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    line: u32,
    column: u32,
) -> Result<Option<LspHover>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(None);
    };
    manager
        .hover(&document, line, column)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn lsp_definition(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    line: u32,
    column: u32,
) -> Result<Vec<LspLocation>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .definition(&document, line, column)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn lsp_references(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    line: u32,
    column: u32,
) -> Result<Vec<LspLocation>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .references(&document, line, column)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn lsp_document_symbols(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
) -> Result<Vec<LspDocumentSymbol>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .document_symbols(&document)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn lsp_workspace_symbols(
    state: State<'_, SharedState>,
    query: String,
) -> Result<Vec<lux_core::LspWorkspaceSymbol>, String> {
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager.workspace_symbols(query).await.map_err(String::from)
}

#[tauri::command]
pub async fn lsp_folding_ranges(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
) -> Result<Vec<LspFoldingRange>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .folding_ranges(&document)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn lsp_inlay_hints(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    range: LspRange,
) -> Result<Vec<LspInlayHint>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .inlay_hints(&document, range)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn lsp_semantic_tokens(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
) -> Result<Option<LspSemanticTokens>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(None);
    };
    manager
        .semantic_tokens(&document)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn lsp_completion(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    line: u32,
    column: u32,
) -> Result<LspCompletionList, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(LspCompletionList {
            is_incomplete: false,
            items: Vec::new(),
        });
    };
    manager
        .completion(&document, line, column)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn lsp_code_actions(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    range: LspRange,
    diagnostics: Vec<LspCodeActionDiagnostic>,
    only: Option<Vec<String>>,
    trigger: LspCodeActionTrigger,
) -> Result<Vec<LspCodeAction>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .code_actions(&document, range, diagnostics, only, trigger)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn lsp_format_document(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    options: LspFormattingOptions,
) -> Result<Vec<LspTextEdit>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .format_document(&document, options)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn lsp_format_range(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    range: LspRange,
    options: LspFormattingOptions,
) -> Result<Vec<LspTextEdit>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(Vec::new());
    };
    manager
        .format_range(&document, range, options)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn lsp_signature_help(
    state: State<'_, SharedState>,
    buffer_id: BufferId,
    line: u32,
    column: u32,
) -> Result<Option<LspSignatureHelp>, String> {
    let document = state
        .documents
        .lock()
        .map_err(lock_error)?
        .snapshot(buffer_id)
        .map_err(String::from)?;
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(None);
    };
    manager
        .signature_help(&document, line, column)
        .await
        .map_err(String::from)
}

pub fn replace_diagnostics(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    diagnostics: Vec<WorkspaceDiagnostic>,
) -> Result<(), String> {
    let mut by_path: BTreeMap<_, Vec<WorkspaceDiagnostic>> = BTreeMap::new();
    for diagnostic in &diagnostics {
        by_path
            .entry(diagnostic.path.clone())
            .or_default()
            .push(diagnostic.clone());
    }
    let previous_paths = {
        let mut current = state.diagnostics.lock().map_err(lock_error)?;
        let paths = current
            .iter()
            .map(|diagnostic| diagnostic.path.clone())
            .collect::<Vec<_>>();
        *current = diagnostics;
        paths
    };

    for path in previous_paths {
        by_path.entry(path).or_default();
    }

    for (path, path_diagnostics) in by_path {
        emit_event(
            app,
            LuxEvent::EditorDiagnosticsChanged {
                path,
                diagnostics: path_diagnostics,
            },
        )?;
    }

    Ok(())
}

/// Owned-state variant of [`replace_diagnostics`] for use from detached
/// background tasks (which hold an `Arc<AppState>`, not a borrowed `State`).
fn replace_diagnostics_owned(
    app: &AppHandle,
    state: &SharedState,
    diagnostics: Vec<WorkspaceDiagnostic>,
) -> Result<(), String> {
    let mut by_path: BTreeMap<_, Vec<WorkspaceDiagnostic>> = BTreeMap::new();
    for diagnostic in &diagnostics {
        by_path
            .entry(diagnostic.path.clone())
            .or_default()
            .push(diagnostic.clone());
    }
    let previous_paths = {
        let mut current = state.diagnostics.lock().map_err(lock_error)?;
        let paths = current
            .iter()
            .map(|diagnostic| diagnostic.path.clone())
            .collect::<Vec<_>>();
        *current = diagnostics;
        paths
    };

    for path in previous_paths {
        by_path.entry(path).or_default();
    }

    for (path, path_diagnostics) in by_path {
        emit_event(
            app,
            LuxEvent::EditorDiagnosticsChanged {
                path,
                diagnostics: path_diagnostics,
            },
        )?;
    }

    Ok(())
}

pub fn clear_diagnostics(app: &AppHandle, state: &State<'_, SharedState>) -> Result<(), String> {
    let previous_paths = {
        let mut current = state.diagnostics.lock().map_err(lock_error)?;
        let paths = current
            .iter()
            .map(|diagnostic| diagnostic.path.clone())
            .collect::<Vec<_>>();
        current.clear();
        paths
    };

    for path in previous_paths {
        emit_event(
            app,
            LuxEvent::EditorDiagnosticsChanged {
                path,
                diagnostics: Vec::new(),
            },
        )?;
    }

    Ok(())
}

pub fn apply_diagnostics_update(
    app: &AppHandle,
    state: &SharedState,
    update: lux_lsp::DiagnosticsUpdate,
) -> Result<(), String> {
    {
        let mut current = state.diagnostics.lock().map_err(lock_error)?;
        current.retain(|diagnostic| diagnostic.path != update.path);
        current.extend(update.diagnostics.clone());
    }

    emit_event(
        app,
        LuxEvent::EditorDiagnosticsChanged {
            path: update.path,
            diagnostics: update.diagnostics,
        },
    )
}

pub async fn shutdown(state: &State<'_, SharedState>) {
    let mut lsp = state.lsp.lock().await;
    if let Some(manager) = lsp.as_mut() {
        manager.shutdown_all().await;
    }
}

pub async fn forward_document_open(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    document: &DocumentSnapshot,
) -> Result<(), String> {
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(());
    };
    if let Err(error) = manager.open_document(document).await {
        publish_forwarding_error(app, state, document, error)?;
    }
    Ok(())
}

/// Owned-state variant of [`forward_document_open`] for background startup.
/// Errors are surfaced as a diagnostic rather than propagated (the detached task
/// has nowhere to return them).
async fn forward_document_open_owned(
    app: &AppHandle,
    state: &SharedState,
    document: &DocumentSnapshot,
) {
    let result = {
        let mut lsp = state.lsp.lock().await;
        let Some(manager) = lsp.as_mut() else {
            return;
        };
        manager.open_document(document).await
    };
    if let (Err(error), Some(path)) = (result, document.path.clone()) {
        let _ = apply_diagnostics_update(
            app,
            state,
            lux_lsp::DiagnosticsUpdate {
                path: path.clone(),
                diagnostics: vec![WorkspaceDiagnostic {
                    path,
                    line: 1,
                    column: 1,
                    severity: lux_core::DiagnosticSeverity::Warning,
                    source: "lux-lsp".to_string(),
                    message: format!("Language server stopped: {error}"),
                }],
            },
        );
    }
}

pub async fn forward_document_update(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    document: &DocumentSnapshot,
) -> Result<(), String> {
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(());
    };
    if let Err(error) = manager.update_document(document).await {
        publish_forwarding_error(app, state, document, error)?;
    }
    Ok(())
}

pub async fn forward_document_edits(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    document: &DocumentSnapshot,
    edits: &[TextEdit],
) -> Result<(), String> {
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(());
    };
    if let Err(error) = manager.apply_document_edits(document, edits).await {
        publish_forwarding_error(app, state, document, error)?;
    }
    Ok(())
}

pub async fn forward_document_save(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    document: &DocumentSnapshot,
) -> Result<(), String> {
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(());
    };
    if let Err(error) = manager.save_document(document).await {
        publish_forwarding_error(app, state, document, error)?;
    }
    Ok(())
}

pub async fn forward_document_close(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    path: &Path,
) -> Result<(), String> {
    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        return Ok(());
    };
    if let Err(error) = manager.close_document(path).await {
        apply_diagnostics_update(
            app,
            state.inner(),
            lux_lsp::DiagnosticsUpdate {
                path: path.to_path_buf(),
                diagnostics: vec![WorkspaceDiagnostic {
                    path: path.to_path_buf(),
                    line: 1,
                    column: 1,
                    severity: lux_core::DiagnosticSeverity::Warning,
                    source: "lux-lsp".to_string(),
                    message: format!("Language server stopped: {error}"),
                }],
            },
        )?;
    }
    Ok(())
}

fn publish_forwarding_error(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    document: &DocumentSnapshot,
    error: lux_core::AppError,
) -> Result<(), String> {
    let Some(path) = document.path.clone() else {
        return Ok(());
    };
    apply_diagnostics_update(
        app,
        state.inner(),
        lux_lsp::DiagnosticsUpdate {
            path: path.clone(),
            diagnostics: vec![WorkspaceDiagnostic {
                path,
                line: 1,
                column: 1,
                severity: lux_core::DiagnosticSeverity::Warning,
                source: "lux-lsp".to_string(),
                message: format!("Language server stopped: {error}"),
            }],
        },
    )
}
