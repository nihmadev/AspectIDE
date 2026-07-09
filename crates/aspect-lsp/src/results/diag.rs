use std::path::Path;

use aspect_core::{DiagnosticSeverity, WorkspaceDiagnostic};
use lsp_types::{Diagnostic, PublishDiagnosticsParams};

use crate::types::DiagnosticsUpdate;

use super::common::{map_lsp_severity, uri_to_path};

pub fn diagnostics_update_from_publish(params: PublishDiagnosticsParams) -> DiagnosticsUpdate {
    let path = uri_to_path(&params.uri).unwrap_or_else(|| std::path::PathBuf::from(params.uri.as_str()));
    let diagnostics = workspace_diagnostics_for_path(&path, params.diagnostics);
    DiagnosticsUpdate { path, diagnostics }
}

pub fn workspace_diagnostics_from_publish(
    params: PublishDiagnosticsParams,
) -> Vec<WorkspaceDiagnostic> {
    let path = uri_to_path(&params.uri).unwrap_or_else(|| std::path::PathBuf::from(params.uri.as_str()));
    workspace_diagnostics_for_path(&path, params.diagnostics)
}

fn workspace_diagnostics_for_path(
    path: &Path,
    diagnostics: Vec<Diagnostic>,
) -> Vec<WorkspaceDiagnostic> {
    diagnostics
        .into_iter()
        .map(|diagnostic| WorkspaceDiagnostic {
            path: path.to_path_buf(),
            line: diagnostic.range.start.line.saturating_add(1),
            column: diagnostic.range.start.character.saturating_add(1),
            severity: diagnostic
                .severity
                .map_or(DiagnosticSeverity::Information, map_lsp_severity),
            source: diagnostic.source.unwrap_or_else(|| "lsp".to_string()),
            message: diagnostic.message,
        })
        .collect()
}


