#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;
use uuid::Uuid;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("glob error: {0}")]
    Glob(#[from] globset::Error),
    #[error("regex error: {0}")]
    Regex(#[from] regex::Error),
    #[error("service error: {0}")]
    Service(String),
}

impl From<AppError> for String {
    fn from(value: AppError) -> Self {
        value.to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspaceInfo {
    pub id: WorkspaceId,
    pub name: String,
    pub root: PathBuf,
    pub opened_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RecentWorkspace {
    pub name: String,
    pub root: PathBuf,
    pub last_opened_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspaceId(pub Uuid);

impl WorkspaceId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for WorkspaceId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FsEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: FsEntryKind,
    #[ts(type = "number")]
    pub size: u64,
    pub modified_at: Option<DateTime<Utc>>,
    pub is_hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FsEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BufferId(pub Uuid);

impl BufferId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for BufferId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DocumentSnapshot {
    pub id: BufferId,
    pub path: Option<PathBuf>,
    pub title: String,
    pub language_id: String,
    pub text: String,
    #[ts(type = "number")]
    pub version: u64,
    pub is_dirty: bool,
    pub is_untitled: bool,
    pub opened_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TextEdit {
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DocumentEditResult {
    pub id: BufferId,
    pub path: Option<PathBuf>,
    pub title: String,
    #[ts(type = "number")]
    pub version: u64,
    pub is_dirty: bool,
    pub is_untitled: bool,
}

impl From<&DocumentSnapshot> for DocumentEditResult {
    fn from(document: &DocumentSnapshot) -> Self {
        Self {
            id: document.id,
            path: document.path.clone(),
            title: document.title.clone(),
            version: document.version,
            is_dirty: document.is_dirty,
            is_untitled: document.is_untitled,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[allow(clippy::struct_excessive_bools)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub use_regex: bool,
    pub include_hidden: bool,
    pub include_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub max_results: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            case_sensitive: false,
            whole_word: false,
            use_regex: false,
            include_hidden: false,
            include_globs: Vec::new(),
            exclude_globs: Vec::new(),
            max_results: 250,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchHit {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub match_length: usize,
    pub match_text: String,
    pub preview: String,
    pub preview_match_start: usize,
    pub preview_match_length: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchResponse {
    pub query: String,
    pub hits: Vec<SearchHit>,
    pub truncated: bool,
    #[ts(type = "number")]
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TerminalSessionInfo {
    pub id: Uuid,
    pub shell: String,
    pub cwd: PathBuf,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitFileStatus {
    pub path: PathBuf,
    pub index_status: String,
    pub worktree_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitStatus {
    pub branch: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<GitFileStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitDiffFile {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitDiff {
    pub files: Vec<GitDiffFile>,
    pub additions: u32,
    pub deletions: u32,
    pub patch: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SettingsScope {
    User,
    Workspace(PathBuf),
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SettingValue {
    pub key: String,
    #[ts(type = "unknown")]
    pub value: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Keybinding {
    pub command: String,
    pub key: String,
    pub when: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct KeybindingProfile {
    pub id: String,
    pub name: String,
    pub bindings: Vec<Keybinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub wasm_module: PathBuf,
    pub contributes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExtensionStatus {
    Discovered,
    Active,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExtensionContributionKind {
    Commands,
    Themes,
    Keybindings,
    Languages,
    Grammars,
    Snippets,
    Views,
    Menus,
    Settings,
    Debuggers,
    Tasks,
    ProblemMatchers,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionContributionPoint {
    pub id: String,
    pub kind: ExtensionContributionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub wasm_module: PathBuf,
    pub contributes: Vec<String>,
    pub contribution_points: Vec<ExtensionContributionPoint>,
    pub status: ExtensionStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DebugAdapterStatus {
    Available,
    Missing,
    NotConfigured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DebugConfigurationRequest {
    Launch,
    Attach,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugAdapterInfo {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub workspace_root: PathBuf,
    pub status: DebugAdapterStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugConfiguration {
    pub name: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub adapter_type: String,
    pub request: DebugConfigurationRequest,
    #[ts(type = "Record<string, unknown>")]
    pub raw: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugWorkspaceInfo {
    pub adapters: Vec<DebugAdapterInfo>,
    pub configurations: Vec<DebugConfiguration>,
    pub launch_json_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LanguageServerStatus {
    Available,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LanguageServerInfo {
    pub language_id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub workspace_root: PathBuf,
    pub status: LanguageServerStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspaceDiagnostic {
    pub path: PathBuf,
    pub line: u32,
    pub column: u32,
    pub severity: DiagnosticSeverity,
    pub source: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspRange {
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspHover {
    pub contents: Vec<String>,
    pub range: Option<LspRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspLocation {
    pub path: PathBuf,
    pub range: LspRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LspSymbolKind {
    File,
    Module,
    Namespace,
    Package,
    Class,
    Method,
    Property,
    Field,
    Constructor,
    Enum,
    Interface,
    Function,
    Variable,
    Constant,
    String,
    Number,
    Boolean,
    Array,
    Object,
    Key,
    Null,
    EnumMember,
    Struct,
    Event,
    Operator,
    TypeParameter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspDocumentSymbol {
    pub name: String,
    pub detail: Option<String>,
    pub kind: LspSymbolKind,
    pub range: LspRange,
    pub selection_range: LspRange,
    pub children: Vec<Self>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspWorkspaceSymbol {
    pub name: String,
    pub kind: LspSymbolKind,
    pub location: LspLocation,
    pub container_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LspFoldingRangeKind {
    Comment,
    Imports,
    Region,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspFoldingRange {
    pub start_line: u32,
    pub end_line: u32,
    pub start_column: Option<u32>,
    pub end_column: Option<u32>,
    pub kind: Option<LspFoldingRangeKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LspInlayHintKind {
    Type,
    Parameter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspInlayHint {
    pub label: String,
    pub tooltip: Option<String>,
    pub line: u32,
    pub column: u32,
    pub kind: Option<LspInlayHintKind>,
    pub padding_left: bool,
    pub padding_right: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspSemanticTokens {
    pub result_id: Option<String>,
    pub data: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LspCompletionItemKind {
    Text,
    Method,
    Function,
    Constructor,
    Field,
    Variable,
    Class,
    Interface,
    Module,
    Property,
    Unit,
    Value,
    Enum,
    Keyword,
    Snippet,
    Color,
    File,
    Reference,
    Folder,
    EnumMember,
    Constant,
    Struct,
    Event,
    Operator,
    TypeParameter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LspInsertTextFormat {
    PlainText,
    Snippet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspCompletionItem {
    pub label: String,
    pub kind: Option<LspCompletionItemKind>,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    pub insert_text: String,
    pub insert_text_format: LspInsertTextFormat,
    pub filter_text: Option<String>,
    pub sort_text: Option<String>,
    pub range: Option<LspRange>,
    pub commit_characters: Vec<String>,
    pub preselect: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspCompletionList {
    pub is_incomplete: bool,
    pub items: Vec<LspCompletionItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspCodeActionDiagnostic {
    pub range: LspRange,
    pub severity: Option<DiagnosticSeverity>,
    pub source: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspCodeAction {
    pub title: String,
    pub kind: Option<String>,
    pub is_preferred: bool,
    pub disabled_reason: Option<String>,
    pub edit: Option<LspWorkspaceEdit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LspCodeActionTrigger {
    Invoke,
    Automatic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspFormattingOptions {
    pub tab_size: u32,
    pub insert_spaces: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspSignatureParameter {
    pub label: String,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspSignatureInformation {
    pub label: String,
    pub documentation: Option<String>,
    pub parameters: Vec<LspSignatureParameter>,
    pub active_parameter: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspSignatureHelp {
    pub signatures: Vec<LspSignatureInformation>,
    pub active_signature: Option<u32>,
    pub active_parameter: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspTextEdit {
    pub range: LspRange,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspWorkspaceEditFile {
    pub path: PathBuf,
    pub edits: Vec<LspTextEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LspWorkspaceEdit {
    pub files: Vec<LspWorkspaceEditFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspaceEditResult {
    pub edited_documents: Vec<DocumentSnapshot>,
    pub changed_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum LuxEvent {
    WorkspaceChanged {
        workspace: Option<WorkspaceInfo>,
    },
    FsChanged {
        path: PathBuf,
    },
    EditorDocumentClosed {
        document: DocumentSnapshot,
    },
    EditorDocumentChanged {
        document: DocumentSnapshot,
    },
    EditorDocumentsChanged {
        documents: Vec<DocumentSnapshot>,
    },
    EditorDocumentEdited {
        document: DocumentEditResult,
    },
    EditorDiagnosticsChanged {
        path: PathBuf,
        diagnostics: Vec<WorkspaceDiagnostic>,
    },
    SearchProgress {
        query: String,
        indexed_files: usize,
    },
    TerminalOutput {
        session_id: Uuid,
        data: String,
    },
    GitStatusChanged {
        status: GitStatus,
    },
    SettingsChanged {
        key: String,
    },
}
