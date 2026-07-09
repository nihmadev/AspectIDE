use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use aspect_core::{
    AppError, AppResult, DocumentSnapshot, LspCodeAction, LspCodeActionDiagnostic,
    LspCodeActionTrigger, LspCompletionList, LspDocumentSymbol, LspFoldingRange,
    LspFormattingOptions, LspHover, LspInlayHint, LspLocation, LspRange, LspSemanticTokens,
    LspSignatureHelp, LspTextEdit, LspWorkspaceEdit, LspWorkspaceSymbol, TextEdit,
};
use serde_json::Value;
use tokio::{
    io::AsyncWriteExt,
    process::{Child, ChildStdin},
    sync::mpsc,
    task::JoinHandle,
};

use crate::{
    helpers::{hide_process_window, prepend_path_dirs},
    protocol::{
        code_action_request, completion_request, definition_request, did_change_edits_notification,
        did_change_notification, did_close_notification, did_open_notification,
        did_save_notification, document_symbol_request, exit_notification, folding_range_request,
        formatting_request, hover_request, initialize_request, initialized_notification,
        inlay_hint_request, range_formatting_request, references_request, rename_request,
        semantic_tokens_full_request, shutdown_request, signature_help_request,
        workspace_symbol_request,
    },
    results::{
        empty_completion_list, parse_code_action_result, parse_completion_result,
        parse_definition_result, parse_document_symbol_result, parse_folding_range_result,
        parse_hover_result, parse_inlay_hint_result, parse_semantic_token_legend_from_initialize,
        parse_semantic_tokens_result, parse_signature_help_result, parse_text_document_sync_kind,
        parse_text_edits_result, parse_workspace_edit_result, parse_workspace_symbol_result,
    },
    transport::{drain_stderr, encode_lsp_message, read_lsp_stdout, LspResponse},
    types::{
        DiagnosticsUpdate, LanguageServerDefinition, SemanticTokenLegend, TextDocumentSyncKind,
    },
};

pub struct LspSession {
    pub(crate) definition: LanguageServerDefinition,
    stdin: ChildStdin,
    pub(crate) child: Child,
    pub(crate) read_task: JoinHandle<()>,
    pub(crate) stderr_task: Option<JoinHandle<()>>,
    responses: mpsc::UnboundedReceiver<LspResponse>,
    request_id: u64,
    pub opened_documents: BTreeMap<PathBuf, u64>,
    semantic_token_legend: Option<SemanticTokenLegend>,
    sync_kind: TextDocumentSyncKind,
}

impl LspSession {
    pub async fn start(
        definition: LanguageServerDefinition,
        diagnostics_tx: mpsc::UnboundedSender<DiagnosticsUpdate>,
    ) -> AppResult<Self> {
        use tokio::process::Command;
        use std::process::Stdio;

        let mut command = Command::new(&definition.command);
        command
            .args(&definition.args)
            .current_dir(&definition.workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
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

    pub async fn semantic_tokens(
        &mut self,
        path: &Path,
    ) -> AppResult<Option<LspSemanticTokens>> {
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

impl Drop for LspSession {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        self.read_task.abort();
        if let Some(stderr_task) = self.stderr_task.take() {
            stderr_task.abort();
        }
    }
}
