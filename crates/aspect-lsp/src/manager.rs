use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use aspect_core::{
    AppResult, DiagnosticSeverity, DocumentSnapshot, LanguageServerInfo, LanguageServerStatus,
    LspCodeAction, LspCodeActionDiagnostic, LspCodeActionTrigger, LspCompletionList,
    LspDocumentSymbol, LspFoldingRange, LspFormattingOptions, LspHover, LspInlayHint,
    LspLocation, LspRange, LspSemanticTokens, LspSignatureHelp, LspTextEdit, LspWorkspaceEdit,
    LspWorkspaceSymbol, TextEdit, WorkspaceDiagnostic,
};
use tokio::sync::mpsc;

use crate::{
    discovery::diagnostic_anchor_path,
    helpers::{join_all, session_language_id},
    results::empty_completion_list,
    session::LspSession,
    types::{DiagnosticsUpdate, LanguageServerDefinition, MAX_WORKSPACE_SYMBOLS},
};

pub struct LspManager {
    diagnostics_tx: mpsc::UnboundedSender<DiagnosticsUpdate>,
    sessions: BTreeMap<String, LspSession>,
}

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
        extra_path_dirs: &[std::path::PathBuf],
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
            let anchor = diagnostic_anchor_path(server);
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
                    source: "aspect-lsp".to_string(),
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

        let query = query.as_str();
        let futures = self
            .sessions
            .iter_mut()
            .map(|(language_id, session)| async move {
                (language_id.clone(), session.workspace_symbols(query).await)
            })
            .collect::<Vec<_>>();
        let results = join_all(futures).await;

        let mut symbols = Vec::new();
        let mut dead_languages = Vec::new();
        for (language_id, result) in results {
            match result {
                Ok(found) => symbols.extend(found),
                Err(_) => dead_languages.push(language_id),
            }
        }
        for language_id in dead_languages {
            self.teardown_if_dead(&language_id).await;
        }

        symbols.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.location.path.cmp(&right.location.path))
        });
        symbols.truncate(MAX_WORKSPACE_SYMBOLS);
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
