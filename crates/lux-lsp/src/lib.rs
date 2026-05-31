use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::{Path, PathBuf},
    process::Stdio,
};

use lsp_types::{Diagnostic, PublishDiagnosticsParams, Uri};
use lux_core::{
    AppError, AppResult, DiagnosticSeverity, DocumentSnapshot, LanguageServerInfo,
    LanguageServerStatus, LspCodeAction, LspCodeActionDiagnostic, LspCodeActionTrigger,
    LspCompletionItem, LspCompletionItemKind, LspCompletionList, LspDocumentSymbol,
    LspFoldingRange, LspFoldingRangeKind, LspFormattingOptions, LspHover, LspInlayHint,
    LspInlayHintKind, LspInsertTextFormat, LspLocation, LspRange, LspSemanticTokens,
    LspSignatureHelp, LspSignatureInformation, LspSignatureParameter, LspSymbolKind, LspTextEdit,
    LspWorkspaceEdit, LspWorkspaceEditFile, LspWorkspaceSymbol, TextEdit, WorkspaceDiagnostic,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, Command},
    sync::mpsc,
    task::JoinHandle,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageServerDefinition {
    pub language_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub workspace_root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsUpdate {
    pub path: PathBuf,
    pub diagnostics: Vec<WorkspaceDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspFrame {
    pub content: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum LspNotification {
    PublishDiagnostics(DiagnosticsUpdate),
    Other { method: String },
}

pub struct LspManager {
    diagnostics_tx: mpsc::UnboundedSender<DiagnosticsUpdate>,
    sessions: BTreeMap<String, LspSession>,
}

pub struct LspSession {
    definition: LanguageServerDefinition,
    stdin: ChildStdin,
    child: Child,
    read_task: JoinHandle<()>,
    stderr_task: Option<JoinHandle<()>>,
    responses: mpsc::UnboundedReceiver<LspResponse>,
    request_id: u64,
    opened_documents: BTreeMap<PathBuf, u64>,
    semantic_token_legend: Option<SemanticTokenLegend>,
}

#[derive(Debug, Clone)]
struct LspResponse {
    id: u64,
    error: Option<String>,
    result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticTokenLegend {
    token_types: Vec<String>,
    token_modifiers: Vec<String>,
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

#[derive(Debug, Clone, Copy)]
struct BuiltinServer {
    language_id: &'static str,
    name: &'static str,
    command: &'static str,
    args: &'static [&'static str],
    extensions: &'static [&'static str],
}

const BUILTIN_SERVERS: &[BuiltinServer] = &[
    BuiltinServer {
        language_id: "rust",
        name: "rust-analyzer",
        command: "rust-analyzer",
        args: &[],
        extensions: &["rs"],
    },
    BuiltinServer {
        language_id: "typescript",
        name: "TypeScript Language Server",
        command: "typescript-language-server",
        args: &["--stdio"],
        extensions: &["ts", "tsx", "js", "jsx"],
    },
    BuiltinServer {
        language_id: "json",
        name: "JSON Language Server",
        command: "vscode-json-language-server",
        args: &["--stdio"],
        extensions: &["json"],
    },
];

impl LspManager {
    pub fn new(diagnostics_tx: mpsc::UnboundedSender<DiagnosticsUpdate>) -> Self {
        Self {
            diagnostics_tx,
            sessions: BTreeMap::new(),
        }
    }

    pub async fn start_available_servers(
        &mut self,
        servers: &[LanguageServerInfo],
    ) -> Vec<WorkspaceDiagnostic> {
        let wanted_languages = servers
            .iter()
            .filter(|server| server.status == LanguageServerStatus::Available)
            .map(|server| server.language_id.clone())
            .collect::<BTreeSet<_>>();
        let stale_languages = self
            .sessions
            .keys()
            .filter(|language_id| !wanted_languages.contains(*language_id))
            .cloned()
            .collect::<Vec<_>>();

        for language_id in stale_languages {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }

        let mut diagnostics = Vec::new();
        for server in servers {
            if server.status != LanguageServerStatus::Available {
                continue;
            }

            let definition = LanguageServerDefinition::from(server);
            let should_restart = self
                .sessions
                .get(&server.language_id)
                .is_some_and(|session| session.definition != definition);
            if should_restart {
                if let Some(session) = self.sessions.remove(&server.language_id) {
                    session.shutdown().await;
                }
            }

            if self.sessions.contains_key(&server.language_id) {
                continue;
            }

            match LspSession::start(definition, self.diagnostics_tx.clone()).await {
                Ok(session) => {
                    self.sessions.insert(server.language_id.clone(), session);
                }
                Err(error) => diagnostics.push(WorkspaceDiagnostic {
                    path: diagnostic_anchor_path(server),
                    line: 1,
                    column: 1,
                    severity: DiagnosticSeverity::Warning,
                    source: "lux-lsp".to_string(),
                    message: format!("Failed to start {}: {error}", server.name),
                }),
            }
        }

        diagnostics
    }

    pub async fn open_document(&mut self, document: &DocumentSnapshot) -> AppResult<()> {
        if document.path.is_none() {
            return Ok(());
        }
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(());
        };
        let result = session.did_open(document).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn update_document(&mut self, document: &DocumentSnapshot) -> AppResult<()> {
        if document.path.is_none() {
            return Ok(());
        }
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(());
        };
        let result = session.did_change(document).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn apply_document_edits(
        &mut self,
        document: &DocumentSnapshot,
        edits: &[TextEdit],
    ) -> AppResult<()> {
        if document.path.is_none() {
            return Ok(());
        }
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(());
        };
        let result = session.did_change_edits(document, edits).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn save_document(&mut self, document: &DocumentSnapshot) -> AppResult<()> {
        if document.path.is_none() {
            return Ok(());
        }
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(());
        };
        let result = session.did_save(document).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn close_document(&mut self, path: &Path) -> AppResult<()> {
        let language_id = session_language_id(&language_id_for_path(path)).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(());
        };
        let result = session.did_close(path).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn hover(
        &mut self,
        document: &DocumentSnapshot,
        line: u32,
        column: u32,
    ) -> AppResult<Option<LspHover>> {
        let Some(path) = document.path.as_ref() else {
            return Ok(None);
        };
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(None);
        };
        let result = session.hover(path, line, column).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn definition(
        &mut self,
        document: &DocumentSnapshot,
        line: u32,
        column: u32,
    ) -> AppResult<Vec<LspLocation>> {
        let Some(path) = document.path.as_ref() else {
            return Ok(Vec::new());
        };
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(Vec::new());
        };
        let result = session.definition(path, line, column).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn references(
        &mut self,
        document: &DocumentSnapshot,
        line: u32,
        column: u32,
    ) -> AppResult<Vec<LspLocation>> {
        let Some(path) = document.path.as_ref() else {
            return Ok(Vec::new());
        };
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(Vec::new());
        };
        let result = session.references(path, line, column).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn document_symbols(
        &mut self,
        document: &DocumentSnapshot,
    ) -> AppResult<Vec<LspDocumentSymbol>> {
        let Some(path) = document.path.as_ref() else {
            return Ok(Vec::new());
        };
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(Vec::new());
        };
        let result = session.document_symbols(path).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn workspace_symbols(&mut self, query: String) -> AppResult<Vec<LspWorkspaceSymbol>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut symbols = Vec::new();
        let languages = self.sessions.keys().cloned().collect::<Vec<_>>();
        for language_id in languages {
            let Some(session) = self.sessions.get_mut(&language_id) else {
                continue;
            };
            let result = session.workspace_symbols(&query).await;
            if result.is_err() {
                if let Some(session) = self.sessions.remove(&language_id) {
                    session.shutdown().await;
                }
                continue;
            }
            symbols.extend(result?);
        }

        symbols.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.location.path.cmp(&right.location.path))
        });
        symbols.truncate(200);
        Ok(symbols)
    }

    pub async fn folding_ranges(
        &mut self,
        document: &DocumentSnapshot,
    ) -> AppResult<Vec<LspFoldingRange>> {
        let Some(path) = document.path.as_ref() else {
            return Ok(Vec::new());
        };
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(Vec::new());
        };
        let result = session.folding_ranges(path).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn inlay_hints(
        &mut self,
        document: &DocumentSnapshot,
        range: LspRange,
    ) -> AppResult<Vec<LspInlayHint>> {
        let Some(path) = document.path.as_ref() else {
            return Ok(Vec::new());
        };
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(Vec::new());
        };
        let result = session.inlay_hints(path, range).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn semantic_tokens(
        &mut self,
        document: &DocumentSnapshot,
    ) -> AppResult<Option<LspSemanticTokens>> {
        let Some(path) = document.path.as_ref() else {
            return Ok(None);
        };
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(None);
        };
        let result = session.semantic_tokens(path).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn rename(
        &mut self,
        document: &DocumentSnapshot,
        line: u32,
        column: u32,
        new_name: String,
    ) -> AppResult<Option<LspWorkspaceEdit>> {
        let Some(path) = document.path.as_ref() else {
            return Ok(None);
        };
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(None);
        };
        let result = session.rename(path, line, column, new_name).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn completion(
        &mut self,
        document: &DocumentSnapshot,
        line: u32,
        column: u32,
    ) -> AppResult<LspCompletionList> {
        let Some(path) = document.path.as_ref() else {
            return Ok(empty_completion_list());
        };
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(empty_completion_list());
        };
        let result = session.completion(path, line, column).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn code_actions(
        &mut self,
        document: &DocumentSnapshot,
        range: LspRange,
        diagnostics: Vec<LspCodeActionDiagnostic>,
        only: Option<Vec<String>>,
        trigger: LspCodeActionTrigger,
    ) -> AppResult<Vec<LspCodeAction>> {
        let Some(path) = document.path.as_ref() else {
            return Ok(Vec::new());
        };
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(Vec::new());
        };
        let result = session
            .code_actions(path, range, diagnostics, only, trigger)
            .await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn format_document(
        &mut self,
        document: &DocumentSnapshot,
        options: LspFormattingOptions,
    ) -> AppResult<Vec<LspTextEdit>> {
        let Some(path) = document.path.as_ref() else {
            return Ok(Vec::new());
        };
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(Vec::new());
        };
        let result = session.format_document(path, options).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn format_range(
        &mut self,
        document: &DocumentSnapshot,
        range: LspRange,
        options: LspFormattingOptions,
    ) -> AppResult<Vec<LspTextEdit>> {
        let Some(path) = document.path.as_ref() else {
            return Ok(Vec::new());
        };
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(Vec::new());
        };
        let result = session.format_range(path, range, options).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn signature_help(
        &mut self,
        document: &DocumentSnapshot,
        line: u32,
        column: u32,
    ) -> AppResult<Option<LspSignatureHelp>> {
        let Some(path) = document.path.as_ref() else {
            return Ok(None);
        };
        let language_id = session_language_id(&document.language_id).to_string();
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(None);
        };
        let result = session.signature_help(path, line, column).await;
        if result.is_err() {
            if let Some(session) = self.sessions.remove(&language_id) {
                session.shutdown().await;
            }
        }
        result
    }

    pub async fn shutdown_all(&mut self) {
        let sessions = std::mem::take(&mut self.sessions);
        for (_, session) in sessions {
            session.shutdown().await;
        }
    }
}

impl Drop for LspManager {
    fn drop(&mut self) {
        for (_, mut session) in std::mem::take(&mut self.sessions) {
            let _ = session.child.start_kill();
            session.read_task.abort();
            if let Some(stderr_task) = session.stderr_task.take() {
                stderr_task.abort();
            }
        }
    }
}

impl LspSession {
    pub async fn start(
        definition: LanguageServerDefinition,
        diagnostics_tx: mpsc::UnboundedSender<DiagnosticsUpdate>,
    ) -> AppResult<Self> {
        let mut child = Command::new(&definition.command)
            .args(&definition.args)
            .current_dir(&definition.workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let stdin = child.stdin.take().ok_or_else(|| {
            AppError::Service(format!("{} did not expose stdin", definition.command))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            AppError::Service(format!("{} did not expose stdout", definition.command))
        })?;
        let stderr = child.stderr.take();
        let (response_tx, response_rx) = mpsc::unbounded_channel();
        let read_task = tokio::spawn(read_lsp_stdout(stdout, diagnostics_tx, response_tx));
        let stderr_task = stderr.map(|stderr| tokio::spawn(drain_stderr(stderr)));

        let mut session = Self {
            definition,
            stdin,
            child,
            read_task,
            stderr_task,
            responses: response_rx,
            request_id: 0,
            opened_documents: BTreeMap::new(),
            semantic_token_legend: None,
        };
        session.initialize().await?;
        Ok(session)
    }

    pub async fn did_open(&mut self, document: &DocumentSnapshot) -> AppResult<()> {
        let Some(path) = document.path.as_ref() else {
            return Ok(());
        };
        let is_open = self.opened_documents.contains_key(path);
        let message = if is_open {
            did_change_notification(document)
        } else {
            did_open_notification(document)
        };

        self.write_message(&message).await?;
        self.opened_documents.insert(path.clone(), document.version);
        Ok(())
    }

    pub async fn did_change(&mut self, document: &DocumentSnapshot) -> AppResult<()> {
        let Some(path) = document.path.as_ref() else {
            return Ok(());
        };
        let is_open = self.opened_documents.contains_key(path);
        let message = if is_open {
            did_change_notification(document)
        } else {
            did_open_notification(document)
        };

        self.write_message(&message).await?;
        self.opened_documents.insert(path.clone(), document.version);
        Ok(())
    }

    pub async fn did_change_edits(
        &mut self,
        document: &DocumentSnapshot,
        edits: &[TextEdit],
    ) -> AppResult<()> {
        let Some(path) = document.path.as_ref() else {
            return Ok(());
        };
        if edits.is_empty() {
            return Ok(());
        }

        if !self.opened_documents.contains_key(path) {
            self.write_message(&did_open_notification(document)).await?;
            self.opened_documents.insert(path.clone(), document.version);
            return Ok(());
        }

        self.write_message(&did_change_edits_notification(document, edits))
            .await?;
        self.opened_documents.insert(path.clone(), document.version);
        Ok(())
    }

    pub async fn did_save(&mut self, document: &DocumentSnapshot) -> AppResult<()> {
        let Some(path) = document.path.as_ref() else {
            return Ok(());
        };
        if !self.opened_documents.contains_key(path) {
            self.write_message(&did_open_notification(document)).await?;
            self.opened_documents.insert(path.clone(), document.version);
        }

        self.write_message(&did_save_notification(document)).await
    }

    pub async fn did_close(&mut self, path: &Path) -> AppResult<()> {
        if self.opened_documents.remove(path).is_none() {
            return Ok(());
        }

        self.write_message(&did_close_notification(path)).await
    }

    pub async fn hover(
        &mut self,
        path: &Path,
        line: u32,
        column: u32,
    ) -> AppResult<Option<LspHover>> {
        if !self.opened_documents.contains_key(path) {
            return Ok(None);
        }
        let request_id = self.next_request_id();
        self.write_message(&hover_request(request_id, path, line, column))
            .await?;
        let response = self
            .wait_for_response_with_timeout(request_id, std::time::Duration::from_secs(5))
            .await?;
        if let Some(error) = response.error {
            return Err(AppError::Service(format!(
                "{} hover failed: {error}",
                self.definition.command
            )));
        }
        Ok(response.result.as_ref().and_then(parse_hover_result))
    }

    pub async fn definition(
        &mut self,
        path: &Path,
        line: u32,
        column: u32,
    ) -> AppResult<Vec<LspLocation>> {
        if !self.opened_documents.contains_key(path) {
            return Ok(Vec::new());
        }
        let request_id = self.next_request_id();
        self.write_message(&definition_request(request_id, path, line, column))
            .await?;
        let response = self
            .wait_for_response_with_timeout(request_id, std::time::Duration::from_secs(5))
            .await?;
        if let Some(error) = response.error {
            return Err(AppError::Service(format!(
                "{} definition failed: {error}",
                self.definition.command
            )));
        }
        Ok(response
            .result
            .as_ref()
            .map(parse_definition_result)
            .unwrap_or_default())
    }

    pub async fn references(
        &mut self,
        path: &Path,
        line: u32,
        column: u32,
    ) -> AppResult<Vec<LspLocation>> {
        if !self.opened_documents.contains_key(path) {
            return Ok(Vec::new());
        }
        let request_id = self.next_request_id();
        self.write_message(&references_request(request_id, path, line, column))
            .await?;
        let response = self
            .wait_for_response_with_timeout(request_id, std::time::Duration::from_secs(5))
            .await?;
        if let Some(error) = response.error {
            return Err(AppError::Service(format!(
                "{} references failed: {error}",
                self.definition.command
            )));
        }
        Ok(response
            .result
            .as_ref()
            .map(parse_definition_result)
            .unwrap_or_default())
    }

    pub async fn document_symbols(&mut self, path: &Path) -> AppResult<Vec<LspDocumentSymbol>> {
        if !self.opened_documents.contains_key(path) {
            return Ok(Vec::new());
        }
        let request_id = self.next_request_id();
        self.write_message(&document_symbol_request(request_id, path))
            .await?;
        let response = self
            .wait_for_response_with_timeout(request_id, std::time::Duration::from_secs(5))
            .await?;
        if let Some(error) = response.error {
            return Err(AppError::Service(format!(
                "{} document symbols failed: {error}",
                self.definition.command
            )));
        }
        Ok(response
            .result
            .as_ref()
            .map(parse_document_symbol_result)
            .unwrap_or_default())
    }

    pub async fn workspace_symbols(&mut self, query: &str) -> AppResult<Vec<LspWorkspaceSymbol>> {
        let request_id = self.next_request_id();
        self.write_message(&workspace_symbol_request(request_id, query))
            .await?;
        let response = self
            .wait_for_response_with_timeout(request_id, std::time::Duration::from_secs(8))
            .await?;
        if let Some(error) = response.error {
            return Err(AppError::Service(format!(
                "{} workspace symbols failed: {error}",
                self.definition.command
            )));
        }
        Ok(response
            .result
            .as_ref()
            .map(parse_workspace_symbol_result)
            .unwrap_or_default())
    }

    pub async fn folding_ranges(&mut self, path: &Path) -> AppResult<Vec<LspFoldingRange>> {
        if !self.opened_documents.contains_key(path) {
            return Ok(Vec::new());
        }
        let request_id = self.next_request_id();
        self.write_message(&folding_range_request(request_id, path))
            .await?;
        let response = self
            .wait_for_response_with_timeout(request_id, std::time::Duration::from_secs(5))
            .await?;
        if let Some(error) = response.error {
            return Err(AppError::Service(format!(
                "{} folding ranges failed: {error}",
                self.definition.command
            )));
        }
        Ok(response
            .result
            .as_ref()
            .map(parse_folding_range_result)
            .unwrap_or_default())
    }

    pub async fn inlay_hints(
        &mut self,
        path: &Path,
        range: LspRange,
    ) -> AppResult<Vec<LspInlayHint>> {
        if !self.opened_documents.contains_key(path) {
            return Ok(Vec::new());
        }
        let request_id = self.next_request_id();
        self.write_message(&inlay_hint_request(request_id, path, &range))
            .await?;
        let response = self
            .wait_for_response_with_timeout(request_id, std::time::Duration::from_secs(5))
            .await?;
        if response.error.is_some() {
            return Ok(Vec::new());
        }
        Ok(response
            .result
            .as_ref()
            .map(parse_inlay_hint_result)
            .unwrap_or_default())
    }

    pub async fn semantic_tokens(&mut self, path: &Path) -> AppResult<Option<LspSemanticTokens>> {
        if !self.opened_documents.contains_key(path) {
            return Ok(None);
        }
        let Some(legend) = self.semantic_token_legend.clone() else {
            return Ok(None);
        };

        let request_id = self.next_request_id();
        self.write_message(&semantic_tokens_full_request(request_id, path))
            .await?;
        let response = self
            .wait_for_response_with_timeout(request_id, std::time::Duration::from_secs(10))
            .await?;
        if response.error.is_some() {
            return Ok(None);
        }
        Ok(response
            .result
            .as_ref()
            .and_then(|value| parse_semantic_tokens_result(value, &legend)))
    }

    pub async fn rename(
        &mut self,
        path: &Path,
        line: u32,
        column: u32,
        new_name: String,
    ) -> AppResult<Option<LspWorkspaceEdit>> {
        if !self.opened_documents.contains_key(path) {
            return Ok(None);
        }
        let request_id = self.next_request_id();
        self.write_message(&rename_request(request_id, path, line, column, &new_name))
            .await?;
        let response = self
            .wait_for_response_with_timeout(request_id, std::time::Duration::from_secs(5))
            .await?;
        if let Some(error) = response.error {
            return Err(AppError::Service(format!(
                "{} rename failed: {error}",
                self.definition.command
            )));
        }
        Ok(response
            .result
            .as_ref()
            .and_then(parse_workspace_edit_result))
    }

    pub async fn completion(
        &mut self,
        path: &Path,
        line: u32,
        column: u32,
    ) -> AppResult<LspCompletionList> {
        if !self.opened_documents.contains_key(path) {
            return Ok(empty_completion_list());
        }
        let request_id = self.next_request_id();
        self.write_message(&completion_request(request_id, path, line, column))
            .await?;
        let response = self
            .wait_for_response_with_timeout(request_id, std::time::Duration::from_secs(5))
            .await?;
        if let Some(error) = response.error {
            return Err(AppError::Service(format!(
                "{} completion failed: {error}",
                self.definition.command
            )));
        }
        Ok(response
            .result
            .as_ref()
            .map(parse_completion_result)
            .unwrap_or_else(empty_completion_list))
    }

    pub async fn code_actions(
        &mut self,
        path: &Path,
        range: LspRange,
        diagnostics: Vec<LspCodeActionDiagnostic>,
        only: Option<Vec<String>>,
        trigger: LspCodeActionTrigger,
    ) -> AppResult<Vec<LspCodeAction>> {
        if !self.opened_documents.contains_key(path) {
            return Ok(Vec::new());
        }
        let request_id = self.next_request_id();
        self.write_message(&code_action_request(
            request_id,
            path,
            &range,
            &diagnostics,
            only.as_deref(),
            trigger,
        ))
        .await?;
        let response = self
            .wait_for_response_with_timeout(request_id, std::time::Duration::from_secs(5))
            .await?;
        if let Some(error) = response.error {
            return Err(AppError::Service(format!(
                "{} code action failed: {error}",
                self.definition.command
            )));
        }
        Ok(response
            .result
            .as_ref()
            .map(parse_code_action_result)
            .unwrap_or_default())
    }

    pub async fn format_document(
        &mut self,
        path: &Path,
        options: LspFormattingOptions,
    ) -> AppResult<Vec<LspTextEdit>> {
        if !self.opened_documents.contains_key(path) {
            return Ok(Vec::new());
        }
        let request_id = self.next_request_id();
        self.write_message(&formatting_request(request_id, path, options))
            .await?;
        let response = self
            .wait_for_response_with_timeout(request_id, std::time::Duration::from_secs(10))
            .await?;
        if let Some(error) = response.error {
            return Err(AppError::Service(format!(
                "{} formatting failed: {error}",
                self.definition.command
            )));
        }
        Ok(response
            .result
            .as_ref()
            .map(parse_text_edits_result)
            .unwrap_or_default())
    }

    pub async fn format_range(
        &mut self,
        path: &Path,
        range: LspRange,
        options: LspFormattingOptions,
    ) -> AppResult<Vec<LspTextEdit>> {
        if !self.opened_documents.contains_key(path) {
            return Ok(Vec::new());
        }
        let request_id = self.next_request_id();
        self.write_message(&range_formatting_request(request_id, path, &range, options))
            .await?;
        let response = self
            .wait_for_response_with_timeout(request_id, std::time::Duration::from_secs(10))
            .await?;
        if let Some(error) = response.error {
            return Err(AppError::Service(format!(
                "{} range formatting failed: {error}",
                self.definition.command
            )));
        }
        Ok(response
            .result
            .as_ref()
            .map(parse_text_edits_result)
            .unwrap_or_default())
    }

    pub async fn signature_help(
        &mut self,
        path: &Path,
        line: u32,
        column: u32,
    ) -> AppResult<Option<LspSignatureHelp>> {
        if !self.opened_documents.contains_key(path) {
            return Ok(None);
        }
        let request_id = self.next_request_id();
        self.write_message(&signature_help_request(request_id, path, line, column))
            .await?;
        let response = self
            .wait_for_response_with_timeout(request_id, std::time::Duration::from_secs(5))
            .await?;
        if let Some(error) = response.error {
            return Err(AppError::Service(format!(
                "{} signature help failed: {error}",
                self.definition.command
            )));
        }
        Ok(response
            .result
            .as_ref()
            .and_then(parse_signature_help_result))
    }

    pub async fn shutdown(mut self) {
        let request_id = self.next_request_id();
        let _ = self.write_message(&shutdown_request(request_id)).await;
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            self.wait_for_response(request_id),
        )
        .await;
        let _ = self.write_message(&exit_notification()).await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), self.child.wait()).await;
        let _ = self.child.start_kill();
        self.read_task.abort();
        if let Some(stderr_task) = self.stderr_task.take() {
            stderr_task.abort();
        }
    }

    async fn initialize(&mut self) -> AppResult<()> {
        let request_id = self.next_request_id();
        self.write_message(&initialize_request(request_id, &self.definition))
            .await?;
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.wait_for_response(request_id),
        )
        .await
        .map_err(|_| {
            AppError::Service(format!("{} initialize timed out", self.definition.command))
        })??;

        if let Some(error) = response.error {
            return Err(AppError::Service(format!(
                "{} initialize failed: {error}",
                self.definition.command
            )));
        }

        self.semantic_token_legend = response
            .result
            .as_ref()
            .and_then(parse_semantic_token_legend_from_initialize);

        self.write_message(&initialized_notification()).await
    }

    async fn wait_for_response(&mut self, request_id: u64) -> AppResult<LspResponse> {
        while let Some(response) = self.responses.recv().await {
            if response.id == request_id {
                return Ok(response);
            }
        }

        Err(AppError::Service(format!(
            "{} exited before response {request_id}",
            self.definition.command
        )))
    }

    async fn wait_for_response_with_timeout(
        &mut self,
        request_id: u64,
        duration: std::time::Duration,
    ) -> AppResult<LspResponse> {
        tokio::time::timeout(duration, self.wait_for_response(request_id))
            .await
            .map_err(|_| {
                AppError::Service(format!(
                    "{} request {request_id} timed out",
                    self.definition.command
                ))
            })?
    }

    async fn write_message(&mut self, value: &Value) -> AppResult<()> {
        let message = encode_lsp_message(value)?;
        self.stdin.write_all(&message).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    fn next_request_id(&mut self) -> u64 {
        self.request_id += 1;
        self.request_id
    }
}

impl Drop for LspSession {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        self.read_task.abort();
        if let Some(stderr_task) = self.stderr_task.take() {
            stderr_task.abort();
        }
    }
}

impl From<&LanguageServerInfo> for LanguageServerDefinition {
    fn from(server: &LanguageServerInfo) -> Self {
        Self {
            language_id: server.language_id.clone(),
            command: server.command.clone(),
            args: server.args.clone(),
            workspace_root: server.workspace_root.clone(),
        }
    }
}

pub fn encode_lsp_message(value: &Value) -> AppResult<Vec<u8>> {
    let content = serde_json::to_vec(value)?;
    let mut message = format!("Content-Length: {}\r\n\r\n", content.len()).into_bytes();
    message.extend_from_slice(&content);
    Ok(message)
}

pub fn initialize_request(id: u64, definition: &LanguageServerDefinition) -> Value {
    let root_uri = path_to_file_uri(&definition.workspace_root);
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "publishDiagnostics": {
                        "relatedInformation": true,
                        "versionSupport": true,
                        "codeDescriptionSupport": true,
                        "dataSupport": true
                    },
                    "synchronization": {
                        "dynamicRegistration": false,
                        "didSave": true
                    },
                    "hover": {
                        "dynamicRegistration": false,
                        "contentFormat": ["markdown", "plaintext"]
                    },
                    "definition": {
                        "dynamicRegistration": false,
                        "linkSupport": true
                    },
                    "references": {
                        "dynamicRegistration": false
                    },
                    "documentSymbol": {
                        "dynamicRegistration": false,
                        "hierarchicalDocumentSymbolSupport": true,
                        "symbolKind": {
                            "valueSet": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26]
                        }
                    },
                    "foldingRange": {
                        "dynamicRegistration": false,
                        "lineFoldingOnly": true
                    },
                    "inlayHint": {
                        "dynamicRegistration": false,
                        "resolveSupport": {
                            "properties": ["tooltip", "textEdits", "label.tooltip", "label.location", "label.command"]
                        }
                    },
                    "semanticTokens": {
                        "dynamicRegistration": false,
                        "requests": {
                            "range": false,
                            "full": true
                        },
                        "tokenTypes": CLIENT_SEMANTIC_TOKEN_TYPES,
                        "tokenModifiers": CLIENT_SEMANTIC_TOKEN_MODIFIERS,
                        "formats": ["relative"],
                        "overlappingTokenSupport": false,
                        "multilineTokenSupport": true,
                        "serverCancelSupport": false,
                        "augmentsSyntaxTokens": true
                    },
                    "rename": {
                        "dynamicRegistration": false,
                        "prepareSupport": false
                    },
                    "codeAction": {
                        "dynamicRegistration": false,
                        "isPreferredSupport": true,
                        "disabledSupport": true,
                        "dataSupport": false,
                        "codeActionLiteralSupport": {
                            "codeActionKind": {
                                "valueSet": ["", "quickfix", "refactor", "refactor.extract", "refactor.inline", "refactor.rewrite", "source", "source.organizeImports", "source.fixAll"]
                            }
                        }
                    },
                    "formatting": {
                        "dynamicRegistration": false
                    },
                    "rangeFormatting": {
                        "dynamicRegistration": false
                    },
                    "completion": {
                        "dynamicRegistration": false,
                        "completionItem": {
                            "snippetSupport": true,
                            "commitCharactersSupport": true,
                            "documentationFormat": ["markdown", "plaintext"],
                            "deprecatedSupport": true,
                            "preselectSupport": true,
                            "tagSupport": {
                                "valueSet": [1]
                            }
                        },
                        "completionItemKind": {
                            "valueSet": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25]
                        },
                        "contextSupport": true
                    },
                    "signatureHelp": {
                        "dynamicRegistration": false,
                        "signatureInformation": {
                            "documentationFormat": ["markdown", "plaintext"],
                            "parameterInformation": {
                                "labelOffsetSupport": true
                            },
                            "activeParameterSupport": true
                        },
                        "contextSupport": true
                    }
                },
                "workspace": {
                    "symbol": {
                        "dynamicRegistration": false,
                        "symbolKind": {
                            "valueSet": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26]
                        }
                    },
                    "workspaceFolders": false,
                    "configuration": false
                }
            },
            "workspaceFolders": [{
                "uri": root_uri,
                "name": definition.workspace_root.file_name().and_then(|name| name.to_str()).unwrap_or("workspace")
            }]
        }
    })
}

pub fn initialized_notification() -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    })
}

pub fn did_open_notification(document: &DocumentSnapshot) -> Value {
    let path = document
        .path
        .as_ref()
        .expect("LSP didOpen requires a file-backed document");
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path),
                "languageId": lsp_language_id(document),
                "version": document_version(document),
                "text": document.text
            }
        }
    })
}

pub fn did_change_notification(document: &DocumentSnapshot) -> Value {
    let path = document
        .path
        .as_ref()
        .expect("LSP didChange requires a file-backed document");
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path),
                "version": document_version(document)
            },
            "contentChanges": [{
                "text": document.text
            }]
        }
    })
}

pub fn did_change_edits_notification(document: &DocumentSnapshot, edits: &[TextEdit]) -> Value {
    let path = document
        .path
        .as_ref()
        .expect("LSP didChange requires a file-backed document");
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path),
                "version": document_version(document)
            },
            "contentChanges": edits.iter().map(lsp_content_change_for_edit).collect::<Vec<_>>()
        }
    })
}

fn lsp_content_change_for_edit(edit: &TextEdit) -> Value {
    json!({
        "range": {
            "start": {
                "line": edit.start_line.saturating_sub(1),
                "character": edit.start_column.saturating_sub(1)
            },
            "end": {
                "line": edit.end_line.saturating_sub(1),
                "character": edit.end_column.saturating_sub(1)
            }
        },
        "text": edit.text
    })
}

pub fn did_save_notification(document: &DocumentSnapshot) -> Value {
    let path = document
        .path
        .as_ref()
        .expect("LSP didSave requires a file-backed document");
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didSave",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path)
            },
            "text": document.text
        }
    })
}

pub fn did_close_notification(path: &Path) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didClose",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path)
            }
        }
    })
}

pub fn hover_request(id: u64, path: &Path, line: u32, column: u32) -> Value {
    text_document_position_request(id, "textDocument/hover", path, line, column)
}

pub fn definition_request(id: u64, path: &Path, line: u32, column: u32) -> Value {
    text_document_position_request(id, "textDocument/definition", path, line, column)
}

pub fn references_request(id: u64, path: &Path, line: u32, column: u32) -> Value {
    let mut request =
        text_document_position_request(id, "textDocument/references", path, line, column);
    request["params"]["context"] = json!({
        "includeDeclaration": true
    });
    request
}

pub fn document_symbol_request(id: u64, path: &Path) -> Value {
    text_document_request(id, "textDocument/documentSymbol", path)
}

pub fn workspace_symbol_request(id: u64, query: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "workspace/symbol",
        "params": {
            "query": query
        }
    })
}

pub fn folding_range_request(id: u64, path: &Path) -> Value {
    text_document_request(id, "textDocument/foldingRange", path)
}

pub fn inlay_hint_request(id: u64, path: &Path, range: &LspRange) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "textDocument/inlayHint",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path)
            },
            "range": lsp_range_value(range)
        }
    })
}

pub fn semantic_tokens_full_request(id: u64, path: &Path) -> Value {
    text_document_request(id, "textDocument/semanticTokens/full", path)
}

pub fn rename_request(id: u64, path: &Path, line: u32, column: u32, new_name: &str) -> Value {
    let mut request = text_document_position_request(id, "textDocument/rename", path, line, column);
    request["params"]["newName"] = json!(new_name);
    request
}

pub fn completion_request(id: u64, path: &Path, line: u32, column: u32) -> Value {
    let mut request =
        text_document_position_request(id, "textDocument/completion", path, line, column);
    request["params"]["context"] = json!({
        "triggerKind": 1
    });
    request
}

pub fn signature_help_request(id: u64, path: &Path, line: u32, column: u32) -> Value {
    let mut request =
        text_document_position_request(id, "textDocument/signatureHelp", path, line, column);
    request["params"]["context"] = json!({
        "triggerKind": 1,
        "isRetrigger": false
    });
    request
}

pub fn code_action_request(
    id: u64,
    path: &Path,
    range: &LspRange,
    diagnostics: &[LspCodeActionDiagnostic],
    only: Option<&[String]>,
    trigger: LspCodeActionTrigger,
) -> Value {
    let mut context = json!({
        "diagnostics": diagnostics.iter().map(lsp_diagnostic_for_code_action).collect::<Vec<_>>(),
        "triggerKind": lsp_code_action_trigger_value(trigger),
    });
    if let Some(only) = only.filter(|items| !items.is_empty()) {
        context["only"] = json!(only);
    }

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path)
            },
            "range": lsp_range_value(range),
            "context": context,
        }
    })
}

pub fn formatting_request(id: u64, path: &Path, options: LspFormattingOptions) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "textDocument/formatting",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path)
            },
            "options": lsp_formatting_options_value(options),
        }
    })
}

pub fn range_formatting_request(
    id: u64,
    path: &Path,
    range: &LspRange,
    options: LspFormattingOptions,
) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "textDocument/rangeFormatting",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path)
            },
            "range": lsp_range_value(range),
            "options": lsp_formatting_options_value(options),
        }
    })
}

fn lsp_range_value(range: &LspRange) -> Value {
    json!({
        "start": {
            "line": range.start_line.saturating_sub(1),
            "character": range.start_column.saturating_sub(1),
        },
        "end": {
            "line": range.end_line.saturating_sub(1),
            "character": range.end_column.saturating_sub(1),
        }
    })
}

fn lsp_formatting_options_value(options: LspFormattingOptions) -> Value {
    json!({
        "tabSize": options.tab_size,
        "insertSpaces": options.insert_spaces,
    })
}

fn lsp_code_action_trigger_value(trigger: LspCodeActionTrigger) -> u8 {
    match trigger {
        LspCodeActionTrigger::Invoke => 1,
        LspCodeActionTrigger::Automatic => 2,
    }
}

fn lsp_diagnostic_for_code_action(diagnostic: &LspCodeActionDiagnostic) -> Value {
    let mut value = json!({
        "range": lsp_range_value(&diagnostic.range),
        "message": diagnostic.message.clone(),
    });
    if let Some(severity) = diagnostic.severity.and_then(lsp_diagnostic_severity_value) {
        value["severity"] = json!(severity);
    }
    if let Some(source) = &diagnostic.source {
        value["source"] = json!(source);
    }
    value
}

fn lsp_diagnostic_severity_value(severity: DiagnosticSeverity) -> Option<u8> {
    Some(match severity {
        DiagnosticSeverity::Error => 1,
        DiagnosticSeverity::Warning => 2,
        DiagnosticSeverity::Information => 3,
        DiagnosticSeverity::Hint => 4,
    })
}

fn text_document_position_request(
    id: u64,
    method: &str,
    path: &Path,
    line: u32,
    column: u32,
) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path)
            },
            "position": {
                "line": line.saturating_sub(1),
                "character": column.saturating_sub(1)
            }
        }
    })
}

fn text_document_request(id: u64, method: &str, path: &Path) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path)
            }
        }
    })
}

pub fn shutdown_request(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "shutdown",
        "params": null
    })
}

pub fn exit_notification() -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "exit",
        "params": null
    })
}

pub fn path_to_file_uri(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let absolute_path = if cfg!(windows) && normalized.as_bytes().get(1) == Some(&b':') {
        format!("/{normalized}")
    } else {
        normalized
    };
    format!("file://{}", percent_encode_path(&absolute_path))
}

pub fn workspace_language_servers(root: impl AsRef<Path>) -> AppResult<Vec<LanguageServerInfo>> {
    let root = root.as_ref().canonicalize()?;
    let detected_extensions = detect_extensions(&root)?;
    let mut servers = Vec::new();

    for server in BUILTIN_SERVERS {
        if !server
            .extensions
            .iter()
            .any(|extension| detected_extensions.contains(*extension))
        {
            continue;
        }

        let available = command_available(server.command);
        servers.push(LanguageServerInfo {
            language_id: server.language_id.to_string(),
            name: server.name.to_string(),
            command: server.command.to_string(),
            args: server.args.iter().map(|arg| (*arg).to_string()).collect(),
            workspace_root: root.clone(),
            status: if available {
                LanguageServerStatus::Available
            } else {
                LanguageServerStatus::Missing
            },
            error: if available {
                None
            } else {
                Some(format!("{} was not found in PATH", server.command))
            },
        });
    }

    Ok(servers)
}

pub fn language_server_diagnostics(servers: &[LanguageServerInfo]) -> Vec<WorkspaceDiagnostic> {
    servers
        .iter()
        .filter(|server| server.status == LanguageServerStatus::Missing)
        .map(|server| WorkspaceDiagnostic {
            path: diagnostic_anchor_path(server),
            line: 1,
            column: 1,
            severity: DiagnosticSeverity::Warning,
            source: "lux-lsp".to_string(),
            message: server
                .error
                .clone()
                .unwrap_or_else(|| format!("{} is configured but unavailable", server.command)),
        })
        .collect()
}

pub fn drain_lsp_frames(buffer: &mut Vec<u8>) -> AppResult<Vec<LspFrame>> {
    let mut frames = Vec::new();

    while let Some(header_end) = find_header_end(buffer) {
        let headers = std::str::from_utf8(&buffer[..header_end])
            .map_err(|error| AppError::Service(format!("invalid LSP header encoding: {error}")))?;
        let content_length = parse_content_length(headers)?;
        let frame_start = header_end + 4;
        let frame_end = frame_start + content_length;

        if buffer.len() < frame_end {
            break;
        }

        let content = buffer[frame_start..frame_end].to_vec();
        buffer.drain(..frame_end);
        frames.push(LspFrame { content });
    }

    Ok(frames)
}

pub fn parse_lsp_notification(frame: &LspFrame) -> AppResult<Option<LspNotification>> {
    let value: Value = serde_json::from_slice(&frame.content)?;
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return Ok(None);
    };

    if method != "textDocument/publishDiagnostics" {
        return Ok(Some(LspNotification::Other {
            method: method.to_string(),
        }));
    }

    let params = value.get("params").cloned().ok_or_else(|| {
        AppError::Service("publishDiagnostics notification is missing params".into())
    })?;
    let params: PublishDiagnosticsParams = serde_json::from_value(params)?;
    Ok(Some(LspNotification::PublishDiagnostics(
        diagnostics_update_from_publish(params),
    )))
}

pub fn parse_lsp_response(frame: &LspFrame) -> AppResult<Option<u64>> {
    let value: Value = serde_json::from_slice(&frame.content)?;
    Ok(parse_lsp_response_value(&value).map(|response| response.id))
}

pub fn diagnostics_update_from_publish(params: PublishDiagnosticsParams) -> DiagnosticsUpdate {
    let path = uri_to_path(&params.uri).unwrap_or_else(|| PathBuf::from(params.uri.as_str()));
    let diagnostics = workspace_diagnostics_for_path(path.clone(), params.diagnostics);
    DiagnosticsUpdate { path, diagnostics }
}

pub fn workspace_diagnostics_from_publish(
    params: PublishDiagnosticsParams,
) -> Vec<WorkspaceDiagnostic> {
    let path = uri_to_path(&params.uri).unwrap_or_else(|| PathBuf::from(params.uri.as_str()));

    workspace_diagnostics_for_path(path, params.diagnostics)
}

pub fn parse_hover_result(value: &Value) -> Option<LspHover> {
    if value.is_null() {
        return None;
    }
    let contents_value = value.get("contents").unwrap_or(value);
    let contents = markdown_strings_from_markup(contents_value);
    if contents.is_empty() {
        return None;
    }
    Some(LspHover {
        contents,
        range: value.get("range").and_then(lsp_range_from_value),
    })
}

pub fn parse_definition_result(value: &Value) -> Vec<LspLocation> {
    if value.is_null() {
        return Vec::new();
    }

    let values = match value {
        Value::Array(items) => items.iter().collect::<Vec<_>>(),
        _ => vec![value],
    };

    values
        .into_iter()
        .filter_map(lsp_location_from_value)
        .collect()
}

pub fn parse_completion_result(value: &Value) -> LspCompletionList {
    match value {
        Value::Array(items) => LspCompletionList {
            is_incomplete: false,
            items: items.iter().filter_map(parse_completion_item).collect(),
        },
        Value::Object(object) => LspCompletionList {
            is_incomplete: object
                .get("isIncomplete")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            items: object
                .get("items")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(parse_completion_item).collect())
                .unwrap_or_default(),
        },
        _ => empty_completion_list(),
    }
}

pub fn parse_document_symbol_result(value: &Value) -> Vec<LspDocumentSymbol> {
    value
        .as_array()
        .map(|symbols| {
            symbols
                .iter()
                .filter_map(parse_document_symbol_item)
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_workspace_symbol_result(value: &Value) -> Vec<LspWorkspaceSymbol> {
    value
        .as_array()
        .map(|symbols| {
            symbols
                .iter()
                .filter_map(parse_workspace_symbol_item)
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_folding_range_result(value: &Value) -> Vec<LspFoldingRange> {
    value
        .as_array()
        .map(|ranges| ranges.iter().filter_map(parse_folding_range_item).collect())
        .unwrap_or_default()
}

pub fn parse_semantic_token_legend_from_initialize(value: &Value) -> Option<SemanticTokenLegend> {
    let legend = value
        .get("capabilities")?
        .get("semanticTokensProvider")?
        .get("legend")?;
    let token_types = legend
        .get("tokenTypes")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let token_modifiers = legend
        .get("tokenModifiers")
        .and_then(Value::as_array)
        .map(|modifiers| {
            modifiers
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    (!token_types.is_empty()).then_some(SemanticTokenLegend {
        token_types,
        token_modifiers,
    })
}

pub fn parse_inlay_hint_result(value: &Value) -> Vec<LspInlayHint> {
    value
        .as_array()
        .map(|hints| hints.iter().filter_map(parse_inlay_hint_item).collect())
        .unwrap_or_default()
}

pub fn parse_semantic_tokens_result(
    value: &Value,
    legend: &SemanticTokenLegend,
) -> Option<LspSemanticTokens> {
    let data = value
        .get("data")?
        .as_array()?
        .iter()
        .filter_map(|value| value.as_u64().and_then(|value| u32::try_from(value).ok()))
        .collect::<Vec<_>>();
    if data.is_empty() || data.len() % 5 != 0 {
        return None;
    }

    Some(LspSemanticTokens {
        result_id: value
            .get("resultId")
            .and_then(Value::as_str)
            .map(str::to_string),
        data: remap_semantic_token_data(&data, legend),
    })
}

pub fn parse_signature_help_result(value: &Value) -> Option<LspSignatureHelp> {
    if value.is_null() {
        return None;
    }
    let signatures = value
        .get("signatures")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(parse_signature_information)
        .collect::<Vec<_>>();
    if signatures.is_empty() {
        return None;
    }

    Some(LspSignatureHelp {
        signatures,
        active_signature: value.get("activeSignature").and_then(value_to_u32),
        active_parameter: value.get("activeParameter").and_then(value_to_u32),
    })
}

pub fn parse_workspace_edit_result(value: &Value) -> Option<LspWorkspaceEdit> {
    if value.is_null() {
        return None;
    }

    let mut files = BTreeMap::<PathBuf, Vec<LspTextEdit>>::new();
    if let Some(changes) = value.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            let Ok(uri) = uri.parse::<Uri>() else {
                continue;
            };
            let Some(path) = uri_to_path(&uri) else {
                continue;
            };
            let Some(edits) = edits.as_array() else {
                continue;
            };
            files
                .entry(path)
                .or_default()
                .extend(edits.iter().filter_map(lsp_text_edit_from_value));
        }
    }

    if let Some(document_changes) = value.get("documentChanges").and_then(Value::as_array) {
        for change in document_changes {
            let Some(text_document) = change.get("textDocument") else {
                continue;
            };
            let Some(uri) = text_document
                .get("uri")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<Uri>().ok())
            else {
                continue;
            };
            let Some(path) = uri_to_path(&uri) else {
                continue;
            };
            let Some(edits) = change.get("edits").and_then(Value::as_array) else {
                continue;
            };
            files
                .entry(path)
                .or_default()
                .extend(edits.iter().filter_map(lsp_text_edit_from_value));
        }
    }

    let files = files
        .into_iter()
        .filter_map(|(path, edits)| {
            (!edits.is_empty()).then_some(LspWorkspaceEditFile { path, edits })
        })
        .collect::<Vec<_>>();

    (!files.is_empty()).then_some(LspWorkspaceEdit { files })
}

pub fn parse_text_edits_result(value: &Value) -> Vec<LspTextEdit> {
    value
        .as_array()
        .map(|edits| edits.iter().filter_map(lsp_text_edit_from_value).collect())
        .unwrap_or_default()
}

pub fn parse_code_action_result(value: &Value) -> Vec<LspCodeAction> {
    value
        .as_array()
        .map(|actions| actions.iter().filter_map(parse_code_action).collect())
        .unwrap_or_default()
}

fn parse_code_action(value: &Value) -> Option<LspCodeAction> {
    let title = value.get("title")?.as_str()?.to_string();
    let disabled_reason = value
        .get("disabled")
        .and_then(|disabled| disabled.get("reason"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let edit = value.get("edit").and_then(parse_workspace_edit_result);
    if edit.is_none() && disabled_reason.is_none() {
        return None;
    }

    Some(LspCodeAction {
        title,
        kind: value
            .get("kind")
            .and_then(Value::as_str)
            .map(str::to_string),
        is_preferred: value
            .get("isPreferred")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        disabled_reason,
        edit,
    })
}

fn parse_document_symbol_item(value: &Value) -> Option<LspDocumentSymbol> {
    let name = value.get("name")?.as_str()?.to_string();
    let kind = value
        .get("kind")
        .and_then(parse_symbol_kind)
        .unwrap_or(LspSymbolKind::Variable);

    if let Some(selection_range) = value.get("selectionRange").and_then(lsp_range_from_value) {
        let range = value
            .get("range")
            .and_then(lsp_range_from_value)
            .unwrap_or_else(|| selection_range.clone());
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .map(|children| {
                children
                    .iter()
                    .filter_map(parse_document_symbol_item)
                    .collect()
            })
            .unwrap_or_default();
        return Some(LspDocumentSymbol {
            name,
            detail: value
                .get("detail")
                .and_then(Value::as_str)
                .map(str::to_string),
            kind,
            range,
            selection_range,
            children,
        });
    }

    let location = value.get("location").and_then(lsp_location_from_value)?;
    Some(LspDocumentSymbol {
        name,
        detail: value
            .get("containerName")
            .and_then(Value::as_str)
            .map(str::to_string),
        kind,
        range: location.range.clone(),
        selection_range: location.range,
        children: Vec::new(),
    })
}

fn parse_workspace_symbol_item(value: &Value) -> Option<LspWorkspaceSymbol> {
    let name = value.get("name")?.as_str()?.to_string();
    let kind = value
        .get("kind")
        .and_then(parse_symbol_kind)
        .unwrap_or(LspSymbolKind::Variable);
    let location = value
        .get("location")
        .and_then(lsp_location_from_value)
        .or_else(|| {
            let uri = value
                .get("location")?
                .get("uri")
                .or_else(|| value.get("uri"))?
                .as_str()?
                .parse::<Uri>()
                .ok()?;
            let path = uri_to_path(&uri)?;
            let range = value
                .get("location")?
                .get("range")
                .or_else(|| value.get("range"))
                .and_then(lsp_range_from_value)?;
            Some(LspLocation { path, range })
        })?;
    Some(LspWorkspaceSymbol {
        name,
        kind,
        location,
        container_name: value
            .get("containerName")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn parse_folding_range_item(value: &Value) -> Option<LspFoldingRange> {
    Some(LspFoldingRange {
        start_line: one_based_lsp_position_value(value, "startLine")?,
        end_line: one_based_lsp_position_value(value, "endLine")?,
        start_column: value
            .get("startCharacter")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .map(|value| value.saturating_add(1)),
        end_column: value
            .get("endCharacter")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .map(|value| value.saturating_add(1)),
        kind: value
            .get("kind")
            .and_then(Value::as_str)
            .and_then(parse_folding_range_kind),
    })
}

fn parse_inlay_hint_item(value: &Value) -> Option<LspInlayHint> {
    let position = value.get("position")?;
    Some(LspInlayHint {
        label: inlay_hint_label(value.get("label")?)?,
        tooltip: value.get("tooltip").and_then(markup_to_markdown),
        line: one_based_lsp_position_value(position, "line")?,
        column: one_based_lsp_position_value(position, "character")?,
        kind: value.get("kind").and_then(parse_inlay_hint_kind),
        padding_left: value
            .get("paddingLeft")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        padding_right: value
            .get("paddingRight")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn inlay_hint_label(value: &Value) -> Option<String> {
    match value {
        Value::String(label) => Some(label.clone()),
        Value::Array(parts) => {
            let label = parts
                .iter()
                .filter_map(|part| {
                    part.get("value")
                        .or_else(|| part.get("label"))
                        .and_then(Value::as_str)
                })
                .collect::<String>();
            (!label.is_empty()).then_some(label)
        }
        _ => None,
    }
}

fn parse_inlay_hint_kind(value: &Value) -> Option<LspInlayHintKind> {
    Some(match value.as_u64()? {
        1 => LspInlayHintKind::Type,
        2 => LspInlayHintKind::Parameter,
        _ => return None,
    })
}

fn remap_semantic_token_data(data: &[u32], legend: &SemanticTokenLegend) -> Vec<u32> {
    let type_indexes = semantic_token_type_indexes(legend);
    let modifier_masks = semantic_token_modifier_masks(legend);
    let mut remapped = Vec::with_capacity(data.len());

    for chunk in data.chunks_exact(5) {
        remapped.extend_from_slice(&chunk[..3]);
        let token_type = usize::try_from(chunk[3])
            .ok()
            .and_then(|index| type_indexes.get(index).copied())
            .flatten()
            .unwrap_or(0);
        let token_modifiers = remap_semantic_token_modifier_mask(chunk[4], &modifier_masks);
        remapped.push(token_type);
        remapped.push(token_modifiers);
    }

    remapped
}

fn semantic_token_type_indexes(legend: &SemanticTokenLegend) -> Vec<Option<u32>> {
    legend
        .token_types
        .iter()
        .map(|token_type| {
            CLIENT_SEMANTIC_TOKEN_TYPES
                .iter()
                .position(|candidate| candidate == token_type)
                .and_then(|index| u32::try_from(index).ok())
        })
        .collect()
}

fn semantic_token_modifier_masks(legend: &SemanticTokenLegend) -> Vec<u32> {
    legend
        .token_modifiers
        .iter()
        .map(|modifier| {
            CLIENT_SEMANTIC_TOKEN_MODIFIERS
                .iter()
                .position(|candidate| candidate == modifier)
                .and_then(|index| u32::try_from(index).ok())
                .map(|index| 1_u32.checked_shl(index).unwrap_or(0))
                .unwrap_or(0)
        })
        .collect()
}

fn remap_semantic_token_modifier_mask(source_mask: u32, modifier_masks: &[u32]) -> u32 {
    modifier_masks
        .iter()
        .enumerate()
        .fold(0_u32, |acc, (index, target_mask)| {
            let Ok(index) = u32::try_from(index) else {
                return acc;
            };
            let Some(source_bit) = 1_u32.checked_shl(index) else {
                return acc;
            };
            if source_mask & source_bit != 0 {
                acc | target_mask
            } else {
                acc
            }
        })
}

fn parse_symbol_kind(value: &Value) -> Option<LspSymbolKind> {
    Some(match value.as_u64()? {
        1 => LspSymbolKind::File,
        2 => LspSymbolKind::Module,
        3 => LspSymbolKind::Namespace,
        4 => LspSymbolKind::Package,
        5 => LspSymbolKind::Class,
        6 => LspSymbolKind::Method,
        7 => LspSymbolKind::Property,
        8 => LspSymbolKind::Field,
        9 => LspSymbolKind::Constructor,
        10 => LspSymbolKind::Enum,
        11 => LspSymbolKind::Interface,
        12 => LspSymbolKind::Function,
        13 => LspSymbolKind::Variable,
        14 => LspSymbolKind::Constant,
        15 => LspSymbolKind::String,
        16 => LspSymbolKind::Number,
        17 => LspSymbolKind::Boolean,
        18 => LspSymbolKind::Array,
        19 => LspSymbolKind::Object,
        20 => LspSymbolKind::Key,
        21 => LspSymbolKind::Null,
        22 => LspSymbolKind::EnumMember,
        23 => LspSymbolKind::Struct,
        24 => LspSymbolKind::Event,
        25 => LspSymbolKind::Operator,
        26 => LspSymbolKind::TypeParameter,
        _ => return None,
    })
}

fn parse_folding_range_kind(value: &str) -> Option<LspFoldingRangeKind> {
    Some(match value {
        "comment" => LspFoldingRangeKind::Comment,
        "imports" => LspFoldingRangeKind::Imports,
        "region" => LspFoldingRangeKind::Region,
        _ => return None,
    })
}

fn empty_completion_list() -> LspCompletionList {
    LspCompletionList {
        is_incomplete: false,
        items: Vec::new(),
    }
}

fn workspace_diagnostics_for_path(
    path: PathBuf,
    diagnostics: Vec<Diagnostic>,
) -> Vec<WorkspaceDiagnostic> {
    diagnostics
        .into_iter()
        .map(|diagnostic| WorkspaceDiagnostic {
            path: path.clone(),
            line: diagnostic.range.start.line.saturating_add(1),
            column: diagnostic.range.start.character.saturating_add(1),
            severity: diagnostic
                .severity
                .map(map_lsp_severity)
                .unwrap_or(DiagnosticSeverity::Information),
            source: diagnostic.source.unwrap_or_else(|| "lsp".to_string()),
            message: diagnostic.message,
        })
        .collect()
}

fn lsp_location_from_value(value: &Value) -> Option<LspLocation> {
    let candidate = value
        .get("targetUri")
        .is_some()
        .then(|| {
            (
                value.get("targetUri"),
                value
                    .get("targetSelectionRange")
                    .or_else(|| value.get("targetRange")),
            )
        })
        .unwrap_or_else(|| (value.get("uri"), value.get("range")));
    let uri = candidate.0?.as_str()?.parse::<Uri>().ok()?;
    let path = uri_to_path(&uri)?;
    let range = lsp_range_from_value(candidate.1?)?;
    Some(LspLocation { path, range })
}

fn lsp_text_edit_from_value(value: &Value) -> Option<LspTextEdit> {
    Some(LspTextEdit {
        range: lsp_range_from_value(value.get("range")?)?,
        text: value.get("newText")?.as_str()?.to_string(),
    })
}

fn parse_completion_item(value: &Value) -> Option<LspCompletionItem> {
    let label = value.get("label")?.as_str()?.to_string();
    let text_edit = value.get("textEdit").and_then(parse_completion_text_edit);
    let insert_text = text_edit
        .as_ref()
        .map(|(_, text)| text.clone())
        .or_else(|| {
            value
                .get("insertText")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| label.clone());

    Some(LspCompletionItem {
        label,
        kind: value.get("kind").and_then(parse_completion_item_kind),
        detail: value
            .get("detail")
            .and_then(Value::as_str)
            .map(str::to_string),
        documentation: value
            .get("documentation")
            .and_then(parse_completion_documentation),
        insert_text,
        insert_text_format: value
            .get("insertTextFormat")
            .and_then(Value::as_u64)
            .map(parse_insert_text_format)
            .unwrap_or(LspInsertTextFormat::PlainText),
        filter_text: value
            .get("filterText")
            .and_then(Value::as_str)
            .map(str::to_string),
        sort_text: value
            .get("sortText")
            .and_then(Value::as_str)
            .map(str::to_string),
        range: text_edit.map(|(range, _)| range),
        commit_characters: value
            .get("commitCharacters")
            .and_then(Value::as_array)
            .map(|characters| {
                characters
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        preselect: value
            .get("preselect")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn parse_signature_information(value: &Value) -> Option<LspSignatureInformation> {
    let label = value.get("label")?.as_str()?.to_string();
    let parameters = value
        .get("parameters")
        .and_then(Value::as_array)
        .map(|parameters| {
            parameters
                .iter()
                .filter_map(|parameter| parse_signature_parameter(parameter, &label))
                .collect()
        })
        .unwrap_or_default();

    Some(LspSignatureInformation {
        label,
        documentation: value.get("documentation").and_then(markup_to_markdown),
        parameters,
        active_parameter: value.get("activeParameter").and_then(value_to_u32),
    })
}

fn parse_signature_parameter(
    value: &Value,
    signature_label: &str,
) -> Option<LspSignatureParameter> {
    let label = value
        .get("label")
        .and_then(|label| signature_parameter_label(label, signature_label))?;
    Some(LspSignatureParameter {
        label,
        documentation: value.get("documentation").and_then(markup_to_markdown),
    })
}

fn signature_parameter_label(value: &Value, signature_label: &str) -> Option<String> {
    if let Some(label) = value.as_str() {
        return Some(label.to_string());
    }
    let range = value.as_array()?;
    if range.len() != 2 {
        return None;
    }
    let start = usize::try_from(range[0].as_u64()?).ok()?;
    let end = usize::try_from(range[1].as_u64()?).ok()?;
    if start > end
        || end > signature_label.len()
        || !signature_label.is_char_boundary(start)
        || !signature_label.is_char_boundary(end)
    {
        return None;
    }
    Some(signature_label[start..end].to_string())
}

fn parse_completion_text_edit(value: &Value) -> Option<(LspRange, String)> {
    let range = value
        .get("range")
        .or_else(|| value.get("replace"))
        .and_then(lsp_range_from_value)?;
    let text = value
        .get("newText")
        .or_else(|| value.get("insertText"))
        .and_then(Value::as_str)?
        .to_string();
    Some((range, text))
}

fn parse_completion_documentation(value: &Value) -> Option<String> {
    markup_to_markdown(value)
}

fn markup_to_markdown(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => non_empty_markdown(text),
        Value::Object(object) => object
            .get("value")
            .and_then(Value::as_str)
            .and_then(non_empty_markdown),
        _ => None,
    }
}

fn parse_insert_text_format(value: u64) -> LspInsertTextFormat {
    if value == 2 {
        LspInsertTextFormat::Snippet
    } else {
        LspInsertTextFormat::PlainText
    }
}

fn parse_completion_item_kind(value: &Value) -> Option<LspCompletionItemKind> {
    let kind = value.as_u64()?;
    Some(match kind {
        1 => LspCompletionItemKind::Text,
        2 => LspCompletionItemKind::Method,
        3 => LspCompletionItemKind::Function,
        4 => LspCompletionItemKind::Constructor,
        5 => LspCompletionItemKind::Field,
        6 => LspCompletionItemKind::Variable,
        7 => LspCompletionItemKind::Class,
        8 => LspCompletionItemKind::Interface,
        9 => LspCompletionItemKind::Module,
        10 => LspCompletionItemKind::Property,
        11 => LspCompletionItemKind::Unit,
        12 => LspCompletionItemKind::Value,
        13 => LspCompletionItemKind::Enum,
        14 => LspCompletionItemKind::Keyword,
        15 => LspCompletionItemKind::Snippet,
        16 => LspCompletionItemKind::Color,
        17 => LspCompletionItemKind::File,
        18 => LspCompletionItemKind::Reference,
        19 => LspCompletionItemKind::Folder,
        20 => LspCompletionItemKind::EnumMember,
        21 => LspCompletionItemKind::Constant,
        22 => LspCompletionItemKind::Struct,
        23 => LspCompletionItemKind::Event,
        24 => LspCompletionItemKind::Operator,
        25 => LspCompletionItemKind::TypeParameter,
        _ => return None,
    })
}

fn lsp_range_from_value(value: &Value) -> Option<LspRange> {
    let start = value.get("start")?;
    let end = value.get("end")?;
    Some(LspRange {
        start_line: one_based_lsp_position_value(start, "line")?,
        start_column: one_based_lsp_position_value(start, "character")?,
        end_line: one_based_lsp_position_value(end, "line")?,
        end_column: one_based_lsp_position_value(end, "character")?,
    })
}

fn one_based_lsp_position_value(value: &Value, key: &str) -> Option<u32> {
    let raw = value.get(key)?.as_u64()?;
    Some(
        u32::try_from(raw)
            .unwrap_or(u32::MAX.saturating_sub(1))
            .saturating_add(1),
    )
}

fn value_to_u32(value: &Value) -> Option<u32> {
    u32::try_from(value.as_u64()?).ok()
}

fn markdown_strings_from_markup(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => non_empty_markdown(text).into_iter().collect(),
        Value::Array(items) => items
            .iter()
            .flat_map(markdown_strings_from_markup)
            .collect(),
        Value::Object(object) => {
            if let Some((language, text)) = object
                .get("language")
                .and_then(Value::as_str)
                .zip(object.get("value").and_then(Value::as_str))
            {
                non_empty_markdown(&format!("```{language}\n{text}\n```"))
                    .into_iter()
                    .collect()
            } else if let Some(value) = object.get("value").and_then(Value::as_str) {
                non_empty_markdown(value).into_iter().collect()
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn non_empty_markdown(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

async fn read_lsp_stdout<R>(
    mut stdout: R,
    diagnostics_tx: mpsc::UnboundedSender<DiagnosticsUpdate>,
    response_tx: mpsc::UnboundedSender<LspResponse>,
) where
    R: AsyncRead + Unpin,
{
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8192];

    loop {
        let read = match stdout.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => read,
            Err(_) => break,
        };
        buffer.extend_from_slice(&chunk[..read]);

        let frames = match drain_lsp_frames(&mut buffer) {
            Ok(frames) => frames,
            Err(_) => {
                buffer.clear();
                continue;
            }
        };

        for frame in frames {
            if let Ok(Some(notification)) = parse_lsp_notification(&frame) {
                if let LspNotification::PublishDiagnostics(update) = notification {
                    let _ = diagnostics_tx.send(update);
                }
                continue;
            }

            if let Ok(value) = serde_json::from_slice::<Value>(&frame.content) {
                if let Some(response) = parse_lsp_response_value(&value) {
                    let _ = response_tx.send(response);
                }
            }
        }
    }
}

async fn drain_stderr<R>(mut stderr: R)
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 4096];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

fn parse_lsp_response_value(value: &Value) -> Option<LspResponse> {
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return None;
    }
    if value.get("method").is_some() {
        return None;
    }

    let id = value.get("id")?.as_u64()?;
    let error = value.get("error").map(lsp_error_to_string);
    let result = value.get("result").cloned();
    Some(LspResponse { id, error, result })
}

fn lsp_error_to_string(value: &Value) -> String {
    value
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn session_language_id(language_id: &str) -> &str {
    match language_id {
        "javascript" => "typescript",
        other => other,
    }
}

fn language_id_for_path(path: &Path) -> String {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
    {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "json" => "json",
        "toml" => "toml",
        "md" => "markdown",
        "css" => "css",
        "html" => "html",
        other if !other.is_empty() => other,
        _ => "plaintext",
    }
    .to_string()
}

fn lsp_language_id(document: &DocumentSnapshot) -> &str {
    match document.language_id.as_str() {
        "javascript" => "javascript",
        "typescript" => "typescript",
        other => other,
    }
}

fn document_version(document: &DocumentSnapshot) -> i32 {
    i32::try_from(document.version).unwrap_or(i32::MAX)
}

fn percent_encode_path(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                encoded.push(char::from(byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }

    encoded
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &str) -> AppResult<usize> {
    for line in headers.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse::<usize>().map_err(|error| {
                AppError::Service(format!("invalid LSP Content-Length: {error}"))
            });
        }
    }

    Err(AppError::Service(
        "LSP frame is missing Content-Length header".into(),
    ))
}

fn map_lsp_severity(severity: lsp_types::DiagnosticSeverity) -> DiagnosticSeverity {
    if severity == lsp_types::DiagnosticSeverity::ERROR {
        DiagnosticSeverity::Error
    } else if severity == lsp_types::DiagnosticSeverity::WARNING {
        DiagnosticSeverity::Warning
    } else if severity == lsp_types::DiagnosticSeverity::HINT {
        DiagnosticSeverity::Hint
    } else {
        DiagnosticSeverity::Information
    }
}

fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let value = uri.as_str();
    let path = value.strip_prefix("file://")?;
    let decoded = percent_decode(path);

    #[cfg(windows)]
    {
        let without_leading_slash = decoded
            .strip_prefix('/')
            .filter(|candidate| candidate.as_bytes().get(1) == Some(&b':'))
            .unwrap_or(&decoded);
        Some(PathBuf::from(without_leading_slash.replace('/', "\\")))
    }

    #[cfg(not(windows))]
    {
        Some(PathBuf::from(decoded))
    }
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = hex_value(bytes[index + 1]);
            let low = hex_value(bytes[index + 2]);
            if let (Some(high), Some(low)) = (high, low) {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn diagnostic_anchor_path(server: &LanguageServerInfo) -> PathBuf {
    let candidates: &[&str] = match server.language_id.as_str() {
        "rust" => &["rust-toolchain.toml", "Cargo.toml"],
        "typescript" | "javascript" => &["tsconfig.json", "jsconfig.json", "package.json"],
        "json" => &["package.json"],
        _ => &[],
    };

    for candidate in candidates {
        let path = server.workspace_root.join(candidate);
        if path.is_file() {
            return path;
        }
    }

    server.workspace_root.clone()
}

fn detect_extensions(root: &Path) -> AppResult<BTreeSet<String>> {
    let mut extensions = BTreeSet::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(path) = stack.pop() {
        let Ok(children) = std::fs::read_dir(&path) else {
            continue;
        };

        for child in children {
            let child = child?;
            let file_name = child.file_name();
            let file_name = file_name.to_string_lossy();
            if file_name == "node_modules" || file_name == "target" || file_name == ".git" {
                continue;
            }

            let file_type = child.file_type()?;
            if file_type.is_dir() {
                stack.push(child.path());
            } else if file_type.is_file() {
                if let Some(extension) = child.path().extension().and_then(|value| value.to_str()) {
                    extensions.insert(extension.to_ascii_lowercase());
                }
            }
        }
    }

    Ok(extensions)
}

fn command_available(command: &str) -> bool {
    let command_path = Path::new(command);
    if command_path.components().count() > 1 {
        return command_path.is_file();
    }

    let Some(paths) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&paths).any(|path| command_exists_in_dir(&path, command))
}

fn command_exists_in_dir(dir: &Path, command: &str) -> bool {
    let direct = dir.join(command);
    if direct.is_file() {
        return true;
    }

    #[cfg(windows)]
    {
        let extensions = env::var_os("PATHEXT")
            .map(|value| {
                value
                    .to_string_lossy()
                    .split(';')
                    .filter(|extension| !extension.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                vec![
                    ".COM".to_string(),
                    ".EXE".to_string(),
                    ".BAT".to_string(),
                    ".CMD".to_string(),
                ]
            });

        for extension in extensions {
            if dir.join(format!("{command}{extension}")).is_file() {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Diagnostic, Position, Range};

    #[test]
    fn language_server_diagnostics_reports_missing_servers() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let servers = vec![LanguageServerInfo {
            language_id: "typescript".to_string(),
            name: "TypeScript Language Server".to_string(),
            command: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
            workspace_root: root.clone(),
            status: LanguageServerStatus::Missing,
            error: Some("typescript-language-server was not found in PATH".to_string()),
        }];

        let diagnostics = language_server_diagnostics(&servers);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Warning);
        assert_eq!(diagnostics[0].source, "lux-lsp");
        assert_eq!(diagnostics[0].line, 1);
        assert!(diagnostics[0]
            .message
            .contains("typescript-language-server"));
        assert_eq!(diagnostics[0].path, root);
    }

    #[test]
    fn language_server_diagnostics_ignores_available_servers() {
        let servers = vec![LanguageServerInfo {
            language_id: "rust".to_string(),
            name: "rust-analyzer".to_string(),
            command: "rust-analyzer".to_string(),
            args: Vec::new(),
            workspace_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            status: LanguageServerStatus::Available,
            error: None,
        }];

        assert!(language_server_diagnostics(&servers).is_empty());
    }

    #[test]
    fn drain_lsp_frames_extracts_complete_frames_and_keeps_partial_tail() {
        let first = br#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
        let second = br#"{"jsonrpc":"2.0","method":"shutdown"}"#;
        let mut buffer = Vec::new();
        buffer.extend_from_slice(format!("Content-Length: {}\r\n\r\n", first.len()).as_bytes());
        buffer.extend_from_slice(first);
        buffer.extend_from_slice(format!("Content-Length: {}\r\n\r\n", second.len()).as_bytes());
        buffer.extend_from_slice(&second[..8]);

        let frames = drain_lsp_frames(&mut buffer).expect("valid frame should parse");

        assert_eq!(
            frames,
            vec![LspFrame {
                content: first.to_vec()
            }]
        );
        assert_eq!(
            buffer,
            [
                format!("Content-Length: {}\r\n\r\n", second.len()).as_bytes(),
                &second[..8]
            ]
            .concat()
        );
    }

    #[test]
    fn parse_publish_diagnostics_notification_maps_to_workspace_diagnostics() {
        let path = if cfg!(windows) {
            "file:///C:/Users/Test/project/src/main.rs"
        } else {
            "file:///home/test/project/src/main.rs"
        };
        let payload = format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{{"uri":"{path}","diagnostics":[{{"range":{{"start":{{"line":4,"character":2}},"end":{{"line":4,"character":8}}}},"severity":1,"source":"rust-analyzer","message":"expected semicolon"}}]}}}}"#,
        );
        let frame = LspFrame {
            content: payload.into_bytes(),
        };

        let notification = parse_lsp_notification(&frame).expect("notification should parse");

        let Some(LspNotification::PublishDiagnostics(update)) = notification else {
            panic!("expected publishDiagnostics notification");
        };
        let diagnostics = update.diagnostics;
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 5);
        assert_eq!(diagnostics[0].column, 3);
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Error);
        assert_eq!(diagnostics[0].source, "rust-analyzer");
        assert_eq!(diagnostics[0].message, "expected semicolon");
        assert!(diagnostics[0]
            .path
            .to_string_lossy()
            .ends_with(if cfg!(windows) {
                "C:\\Users\\Test\\project\\src\\main.rs"
            } else {
                "/home/test/project/src/main.rs"
            }));
        assert_eq!(diagnostics[0].path, update.path);
    }

    #[test]
    fn diagnostics_update_preserves_path_when_server_clears_file() {
        let uri: Uri = "file:///tmp/example.ts".parse().expect("uri should parse");

        let update =
            diagnostics_update_from_publish(PublishDiagnosticsParams::new(uri, Vec::new(), None));

        assert!(update.diagnostics.is_empty());
        assert!(update.path.to_string_lossy().ends_with(if cfg!(windows) {
            "\\tmp\\example.ts"
        } else {
            "/tmp/example.ts"
        }));
    }

    #[test]
    fn encode_lsp_message_adds_content_length_header() {
        let value = json!({"jsonrpc":"2.0","method":"initialized","params":{}});

        let message = encode_lsp_message(&value).expect("message should encode");
        let message = String::from_utf8(message).expect("message should be utf8");

        assert!(message.starts_with("Content-Length: "));
        assert!(message.contains("\r\n\r\n"));
        assert!(message.ends_with(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#));
    }

    #[test]
    fn request_builders_use_file_uris_and_versions() {
        let workspace_root = if cfg!(windows) {
            PathBuf::from(r"C:\Users\Test Project")
        } else {
            PathBuf::from("/home/test/Test Project")
        };
        let definition = LanguageServerDefinition {
            language_id: "typescript".to_string(),
            command: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
            workspace_root: workspace_root.clone(),
        };
        let document = DocumentSnapshot {
            id: lux_core::BufferId::new(),
            path: Some(workspace_root.join("src").join("file name.ts")),
            title: "file name.ts".to_string(),
            language_id: "typescript".to_string(),
            text: "const value = 1;\n".to_string(),
            version: 7,
            is_dirty: true,
            is_untitled: false,
            opened_at: chrono::Utc::now(),
        };

        let initialize = initialize_request(3, &definition);
        let did_open = did_open_notification(&document);
        let did_change = did_change_notification(&document);
        let did_save = did_save_notification(&document);
        let did_close =
            did_close_notification(document.path.as_ref().expect("test document has path"));

        assert_eq!(initialize["id"], 3);
        assert_eq!(initialize["method"], "initialize");
        assert!(initialize["params"]["rootUri"]
            .as_str()
            .unwrap()
            .starts_with("file://"));
        assert_eq!(
            initialize["params"]["capabilities"]["textDocument"]["references"]
                ["dynamicRegistration"],
            false
        );
        assert_eq!(
            initialize["params"]["capabilities"]["textDocument"]["definition"]["linkSupport"],
            true
        );
        assert_eq!(
            initialize["params"]["capabilities"]["textDocument"]["documentSymbol"]
                ["hierarchicalDocumentSymbolSupport"],
            true
        );
        assert_eq!(
            initialize["params"]["capabilities"]["textDocument"]["foldingRange"]["lineFoldingOnly"],
            true
        );
        assert_eq!(
            initialize["params"]["capabilities"]["textDocument"]["inlayHint"]
                ["dynamicRegistration"],
            false
        );
        assert_eq!(
            initialize["params"]["capabilities"]["textDocument"]["semanticTokens"]["formats"][0],
            "relative"
        );
        assert_eq!(
            initialize["params"]["capabilities"]["textDocument"]["semanticTokens"]["requests"]
                ["full"],
            true
        );
        assert_eq!(
            initialize["params"]["capabilities"]["workspace"]["symbol"]["dynamicRegistration"],
            false
        );
        assert!(did_open["params"]["textDocument"]["uri"]
            .as_str()
            .unwrap()
            .contains("file%20name.ts"));
        assert_eq!(did_open["params"]["textDocument"]["version"], 7);
        assert_eq!(did_change["params"]["textDocument"]["version"], 7);
        assert_eq!(
            did_change["params"]["contentChanges"][0]["text"],
            "const value = 1;\n"
        );
        assert_eq!(did_save["method"], "textDocument/didSave");
        assert_eq!(did_save["params"]["text"], "const value = 1;\n");
        assert_eq!(did_close["method"], "textDocument/didClose");
        assert!(did_close["params"]["textDocument"]["uri"]
            .as_str()
            .unwrap()
            .contains("file%20name.ts"));
    }

    #[test]
    fn did_change_edits_notification_uses_incremental_lsp_ranges() {
        let document = DocumentSnapshot {
            id: lux_core::BufferId::new(),
            path: Some(PathBuf::from("/tmp/main.rs")),
            title: "main.rs".to_string(),
            language_id: "rust".to_string(),
            text: "fn main() {}\n".to_string(),
            version: 12,
            is_dirty: true,
            is_untitled: false,
            opened_at: chrono::Utc::now(),
        };
        let edits = vec![TextEdit {
            start_line: 1,
            start_column: 11,
            end_line: 1,
            end_column: 11,
            text: " println!(\"lux\");".to_string(),
        }];

        let value = did_change_edits_notification(&document, &edits);

        assert_eq!(value["method"], "textDocument/didChange");
        assert_eq!(value["params"]["textDocument"]["version"], 12);
        assert_eq!(
            value["params"]["contentChanges"][0]["range"]["start"]["line"],
            0
        );
        assert_eq!(
            value["params"]["contentChanges"][0]["range"]["start"]["character"],
            10
        );
        assert_eq!(
            value["params"]["contentChanges"][0]["text"],
            " println!(\"lux\");"
        );
        assert!(value["params"]["contentChanges"][0]
            .get("rangeLength")
            .is_none());
    }

    #[test]
    fn navigation_and_assistance_requests_use_zero_based_lsp_positions() {
        let path = PathBuf::from("/tmp/src/main.rs");

        let hover = hover_request(41, &path, 5, 3);
        let definition = definition_request(42, &path, 1, 1);
        let completion = completion_request(43, &path, 2, 4);
        let signature = signature_help_request(44, &path, 9, 12);
        let references = references_request(45, &path, 7, 2);
        let rename = rename_request(46, &path, 11, 6, "renamed_value");
        let document_symbol = document_symbol_request(50, &path);
        let workspace_symbol = workspace_symbol_request(51, "LuxStore");
        let folding_range = folding_range_request(52, &path);
        let semantic_tokens = semantic_tokens_full_request(54, &path);
        let code_action_range = LspRange {
            start_line: 3,
            start_column: 5,
            end_line: 3,
            end_column: 9,
        };
        let inlay_hint = inlay_hint_request(53, &path, &code_action_range);
        let code_action = code_action_request(
            47,
            &path,
            &code_action_range,
            &[LspCodeActionDiagnostic {
                range: code_action_range.clone(),
                severity: Some(DiagnosticSeverity::Warning),
                source: Some("rust-analyzer".to_string()),
                message: "unused import".to_string(),
            }],
            Some(&["quickfix".to_string()]),
            LspCodeActionTrigger::Invoke,
        );
        let formatting = formatting_request(
            48,
            &path,
            LspFormattingOptions {
                tab_size: 4,
                insert_spaces: true,
            },
        );
        let range_formatting = range_formatting_request(
            49,
            &path,
            &code_action_range,
            LspFormattingOptions {
                tab_size: 2,
                insert_spaces: false,
            },
        );

        assert_eq!(hover["id"], 41);
        assert_eq!(hover["method"], "textDocument/hover");
        assert_eq!(hover["params"]["position"]["line"], 4);
        assert_eq!(hover["params"]["position"]["character"], 2);
        assert_eq!(definition["id"], 42);
        assert_eq!(definition["method"], "textDocument/definition");
        assert_eq!(definition["params"]["position"]["line"], 0);
        assert_eq!(definition["params"]["position"]["character"], 0);
        assert_eq!(references["id"], 45);
        assert_eq!(references["method"], "textDocument/references");
        assert_eq!(references["params"]["position"]["line"], 6);
        assert_eq!(references["params"]["position"]["character"], 1);
        assert_eq!(references["params"]["context"]["includeDeclaration"], true);
        assert_eq!(rename["id"], 46);
        assert_eq!(rename["method"], "textDocument/rename");
        assert_eq!(rename["params"]["position"]["line"], 10);
        assert_eq!(rename["params"]["position"]["character"], 5);
        assert_eq!(rename["params"]["newName"], "renamed_value");
        assert_eq!(completion["id"], 43);
        assert_eq!(completion["method"], "textDocument/completion");
        assert_eq!(completion["params"]["position"]["line"], 1);
        assert_eq!(completion["params"]["position"]["character"], 3);
        assert_eq!(completion["params"]["context"]["triggerKind"], 1);
        assert_eq!(signature["id"], 44);
        assert_eq!(signature["method"], "textDocument/signatureHelp");
        assert_eq!(signature["params"]["position"]["line"], 8);
        assert_eq!(signature["params"]["position"]["character"], 11);
        assert_eq!(signature["params"]["context"]["triggerKind"], 1);
        assert_eq!(signature["params"]["context"]["isRetrigger"], false);
        assert_eq!(code_action["id"], 47);
        assert_eq!(code_action["method"], "textDocument/codeAction");
        assert_eq!(code_action["params"]["range"]["start"]["line"], 2);
        assert_eq!(code_action["params"]["range"]["start"]["character"], 4);
        assert_eq!(
            code_action["params"]["context"]["diagnostics"][0]["severity"],
            2
        );
        assert_eq!(
            code_action["params"]["context"]["diagnostics"][0]["message"],
            "unused import"
        );
        assert_eq!(code_action["params"]["context"]["only"][0], "quickfix");
        assert_eq!(code_action["params"]["context"]["triggerKind"], 1);
        assert_eq!(formatting["id"], 48);
        assert_eq!(formatting["method"], "textDocument/formatting");
        assert_eq!(formatting["params"]["options"]["tabSize"], 4);
        assert_eq!(formatting["params"]["options"]["insertSpaces"], true);
        assert_eq!(range_formatting["id"], 49);
        assert_eq!(range_formatting["method"], "textDocument/rangeFormatting");
        assert_eq!(range_formatting["params"]["range"]["end"]["line"], 2);
        assert_eq!(range_formatting["params"]["options"]["insertSpaces"], false);
        assert_eq!(document_symbol["id"], 50);
        assert_eq!(document_symbol["method"], "textDocument/documentSymbol");
        assert!(document_symbol["params"]["textDocument"]["uri"]
            .as_str()
            .unwrap()
            .starts_with("file://"));
        assert_eq!(workspace_symbol["id"], 51);
        assert_eq!(workspace_symbol["method"], "workspace/symbol");
        assert_eq!(workspace_symbol["params"]["query"], "LuxStore");
        assert_eq!(folding_range["id"], 52);
        assert_eq!(folding_range["method"], "textDocument/foldingRange");
        assert!(folding_range["params"]["textDocument"]["uri"]
            .as_str()
            .unwrap()
            .starts_with("file://"));
        assert_eq!(inlay_hint["id"], 53);
        assert_eq!(inlay_hint["method"], "textDocument/inlayHint");
        assert_eq!(inlay_hint["params"]["range"]["start"]["line"], 2);
        assert_eq!(inlay_hint["params"]["range"]["end"]["character"], 8);
        assert_eq!(semantic_tokens["id"], 54);
        assert_eq!(
            semantic_tokens["method"],
            "textDocument/semanticTokens/full"
        );
        assert!(semantic_tokens["params"]["textDocument"]["uri"]
            .as_str()
            .unwrap()
            .starts_with("file://"));
    }

    #[test]
    fn parse_hover_result_accepts_marked_strings_and_range() {
        let value = json!({
            "contents": [
                {"language": "rust", "value": "fn main()"},
                {"kind": "markdown", "value": "Runs the program."}
            ],
            "range": {
                "start": {"line": 2, "character": 4},
                "end": {"line": 2, "character": 8}
            }
        });

        let hover = parse_hover_result(&value).expect("hover should parse");

        assert_eq!(
            hover.contents,
            vec!["```rust\nfn main()\n```", "Runs the program."]
        );
        assert_eq!(
            hover.range.unwrap(),
            LspRange {
                start_line: 3,
                start_column: 5,
                end_line: 3,
                end_column: 9,
            }
        );
    }

    #[test]
    fn parse_definition_result_accepts_locations_and_location_links() {
        let first_uri = if cfg!(windows) {
            "file:///C:/work/project/src/lib.rs"
        } else {
            "file:///work/project/src/lib.rs"
        };
        let second_uri = if cfg!(windows) {
            "file:///C:/work/project/src/main.rs"
        } else {
            "file:///work/project/src/main.rs"
        };
        let value = json!([
            {
                "uri": first_uri,
                "range": {
                    "start": {"line": 9, "character": 2},
                    "end": {"line": 9, "character": 5}
                }
            },
            {
                "targetUri": second_uri,
                "targetRange": {
                    "start": {"line": 1, "character": 0},
                    "end": {"line": 1, "character": 10}
                },
                "targetSelectionRange": {
                    "start": {"line": 1, "character": 3},
                    "end": {"line": 1, "character": 7}
                }
            }
        ]);

        let locations = parse_definition_result(&value);

        assert_eq!(locations.len(), 2);
        assert!(locations[0]
            .path
            .to_string_lossy()
            .ends_with(if cfg!(windows) {
                "C:\\work\\project\\src\\lib.rs"
            } else {
                "/work/project/src/lib.rs"
            }));
        assert_eq!(locations[0].range.start_line, 10);
        assert_eq!(locations[0].range.start_column, 3);
        assert!(locations[1]
            .path
            .to_string_lossy()
            .ends_with(if cfg!(windows) {
                "C:\\work\\project\\src\\main.rs"
            } else {
                "/work/project/src/main.rs"
            }));
        assert_eq!(locations[1].range.start_line, 2);
        assert_eq!(locations[1].range.start_column, 4);
    }

    #[test]
    fn parse_completion_result_accepts_completion_lists_and_text_edits() {
        let value = json!({
            "isIncomplete": true,
            "items": [{
                "label": "println!",
                "kind": 3,
                "detail": "macro println",
                "documentation": {"kind": "markdown", "value": "Prints to stdout."},
                "insertTextFormat": 2,
                "filterText": "println",
                "sortText": "0001",
                "preselect": true,
                "commitCharacters": [";", ")"],
                "textEdit": {
                    "range": {
                        "start": {"line": 4, "character": 8},
                        "end": {"line": 4, "character": 15}
                    },
                    "newText": "println!($1)"
                }
            }]
        });

        let completion = parse_completion_result(&value);

        assert!(completion.is_incomplete);
        assert_eq!(completion.items.len(), 1);
        let item = &completion.items[0];
        assert_eq!(item.label, "println!");
        assert_eq!(item.kind, Some(LspCompletionItemKind::Function));
        assert_eq!(item.detail.as_deref(), Some("macro println"));
        assert_eq!(item.documentation.as_deref(), Some("Prints to stdout."));
        assert_eq!(item.insert_text, "println!($1)");
        assert_eq!(item.insert_text_format, LspInsertTextFormat::Snippet);
        assert_eq!(item.filter_text.as_deref(), Some("println"));
        assert_eq!(item.sort_text.as_deref(), Some("0001"));
        assert_eq!(
            item.commit_characters,
            vec![";".to_string(), ")".to_string()]
        );
        assert!(item.preselect);
        assert_eq!(
            item.range.as_ref().unwrap(),
            &LspRange {
                start_line: 5,
                start_column: 9,
                end_line: 5,
                end_column: 16,
            }
        );
    }

    #[test]
    fn parse_completion_result_accepts_item_arrays_with_defaults() {
        let value = json!([
            {"label": "value"},
            {"label": "ignored invalid kind", "kind": 999}
        ]);

        let completion = parse_completion_result(&value);

        assert!(!completion.is_incomplete);
        assert_eq!(completion.items.len(), 2);
        assert_eq!(completion.items[0].insert_text, "value");
        assert_eq!(
            completion.items[0].insert_text_format,
            LspInsertTextFormat::PlainText
        );
        assert_eq!(completion.items[0].kind, None);
        assert_eq!(completion.items[1].kind, None);
    }

    #[test]
    fn parse_document_symbol_result_accepts_hierarchical_symbols() {
        let value = json!([{
            "name": "EditorArea",
            "detail": "component",
            "kind": 12,
            "range": {
                "start": {"line": 9, "character": 0},
                "end": {"line": 40, "character": 1}
            },
            "selectionRange": {
                "start": {"line": 9, "character": 9},
                "end": {"line": 9, "character": 19}
            },
            "children": [{
                "name": "registerLspProviders",
                "kind": 12,
                "range": {
                    "start": {"line": 20, "character": 0},
                    "end": {"line": 35, "character": 1}
                },
                "selectionRange": {
                    "start": {"line": 20, "character": 9},
                    "end": {"line": 20, "character": 29}
                }
            }]
        }]);

        let symbols = parse_document_symbol_result(&value);

        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "EditorArea");
        assert_eq!(symbols[0].detail.as_deref(), Some("component"));
        assert_eq!(symbols[0].kind, LspSymbolKind::Function);
        assert_eq!(symbols[0].selection_range.start_line, 10);
        assert_eq!(symbols[0].selection_range.start_column, 10);
        assert_eq!(symbols[0].children.len(), 1);
        assert_eq!(symbols[0].children[0].name, "registerLspProviders");
    }

    #[test]
    fn parse_document_symbol_result_accepts_symbol_information() {
        let uri = if cfg!(windows) {
            "file:///C:/work/project/src/lib.rs"
        } else {
            "file:///work/project/src/lib.rs"
        };
        let value = json!([{
            "name": "DocumentStore",
            "kind": 23,
            "containerName": "lux_editor",
            "location": {
                "uri": uri,
                "range": {
                    "start": {"line": 4, "character": 0},
                    "end": {"line": 8, "character": 1}
                }
            }
        }]);

        let symbols = parse_document_symbol_result(&value);

        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "DocumentStore");
        assert_eq!(symbols[0].kind, LspSymbolKind::Struct);
        assert_eq!(symbols[0].detail.as_deref(), Some("lux_editor"));
        assert_eq!(symbols[0].range.start_line, 5);
        assert_eq!(symbols[0].range.end_column, 2);
    }

    #[test]
    fn parse_workspace_symbol_result_accepts_locations() {
        let uri = if cfg!(windows) {
            "file:///C:/work/project/src/store.rs"
        } else {
            "file:///work/project/src/store.rs"
        };
        let value = json!([{
            "name": "LuxStore",
            "kind": 5,
            "containerName": "state",
            "location": {
                "uri": uri,
                "range": {
                    "start": {"line": 2, "character": 4},
                    "end": {"line": 2, "character": 12}
                }
            }
        }]);

        let symbols = parse_workspace_symbol_result(&value);

        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "LuxStore");
        assert_eq!(symbols[0].kind, LspSymbolKind::Class);
        assert_eq!(symbols[0].container_name.as_deref(), Some("state"));
        assert!(symbols[0]
            .location
            .path
            .to_string_lossy()
            .ends_with(if cfg!(windows) {
                "C:\\work\\project\\src\\store.rs"
            } else {
                "/work/project/src/store.rs"
            }));
        assert_eq!(symbols[0].location.range.start_column, 5);
    }

    #[test]
    fn parse_folding_range_result_accepts_kinds_and_optional_columns() {
        let value = json!([
            {"startLine": 0, "endLine": 5, "kind": "imports"},
            {"startLine": 8, "startCharacter": 2, "endLine": 12, "endCharacter": 4, "kind": "region"}
        ]);

        let ranges = parse_folding_range_result(&value);

        assert_eq!(ranges.len(), 2);
        assert_eq!(
            ranges[0],
            LspFoldingRange {
                start_line: 1,
                end_line: 6,
                start_column: None,
                end_column: None,
                kind: Some(LspFoldingRangeKind::Imports),
            }
        );
        assert_eq!(
            ranges[1],
            LspFoldingRange {
                start_line: 9,
                end_line: 13,
                start_column: Some(3),
                end_column: Some(5),
                kind: Some(LspFoldingRangeKind::Region),
            }
        );
    }

    #[test]
    fn parse_semantic_token_legend_from_initialize_extracts_server_legend() {
        let value = json!({
            "capabilities": {
                "semanticTokensProvider": {
                    "legend": {
                        "tokenTypes": ["function", "variable"],
                        "tokenModifiers": ["declaration", "readonly"]
                    },
                    "full": true
                }
            }
        });

        let legend =
            parse_semantic_token_legend_from_initialize(&value).expect("legend should parse");

        assert_eq!(legend.token_types, vec!["function", "variable"]);
        assert_eq!(legend.token_modifiers, vec!["declaration", "readonly"]);
    }

    #[test]
    fn parse_inlay_hint_result_accepts_string_and_part_labels() {
        let value = json!([
            {
                "position": {"line": 3, "character": 8},
                "label": ": String",
                "kind": 1,
                "tooltip": {"kind": "markdown", "value": "Inferred type"},
                "paddingLeft": true
            },
            {
                "position": {"line": 4, "character": 12},
                "label": [{"value": "name"}, {"value": ": "}],
                "kind": 2,
                "paddingRight": true
            }
        ]);

        let hints = parse_inlay_hint_result(&value);

        assert_eq!(hints.len(), 2);
        assert_eq!(
            hints[0],
            LspInlayHint {
                label: ": String".to_string(),
                tooltip: Some("Inferred type".to_string()),
                line: 4,
                column: 9,
                kind: Some(LspInlayHintKind::Type),
                padding_left: true,
                padding_right: false,
            }
        );
        assert_eq!(
            hints[1],
            LspInlayHint {
                label: "name: ".to_string(),
                tooltip: None,
                line: 5,
                column: 13,
                kind: Some(LspInlayHintKind::Parameter),
                padding_left: false,
                padding_right: true,
            }
        );
    }

    #[test]
    fn parse_semantic_tokens_result_remaps_server_legend_to_client_legend() {
        let legend = SemanticTokenLegend {
            token_types: vec!["variable".to_string(), "function".to_string()],
            token_modifiers: vec!["readonly".to_string(), "declaration".to_string()],
        };
        let value = json!({
            "resultId": "snapshot-1",
            "data": [0, 2, 4, 1, 3, 1, 0, 5, 0, 1]
        });

        let tokens = parse_semantic_tokens_result(&value, &legend).expect("tokens should parse");

        assert_eq!(tokens.result_id.as_deref(), Some("snapshot-1"));
        assert_eq!(tokens.data, vec![0, 2, 4, 12, 5, 1, 0, 5, 8, 4,]);
        assert!(parse_semantic_tokens_result(&json!({"data": [0, 1, 2, 3]}), &legend).is_none());
    }

    #[test]
    fn parse_signature_help_result_accepts_signatures_and_parameter_ranges() {
        let value = json!({
            "activeSignature": 0,
            "activeParameter": 1,
            "signatures": [{
                "label": "write(path: string, contents: string): void",
                "documentation": {"kind": "markdown", "value": "Writes a file."},
                "activeParameter": 1,
                "parameters": [
                    {"label": [6, 18], "documentation": "Target path."},
                    {"label": "contents: string", "documentation": {"kind": "plaintext", "value": "File contents."}}
                ]
            }]
        });

        let help = parse_signature_help_result(&value).expect("signature help should parse");

        assert_eq!(help.active_signature, Some(0));
        assert_eq!(help.active_parameter, Some(1));
        assert_eq!(help.signatures.len(), 1);
        let signature = &help.signatures[0];
        assert_eq!(
            signature.label,
            "write(path: string, contents: string): void"
        );
        assert_eq!(signature.documentation.as_deref(), Some("Writes a file."));
        assert_eq!(signature.active_parameter, Some(1));
        assert_eq!(signature.parameters.len(), 2);
        assert_eq!(signature.parameters[0].label, "path: string");
        assert_eq!(
            signature.parameters[0].documentation.as_deref(),
            Some("Target path.")
        );
        assert_eq!(signature.parameters[1].label, "contents: string");
        assert_eq!(
            signature.parameters[1].documentation.as_deref(),
            Some("File contents.")
        );
    }

    #[test]
    fn parse_signature_help_result_ignores_empty_payloads() {
        assert!(parse_signature_help_result(&Value::Null).is_none());
        assert!(parse_signature_help_result(&json!({"signatures": []})).is_none());
    }

    #[test]
    fn parse_text_edits_result_accepts_formatting_edits() {
        let value = json!([{
            "range": {
                "start": {"line": 1, "character": 2},
                "end": {"line": 1, "character": 6}
            },
            "newText": "formatted"
        }]);

        let edits = parse_text_edits_result(&value);

        assert_eq!(
            edits,
            vec![LspTextEdit {
                range: LspRange {
                    start_line: 2,
                    start_column: 3,
                    end_line: 2,
                    end_column: 7,
                },
                text: "formatted".to_string(),
            }]
        );
        assert!(parse_text_edits_result(&Value::Null).is_empty());
    }

    #[test]
    fn parse_code_action_result_accepts_workspace_edits_and_disabled_actions() {
        let uri = if cfg!(windows) {
            "file:///C:/work/project/src/lib.rs"
        } else {
            "file:///work/project/src/lib.rs"
        };
        let value = json!([
            {
                "title": "Remove unused import",
                "kind": "quickfix",
                "isPreferred": true,
                "edit": {
                    "changes": {
                        uri: [{
                            "range": {
                                "start": {"line": 0, "character": 0},
                                "end": {"line": 0, "character": 12}
                            },
                            "newText": ""
                        }]
                    }
                }
            },
            {
                "title": "Generate missing impl",
                "kind": "quickfix",
                "disabled": {"reason": "requires valid selection"}
            },
            {
                "title": "Server command without edit is ignored",
                "command": {"command": "server.command", "title": "Run"}
            }
        ]);

        let actions = parse_code_action_result(&value);

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].title, "Remove unused import");
        assert_eq!(actions[0].kind.as_deref(), Some("quickfix"));
        assert!(actions[0].is_preferred);
        let edit = actions[0]
            .edit
            .as_ref()
            .expect("action should carry workspace edit");
        assert_eq!(edit.files.len(), 1);
        assert!(edit.files[0]
            .path
            .to_string_lossy()
            .ends_with(if cfg!(windows) {
                "C:\\work\\project\\src\\lib.rs"
            } else {
                "/work/project/src/lib.rs"
            }));
        assert_eq!(edit.files[0].edits[0].range.start_line, 1);
        assert_eq!(edit.files[0].edits[0].range.end_column, 13);
        assert_eq!(actions[1].title, "Generate missing impl");
        assert_eq!(
            actions[1].disabled_reason.as_deref(),
            Some("requires valid selection")
        );
        assert!(actions[1].edit.is_none());
    }

    #[test]
    fn parse_workspace_edit_result_accepts_changes() {
        let uri = if cfg!(windows) {
            "file:///C:/work/project/src/lib.rs"
        } else {
            "file:///work/project/src/lib.rs"
        };
        let value = json!({
            "changes": {
                uri: [{
                    "range": {
                        "start": {"line": 2, "character": 4},
                        "end": {"line": 2, "character": 9}
                    },
                    "newText": "after"
                }]
            }
        });

        let edit = parse_workspace_edit_result(&value).expect("workspace edit should parse");

        assert_eq!(edit.files.len(), 1);
        assert!(edit.files[0]
            .path
            .to_string_lossy()
            .ends_with(if cfg!(windows) {
                "C:\\work\\project\\src\\lib.rs"
            } else {
                "/work/project/src/lib.rs"
            }));
        assert_eq!(
            edit.files[0].edits[0].range,
            LspRange {
                start_line: 3,
                start_column: 5,
                end_line: 3,
                end_column: 10,
            }
        );
        assert_eq!(edit.files[0].edits[0].text, "after");
    }

    #[test]
    fn parse_workspace_edit_result_accepts_document_changes() {
        let uri = if cfg!(windows) {
            "file:///C:/work/project/src/main.rs"
        } else {
            "file:///work/project/src/main.rs"
        };
        let value = json!({
            "documentChanges": [{
                "textDocument": {"uri": uri, "version": 4},
                "edits": [{
                    "range": {
                        "start": {"line": 0, "character": 0},
                        "end": {"line": 0, "character": 4}
                    },
                    "newText": "main"
                }]
            }]
        });

        let edit = parse_workspace_edit_result(&value).expect("workspace edit should parse");

        assert_eq!(edit.files.len(), 1);
        assert!(edit.files[0]
            .path
            .to_string_lossy()
            .ends_with(if cfg!(windows) {
                "C:\\work\\project\\src\\main.rs"
            } else {
                "/work/project/src/main.rs"
            }));
        assert_eq!(
            edit.files[0].edits[0].range,
            LspRange {
                start_line: 1,
                start_column: 1,
                end_line: 1,
                end_column: 5,
            }
        );
        assert_eq!(edit.files[0].edits[0].text, "main");
    }

    #[test]
    fn workspace_diagnostics_from_publish_defaults_missing_severity_and_source() {
        let uri: Uri = "file:///tmp/example.ts".parse().expect("uri should parse");
        let diagnostic = Diagnostic {
            range: Range {
                start: Position::new(0, 0),
                end: Position::new(0, 4),
            },
            severity: None,
            source: None,
            message: "type mismatch".to_string(),
            ..Diagnostic::default()
        };

        let diagnostics = workspace_diagnostics_from_publish(PublishDiagnosticsParams::new(
            uri,
            vec![diagnostic],
            None,
        ));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Information);
        assert_eq!(diagnostics[0].source, "lsp");
        assert_eq!(diagnostics[0].line, 1);
        assert_eq!(diagnostics[0].column, 1);
    }
}
