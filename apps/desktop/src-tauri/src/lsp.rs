use std::{
    collections::{BTreeMap, BTreeSet},
    panic::AssertUnwindSafe,
    path::Path,
};

use futures_util::FutureExt;
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
    tracing::info!("lsp_servers: discovering language servers");
    let servers = tokio::task::spawn_blocking(move || lux_lsp::workspace_language_servers(root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)?;
    tracing::info!("lsp_servers: discovered {} server(s)", servers.len());

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
    // Use Tauri's async runtime (not bare `tokio::spawn`): a command may run on a
    // thread without an entered tokio reactor, where `tokio::spawn` panics and the
    // command never resolves — leaving the UI stuck on "Starting language services".
    tauri::async_runtime::spawn(async move {
        start_servers_background(&background_app, &background_state, &background_servers).await;
    });

    tracing::info!("lsp_servers: returning server list to frontend");
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
    // Take the manager OUT of the shared slot so the slow `initialize` handshakes
    // (run in parallel, up to ~10s per server) happen WITHOUT holding the `lsp`
    // mutex. Holding it across the whole startup window parked every feature
    // command (hover, completion, definition, …) behind the same lock — the exact
    // "load hang" this background path exists to avoid. While the manager is taken,
    // feature commands observe `None` and return their existing empty default;
    // they start serving the instant it is restored below.
    let mut manager = {
        let mut lsp = state.lsp.lock().await;
        match lsp.take() {
            Some(manager) => manager,
            None => return,
        }
    };

    // Guard the handshake against an unwinding panic deep in `lux_lsp` (e.g. an
    // `unwrap` on a malformed `initialize` response). Without this, a panic here
    // would drop `manager` while the slot stays `None` for the rest of the
    // session — stranding the LSP and orphaning any already-spawned child
    // servers. `catch_unwind` lets us ALWAYS restore the manager below.
    let result = AssertUnwindSafe(manager.start_available_servers(servers))
        .catch_unwind()
        .await;

    // Restore the (possibly-panicked) manager before forwarding documents /
    // serving requests. Unconditional: even on a caught panic the manager is a
    // valid owned value here, and leaving the slot `None` would kill LSP for the
    // session.
    {
        let mut lsp = state.lsp.lock().await;
        *lsp = Some(manager);
    }

    // If startup panicked there are no diagnostics to merge; the manager is
    // restored, so feature commands resume serving whatever the survivors offer.
    let startup_diagnostics = match result {
        Ok(diagnostics) => diagnostics,
        Err(_) => return,
    };

    // Re-publish: keep the missing-server warnings and add any startup failures,
    // WITHOUT clobbering real diagnostics a server may have published via the
    // diagnostics channel during `initialize`.
    let mut status_diagnostics = lux_lsp::language_server_diagnostics(servers);
    status_diagnostics.extend(startup_diagnostics);
    let _ = replace_status_diagnostics_owned(app, state, status_diagnostics);

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

/// Synthetic server-status entries (missing-server warnings + startup failures)
/// all carry this source; real `publishDiagnostics` from a language server never
/// do (they use the server's own source, falling back to `"lsp"`). Used to merge
/// status updates without clobbering real diagnostics delivered concurrently.
const STATUS_DIAGNOSTIC_SOURCE: &str = "lux-lsp";

/// Synthetic per-document forwarding errors ("Language server stopped: …")
/// carry this DISTINCT source so they are never swept up by the status-merge
/// retain in [`replace_status_diagnostics_owned`] (which strips every
/// `STATUS_DIAGNOSTIC_SOURCE` entry on each refresh). Sharing the status source
/// let a later `lsp_servers` refresh silently delete a still-valid forwarding
/// error. Source is only a display label, so this is purely a keying change.
const FORWARDING_DIAGNOSTIC_SOURCE: &str = "lux-lsp-doc";

/// Owned-state variant for detached background tasks (which hold an
/// `Arc<AppState>`, not a borrowed `State`).
///
/// Unlike a wholesale replace, this MERGES: it swaps only the synthetic
/// server-status entries (`source == "lux-lsp"`) and leaves any real per-path
/// diagnostics already delivered by the concurrent diagnostics channel during
/// `initialize` intact. A wholesale replace here produced a flash-then-vanish,
/// dropping eager `publishDiagnostics` that landed mid-startup.
fn replace_status_diagnostics_owned(
    app: &AppHandle,
    state: &SharedState,
    status_diagnostics: Vec<WorkspaceDiagnostic>,
) -> Result<(), String> {
    // Compute the full per-path diagnostic lists for every affected path under a
    // single lock, then emit after releasing it.
    let affected = {
        let mut current = state.diagnostics.lock().map_err(lock_error)?;

        // Paths that previously carried status entries must be re-emitted so any
        // now-resolved status (e.g. a server that started successfully) is cleared.
        let mut affected_paths = current
            .iter()
            .filter(|diagnostic| diagnostic.source == STATUS_DIAGNOSTIC_SOURCE)
            .map(|diagnostic| diagnostic.path.clone())
            .collect::<BTreeSet<_>>();

        // Replace only the status entries; preserve real diagnostics.
        current.retain(|diagnostic| diagnostic.source != STATUS_DIAGNOSTIC_SOURCE);
        for diagnostic in &status_diagnostics {
            affected_paths.insert(diagnostic.path.clone());
        }
        current.extend(status_diagnostics);

        // Snapshot the full (real + status) list for each affected path.
        let mut by_path: BTreeMap<_, Vec<WorkspaceDiagnostic>> = BTreeMap::new();
        for path in affected_paths {
            by_path.insert(path, Vec::new());
        }
        for diagnostic in current.iter() {
            if let Some(entry) = by_path.get_mut(&diagnostic.path) {
                entry.push(diagnostic.clone());
            }
        }
        by_path
    };

    for (path, path_diagnostics) in affected {
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
                    source: FORWARDING_DIAGNOSTIC_SOURCE.to_string(),
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
                    source: FORWARDING_DIAGNOSTIC_SOURCE.to_string(),
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
                source: FORWARDING_DIAGNOSTIC_SOURCE.to_string(),
                message: format!("Language server stopped: {error}"),
            }],
        },
    )
}
