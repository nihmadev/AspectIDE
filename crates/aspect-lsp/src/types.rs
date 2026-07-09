use std::path::PathBuf;

use aspect_core::WorkspaceDiagnostic;
use serde::{Deserialize, Serialize};

#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Upper bound on the merged `workspace/symbol` result set returned to the IDE.
/// Keeps a polyglot fan-out from flooding the picker while leaving plenty of
/// headroom for relevance.
pub const MAX_WORKSPACE_SYMBOLS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageServerDefinition {
    pub language_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub workspace_root: PathBuf,
    /// Directories prepended to the child process PATH at launch. Carries the
    /// IDE's managed runtime bins (Node/Rust/Python) so a managed server shim
    /// (e.g. `typescript-language-server` -> `node`) finds its interpreter even
    /// when the host has no system toolchain. Empty for system-PATH servers.
    #[serde(default)]
    pub extra_path_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsUpdate {
    pub path: PathBuf,
    pub diagnostics: Vec<WorkspaceDiagnostic>,
}

/// The server's declared `textDocumentSync` change mode (LSP `TextDocumentSyncKind`).
///
/// We default to `Full`: it is the safest, universally-accepted payload when a
/// server doesn't advertise a mode, avoiding the corruption risk of sending ranged
/// edits to a server that can't apply them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDocumentSyncKind {
    /// Server doesn't want incremental change notifications at all.
    None,
    /// Server wants the full document text on every change.
    #[default]
    Full,
    /// Server accepts ranged incremental edits.
    Incremental,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticTokenLegend {
    pub token_types: Vec<String>,
    pub token_modifiers: Vec<String>,
}

pub const CLIENT_SEMANTIC_TOKEN_TYPES: &[&str] = &[
    "namespace",
    "type",
    "class",
    "enum",
    "interface",
    "struct",
    "typeParameter",
    "parameter",
    "variable",
    "property",
    "enumMember",
    "event",
    "function",
    "method",
    "macro",
    "keyword",
    "modifier",
    "comment",
    "string",
    "number",
    "regexp",
    "operator",
    "decorator",
];

pub const CLIENT_SEMANTIC_TOKEN_MODIFIERS: &[&str] = &[
    "declaration",
    "definition",
    "readonly",
    "static",
    "deprecated",
    "abstract",
    "async",
    "modification",
    "documentation",
    "defaultLibrary",
];
