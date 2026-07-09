use std::path::PathBuf;

use tauri::State;

use crate::{lock_error, resolve_workspace_path, workspace_root, SharedState};
use aspect_agent_tools::{
    symbol_utils::{count_document_symbols, filter_document_symbols, truncate_document_symbols},
    types::{AiSymbolContextResponse, AiSymbolPosition},
};

use super::common::symbol_context_document_for_path;

#[tauri::command]
pub async fn ai_symbol_context(
    state: State<'_, SharedState>,
    query: Option<String>,
    path: Option<PathBuf>,
    line: Option<u32>,
    column: Option<u32>,
    max_results: Option<usize>,
) -> Result<AiSymbolContextResponse, String> {
    let root = workspace_root(&state)?;
    let query = query.unwrap_or_default().trim().to_string();
    let max_results = max_results.unwrap_or(80).clamp(1, 300);
    let position = line
        .zip(column)
        .map(|(line, column)| AiSymbolPosition { line, column });
    let resolved_path = path
        .as_deref()
        .map(|path| resolve_workspace_path(&state, path))
        .transpose()?;
    let diagnostics = state.diagnostics.lock().map_err(lock_error)?.clone();
    let mut notes = Vec::new();
    let mut workspace_symbols = Vec::new();
    let mut document_symbols = Vec::new();
    let mut hover = None;
    let mut definitions = Vec::new();
    let mut references = Vec::new();
    let mut signature_help = None;

    let mut lsp = state.lsp.lock().await;
    let Some(manager) = lsp.as_mut() else {
        notes.push("language service is not initialized".to_string());
        return Ok(AiSymbolContextResponse {
            workspace_root: root,
            query,
            path: resolved_path,
            position,
            workspace_symbols,
            document_symbols,
            hover,
            definitions,
            references,
            signature_help,
            diagnostics,
            notes,
        });
    };

    if !query.is_empty() {
        workspace_symbols = manager
            .workspace_symbols(query.clone())
            .await
            .map_err(String::from)?;
        let total = workspace_symbols.len();
        workspace_symbols.truncate(max_results);
        if total > workspace_symbols.len() {
            notes.push(format!(
                "workspace symbols truncated to {} of {total}; raise maxResults to see more",
                workspace_symbols.len()
            ));
        }
    }

    if let Some(path) = &resolved_path {
        let document = symbol_context_document_for_path(&state, path).await?;
        manager
            .open_document(&document)
            .await
            .map_err(String::from)?;
        document_symbols = manager
            .document_symbols(&document)
            .await
            .map_err(String::from)?;
        let total = count_document_symbols(&document_symbols);
        truncate_document_symbols(&mut document_symbols, max_results);
        let shown = count_document_symbols(&document_symbols);
        if total > shown {
            notes.push(format!(
                "document symbols truncated to {shown} of {total}; raise maxResults to see more"
            ));
        }

        if let Some(position) = &position {
            hover = manager
                .hover(&document, position.line, position.column)
                .await
                .map_err(String::from)?;
            definitions = manager
                .definition(&document, position.line, position.column)
                .await
                .map_err(String::from)?;
            references = manager
                .references(&document, position.line, position.column)
                .await
                .map_err(String::from)?;
            signature_help = manager
                .signature_help(&document, position.line, position.column)
                .await
                .map_err(String::from)?;
            let definitions_total = definitions.len();
            let references_total = references.len();
            definitions.truncate(max_results);
            references.truncate(max_results);
            if definitions_total > definitions.len() {
                notes.push(format!(
                    "definitions truncated to {} of {definitions_total}; raise maxResults to see more",
                    definitions.len()
                ));
            }
            if references_total > references.len() {
                notes.push(format!(
                    "references truncated to {} of {references_total}; raise maxResults to see more",
                    references.len()
                ));
            }
        } else if !query.is_empty() {
            document_symbols = filter_document_symbols(&document_symbols, &query, max_results);
        }

        if document_symbols.is_empty() {
            notes.push("no document symbols returned for this file; the language server may still be indexing, may not be running for this language (check Settings -> Language Servers), or may not support document symbols".to_string());
        }
    }

    if workspace_symbols.is_empty()
        && document_symbols.is_empty()
        && resolved_path.is_none()
        && query.is_empty()
    {
        notes.push("provide a query or path for semantic symbol context".to_string());
    }

    Ok(AiSymbolContextResponse {
        workspace_root: root,
        query,
        path: resolved_path,
        position,
        workspace_symbols,
        document_symbols,
        hover,
        definitions,
        references,
        signature_help,
        diagnostics,
        notes,
    })
}
