#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Stdio,
};

mod discovery;
mod protocol;
mod results;
mod transport;

pub use discovery::{
    language_server_diagnostics, workspace_language_servers, workspace_language_servers_with_dirs,
    BuiltinServer, BUILTIN_SERVERS,
};
pub use protocol::{
    code_action_request, completion_request, definition_request, did_change_edits_notification,
    did_change_notification, did_close_notification, did_open_notification, did_save_notification,
    document_symbol_request, exit_notification, folding_range_request, formatting_request,
    hover_request, initialize_request, initialized_notification, inlay_hint_request,
    path_to_file_uri, range_formatting_request, references_request, rename_request,
    semantic_tokens_full_request, shutdown_request, signature_help_request,
    workspace_symbol_request,
};
use results::empty_completion_list;
pub use results::{
    diagnostics_update_from_publish, parse_code_action_result, parse_completion_result,
    parse_definition_result, parse_document_symbol_result, parse_folding_range_result,
    parse_hover_result, parse_inlay_hint_result, parse_semantic_token_legend_from_initialize,
    parse_semantic_tokens_result, parse_signature_help_result, parse_text_document_sync_kind,
    parse_text_edits_result, parse_workspace_edit_result, parse_workspace_symbol_result,
    workspace_diagnostics_from_publish,
};
pub use transport::{
    drain_lsp_frames, encode_lsp_message, parse_lsp_notification, parse_lsp_response, LspFrame,
    LspNotification,
};
use transport::{drain_stderr, read_lsp_stdout, LspResponse};

use lux_core::{
    AppError, AppResult, DiagnosticSeverity, DocumentSnapshot, LanguageServerInfo,
    LanguageServerStatus, LspCodeAction, LspCodeActionDiagnostic, LspCodeActionTrigger,
    LspCompletionList, LspDocumentSymbol, LspFoldingRange, LspFormattingOptions, LspHover,
    LspInlayHint, LspLocation, LspRange, LspSemanticTokens, LspSignatureHelp, LspTextEdit,
    LspWorkspaceEdit, LspWorkspaceSymbol, TextEdit, WorkspaceDiagnostic,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    io::AsyncWriteExt,
    process::{Child, ChildStdin, Command},
    sync::mpsc,
    task::JoinHandle,
};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageServerDefinition {
    pub language_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub workspace_root: PathBuf,
    /// Directories prepended to the child process PATH at launch. Carries the
    /// IDE's managed runtime bins (Node/Rust/Python) so a managed server shim
    /// (e.g. `typescript-language-server` → `node`) finds its interpreter even
    /// when the host has no system toolchain. Empty for system-PATH servers.
    #[serde(default)]
    pub extra_path_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsUpdate {
    pub path: PathBuf,
    pub diagnostics: Vec<WorkspaceDiagnostic>,
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
    /// How this server wants `textDocument/didChange` payloads, parsed from the
    /// `initialize` reply. Drives whether we send ranged edits, full text, or skip.
    sync_kind: TextDocumentSyncKind,
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

impl LspManager {
    #[must_use]
    pub const fn new(diagnostics_tx: mpsc::UnboundedSender<DiagnosticsUpdate>) -> Self {
        Self {
            diagnostics_tx,
            sessions: BTreeMap::new(),
        }
    }

    pub async fn start_available_servers(
        &mut self,
        servers: &[LanguageServerInfo],
        extra_path_dirs: &[PathBuf],
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

        // Shut down sessions whose definition changed (command/args/root) so they
        // can be restarted with the new config.
        for server in servers {
            if server.status != LanguageServerStatus::Available {
                continue;
            }
            let mut definition = LanguageServerDefinition::from(server);
            definition.extra_path_dirs = extra_path_dirs.to_vec();
            let should_restart = self
                .sessions
                .get(&server.language_id)
                .is_some_and(|session| session.definition != definition);
            if should_restart {
                if let Some(session) = self.sessions.remove(&server.language_id) {
                    session.shutdown().await;
                }
            }
        }

        // Start every missing server CONCURRENTLY. Each `initialize` handshake can
        // take up to its 10s timeout; doing them sequentially made N servers stack
        // their timeouts and freeze language-service startup. A JoinSet lets them
        // race in parallel so total time is the slowest single server, not the sum.
        let mut join_set = tokio::task::JoinSet::new();
        for server in servers {
            if server.status != LanguageServerStatus::Available
                || self.sessions.contains_key(&server.language_id)
            {
                continue;
            }
            let mut definition = LanguageServerDefinition::from(server);
            definition.extra_path_dirs = extra_path_dirs.to_vec();
            let diagnostics_tx = self.diagnostics_tx.clone();
            let language_id = server.language_id.clone();
            let server_name = server.name.clone();
            let anchor = discovery::diagnostic_anchor_path(server);
            join_set.spawn(async move {
                let result = LspSession::start(definition, diagnostics_tx).await;
                (language_id, server_name, anchor, result)
            });
        }

        let mut diagnostics = Vec::new();
        while let Some(joined) = join_set.join_next().await {
            let Ok((language_id, server_name, anchor, result)) = joined else {
                continue;
            };
            match result {
                Ok(session) => {
                    self.sessions.insert(language_id, session);
                }
                Err(error) => diagnostics.push(WorkspaceDiagnostic {
                    path: anchor,
                    line: 1,
                    column: 1,
                    severity: DiagnosticSeverity::Warning,
                    source: "lux-lsp".to_string(),
                    message: format!("Failed to start {server_name}: {error}"),
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
            self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
        }
        result
    }

    pub async fn close_document(&mut self, path: &Path) -> AppResult<()> {
        // `close_document` only has a `&Path`, so it cannot reuse the editor's
        // `document.language_id` the way open/update/save do. Re-deriving a key
        // from the extension (the old `language_id_for_path`) diverges from the
        // richer map that keyed the open (e.g. user-overridden languages, or
        // `mts`/`scss`/`vue`/`typescriptreact` paths), so `get_mut` missed and
        // `didClose` never fired — leaking client + server open-file state.
        // Find the session that actually holds the path instead.
        let Some(language_id) = self.sessions.iter().find_map(|(language_id, session)| {
            session
                .opened_documents
                .contains_key(path)
                .then(|| language_id.clone())
        }) else {
            return Ok(());
        };
        let Some(session) = self.sessions.get_mut(&language_id) else {
            return Ok(());
        };
        let result = session.did_close(path).await;
        if result.is_err() {
            self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
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
                self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
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
            self.teardown_if_dead(&language_id).await;
        }
        result
    }

    pub async fn shutdown_all(&mut self) {
        let sessions = std::mem::take(&mut self.sessions);
        for (_, session) in sessions {
            session.shutdown().await;
        }
    }

    /// Tear down a session only when it is genuinely dead: either the child
    /// process has exited (or `try_wait` errored on a broken handle), or the
    /// stdout reader task has ended — which drops `response_tx` and makes every
    /// future request fail forever even though the OS process may briefly linger.
    ///
    /// A recoverable per-request failure (a timeout against a busy-but-alive
    /// server, or a single server error-response) surfaces as the same
    /// `AppError::Service` as genuine transport death, so an error-variant check
    /// cannot distinguish them. Removing the session on every `Err` would
    /// silently disable diagnostics, completion and every other feature for all
    /// files of that language until the servers are restarted. Liveness of both
    /// the process and its transport is the source of truth: only reap when one
    /// of them is actually gone. A still-running reader plus a live process
    /// (`try_wait() == Ok(None)`) is preserved so a busy server is not dropped
    /// mid-request.
    async fn teardown_if_dead(&mut self, language_id: &str) {
        let dead = self.sessions.get_mut(language_id).is_none_or(|session| {
            session.read_task.is_finished() || !matches!(session.child.try_wait(), Ok(None))
        });
        if dead {
            if let Some(session) = self.sessions.remove(language_id) {
                session.shutdown().await;
            }
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
        let mut command = Command::new(&definition.command);
        command
            .args(&definition.args)
            .current_dir(&definition.workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Prepend the IDE's managed runtime bins so a managed server shim resolves
        // its interpreter (node/python) without a system toolchain on PATH.
        if let Some(path) = prepend_path_dirs(&definition.extra_path_dirs) {
            command.env("PATH", path);
        }
        hide_process_window(&mut command);
        let mut child = command.spawn()?;

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
            sync_kind: TextDocumentSyncKind::default(),
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

        // Honor the server's declared sync mode: ranged edits ONLY when the server
        // advertised `Incremental`. A `Full`-sync server would mis-apply ranged
        // changes (corrupting its view), and a `None`-sync server wants nothing.
        let message = match self.sync_kind {
            TextDocumentSyncKind::Incremental => {
                Some(did_change_edits_notification(document, edits))
            }
            TextDocumentSyncKind::Full => Some(did_change_notification(document)),
            TextDocumentSyncKind::None => None,
        };
        if let Some(message) = message {
            self.write_message(&message).await?;
        }
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
            .map_or_else(empty_completion_list, parse_completion_result))
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
        // The `initialize` *handshake* reply is normally fast (heavy indexing
        // happens asynchronously afterwards), but a cold or busy machine can be
        // slow to even launch the server binary. Startup now runs in the
        // background off the UI path, so a generous ceiling costs nothing and
        // avoids dropping a server that was merely slow to wake up.
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(30),
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

        // Record the server's declared change-sync mode so we never send ranged
        // edits to a server that requires full text (or no change sync at all).
        self.sync_kind = response
            .result
            .as_ref()
            .map(parse_text_document_sync_kind)
            .unwrap_or_default();

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

    const fn next_request_id(&mut self) -> u64 {
        self.request_id += 1;
        self.request_id
    }
}

#[cfg(windows)]
fn hide_process_window(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
const fn hide_process_window(_command: &mut Command) {}

/// Build a PATH value with `dirs` (that exist) prepended ahead of the inherited
/// PATH. Returns None when there is nothing to prepend, so the child just inherits
/// the parent PATH unchanged.
fn prepend_path_dirs(dirs: &[PathBuf]) -> Option<std::ffi::OsString> {
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let mut parts: Vec<PathBuf> = dirs.iter().filter(|dir| dir.is_dir()).cloned().collect();
    if parts.is_empty() {
        return None;
    }
    parts.extend(std::env::split_paths(&existing));
    std::env::join_paths(parts).ok()
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
            extra_path_dirs: Vec::new(),
        }
    }
}

fn session_language_id(language_id: &str) -> &str {
    match language_id {
        "javascript" | "javascriptreact" | "typescriptreact" => "typescript",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Diagnostic, Position, PublishDiagnosticsParams, Range, Uri};
    use lux_core::{
        LspCompletionItemKind, LspFoldingRangeKind, LspInlayHintKind, LspInsertTextFormat,
        LspSymbolKind,
    };
    use serde_json::json;

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
            extra_path_dirs: Vec::new(),
        };
        let document = DocumentSnapshot {
            id: lux_core::BufferId::new(),
            path: Some(workspace_root.join("src").join("file name.ts")),
            title: "file name.ts".to_string(),
            language_id: "typescript".to_string(),
            text: "const value = 1;\n".to_string(),
            view: lux_core::FileViewDescriptor::default(),
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
            view: lux_core::FileViewDescriptor::default(),
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

    fn sample_request_range() -> LspRange {
        LspRange {
            start_line: 3,
            start_column: 5,
            end_line: 3,
            end_column: 9,
        }
    }

    #[test]
    fn navigation_requests_use_zero_based_lsp_positions() {
        let path = PathBuf::from("/tmp/src/main.rs");

        let hover = hover_request(41, &path, 5, 3);
        let definition = definition_request(42, &path, 1, 1);
        let references = references_request(45, &path, 7, 2);
        let rename = rename_request(46, &path, 11, 6, "renamed_value");

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
    }

    #[test]
    fn assistance_requests_include_expected_context() {
        let path = PathBuf::from("/tmp/src/main.rs");
        let completion = completion_request(43, &path, 2, 4);
        let signature = signature_help_request(44, &path, 9, 12);
        let code_action_range = sample_request_range();
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
    }

    #[test]
    fn formatting_requests_keep_options_and_ranges() {
        let path = PathBuf::from("/tmp/src/main.rs");
        let code_action_range = sample_request_range();
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

        assert_eq!(formatting["id"], 48);
        assert_eq!(formatting["method"], "textDocument/formatting");
        assert_eq!(formatting["params"]["options"]["tabSize"], 4);
        assert_eq!(formatting["params"]["options"]["insertSpaces"], true);
        assert_eq!(range_formatting["id"], 49);
        assert_eq!(range_formatting["method"], "textDocument/rangeFormatting");
        assert_eq!(range_formatting["params"]["range"]["end"]["line"], 2);
        assert_eq!(range_formatting["params"]["options"]["insertSpaces"], false);
    }

    #[test]
    fn symbol_and_token_requests_use_file_uris() {
        let path = PathBuf::from("/tmp/src/main.rs");
        let document_symbol = document_symbol_request(50, &path);
        let workspace_symbol = workspace_symbol_request(51, "LuxStore");
        let folding_range = folding_range_request(52, &path);
        let inlay_hint_range = sample_request_range();
        let inlay_hint = inlay_hint_request(53, &path, &inlay_hint_range);
        let semantic_tokens = semantic_tokens_full_request(54, &path);

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
    fn parse_text_document_sync_kind_handles_number_object_and_default() {
        // Numeric form.
        assert_eq!(
            parse_text_document_sync_kind(&json!({ "capabilities": { "textDocumentSync": 2 } })),
            TextDocumentSyncKind::Incremental
        );
        assert_eq!(
            parse_text_document_sync_kind(&json!({ "capabilities": { "textDocumentSync": 0 } })),
            TextDocumentSyncKind::None
        );
        // Object form with a `change` field.
        assert_eq!(
            parse_text_document_sync_kind(
                &json!({ "capabilities": { "textDocumentSync": { "change": 1, "openClose": true } } })
            ),
            TextDocumentSyncKind::Full
        );
        // Missing capability → safe Full default (never silently send ranged edits).
        assert_eq!(
            parse_text_document_sync_kind(&json!({ "capabilities": {} })),
            TextDocumentSyncKind::Full
        );
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
                "title": "Organize imports (server command)",
                "command": {"command": "server.command", "title": "Run"}
            }
        ]);

        let actions = parse_code_action_result(&value);

        // Command-only actions are now SURFACED (with a non-applicable reason),
        // not silently dropped — so the count is 3, including the command action.
        assert_eq!(actions.len(), 3);
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

        // The command-only action is present, has no edit, and carries a reason
        // explaining why it can't be applied directly.
        assert_eq!(actions[2].title, "Organize imports (server command)");
        assert!(actions[2].edit.is_none());
        assert!(
            actions[2]
                .disabled_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("command")),
            "command-only action should carry an explanatory reason"
        );
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
