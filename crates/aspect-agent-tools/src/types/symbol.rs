use std::path::PathBuf;
use serde::Serialize;
use aspect_core::{LspDocumentSymbol, LspHover, LspLocation, LspSignatureHelp, LspWorkspaceSymbol, WorkspaceDiagnostic};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSymbolContextResponse {
    pub workspace_root: PathBuf,
    pub query: String,
    pub path: Option<PathBuf>,
    pub position: Option<AiSymbolPosition>,
    pub workspace_symbols: Vec<LspWorkspaceSymbol>,
    pub document_symbols: Vec<LspDocumentSymbol>,
    pub hover: Option<LspHover>,
    pub definitions: Vec<LspLocation>,
    pub references: Vec<LspLocation>,
    pub signature_help: Option<LspSignatureHelp>,
    pub diagnostics: Vec<WorkspaceDiagnostic>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSymbolPosition {
    pub line: u32,
    pub column: u32,
}
