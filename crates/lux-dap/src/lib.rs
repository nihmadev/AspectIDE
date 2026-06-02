#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    time::Duration,
};

use chrono::Utc;
use lux_core::{
    AppError, AppResult, DebugAdapterInfo, DebugAdapterStatus, DebugAdapterTransport,
    DebugBreakpointsUpdate, DebugConfiguration, DebugConfigurationRequest, DebugEvaluateContext,
    DebugEvaluateResult, DebugExecutionAction, DebugFrameScopes, DebugResolvedBreakpoint,
    DebugScopeInfo, DebugSessionInfo, DebugSessionStatus, DebugSourceBreakpoint, DebugStackFrame,
    DebugStackTrace, DebugThreadInfo, DebugVariableInfo, DebugVariables, DebugWorkspaceInfo,
};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, WriteHalf},
    net::{TcpListener, TcpStream},
    process::{Child, ChildStdin, Command},
    sync::mpsc,
    task::JoinHandle,
};
use uuid::Uuid;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const TCP_CONNECT_ATTEMPTS: u32 = 40;
const TCP_CONNECT_DELAY: Duration = Duration::from_millis(50);
const DISCONNECT_GRACE_TIMEOUT: Duration = Duration::from_secs(2);
const DISCONNECT_POLL_DELAY: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DapFrame {
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DapResponse {
    pub request_seq: u64,
    pub success: bool,
    pub command: String,
    pub message: Option<String>,
    pub body: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DapEvent {
    pub event: String,
    pub body: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DapMessage {
    Response(DapResponse),
    Event(DapEvent),
    Request { command: String },
}

#[derive(Debug, Clone)]
pub enum DebugSessionUpdate {
    Changed(DebugSessionInfo),
    BreakpointsChanged(DebugBreakpointsUpdate),
    Output { session_id: Uuid, text: String },
}

pub struct DebugSessionManager {
    update_tx: mpsc::UnboundedSender<DebugSessionUpdate>,
    sessions: BTreeMap<Uuid, DebugSession>,
}

struct DebugSession {
    info: DebugSessionInfo,
    writer: DapWriter,
    child: Child,
    read_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
    messages: mpsc::UnboundedReceiver<DapMessage>,
    next_seq: u64,
    disconnect_sent: bool,
    configuration_done_sent: bool,
    breakpoints_by_path: BTreeMap<PathBuf, Vec<DebugSourceBreakpoint>>,
    resolved_breakpoints_by_path: BTreeMap<PathBuf, Vec<DebugResolvedBreakpoint>>,
    pending_breakpoint_requests: BTreeMap<u64, PathBuf>,
    pending_responses: BTreeMap<u64, DapResponse>,
    threads: BTreeMap<u64, DebugThreadInfo>,
}

struct SpawnedDebugAdapter {
    writer: DapWriter,
    child: Child,
    read_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
    messages: mpsc::UnboundedReceiver<DapMessage>,
}

enum DapWriter {
    Stdio(ChildStdin),
    Tcp(WriteHalf<TcpStream>),
}

impl DapWriter {
    async fn write_all(&mut self, encoded: &[u8]) -> AppResult<()> {
        match self {
            Self::Stdio(stdin) => {
                stdin.write_all(encoded).await?;
                stdin.flush().await?;
            }
            Self::Tcp(stream) => {
                stream.write_all(encoded).await?;
                stream.flush().await?;
            }
        }
        Ok(())
    }
}

enum DebugLifecycleRequest {
    ConfigureBreakpoints(Vec<PathBuf>),
}

impl DebugSessionManager {
    #[must_use]
    pub const fn new(update_tx: mpsc::UnboundedSender<DebugSessionUpdate>) -> Self {
        Self {
            update_tx,
            sessions: BTreeMap::new(),
        }
    }

    pub async fn start(
        &mut self,
        adapter: DebugAdapterInfo,
        configuration: DebugConfiguration,
        breakpoints: Vec<DebugSourceBreakpoint>,
        workspace_root: PathBuf,
    ) -> AppResult<DebugSessionInfo> {
        validate_start_adapter(&adapter)?;
        let session_id = Uuid::new_v4();
        let spawned = self
            .spawn_adapter_process(&adapter, &workspace_root, session_id)
            .await?;
        self.insert_starting_session(
            session_id,
            &adapter,
            &configuration,
            breakpoints,
            &workspace_root,
            spawned,
        );
        self.emit_session(session_id)?;

        if let Err(error) = self
            .start_handshake(session_id, &adapter.id, &configuration)
            .await
        {
            return Err(self.fail_start(session_id, error).await);
        }
        self.mark_started(session_id, configuration.request)?;
        let info = self.session_info(session_id)?;
        self.emit_session(session_id)?;
        Ok(info)
    }

    async fn spawn_adapter_process(
        &self,
        adapter: &DebugAdapterInfo,
        workspace_root: &Path,
        session_id: Uuid,
    ) -> AppResult<SpawnedDebugAdapter> {
        match adapter.transport {
            DebugAdapterTransport::Stdio => {
                self.spawn_stdio_adapter_process(adapter, workspace_root, session_id)
            }
            DebugAdapterTransport::TcpServer => {
                self.spawn_tcp_server_adapter_process(adapter, workspace_root, session_id)
                    .await
            }
        }
    }

    fn spawn_stdio_adapter_process(
        &self,
        adapter: &DebugAdapterInfo,
        workspace_root: &Path,
        session_id: Uuid,
    ) -> AppResult<SpawnedDebugAdapter> {
        let mut command = Command::new(&adapter.command);
        command
            .args(&adapter.args)
            .current_dir(workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);

        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::Service("debug adapter stdin is unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Service("debug adapter stdout is unavailable".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::Service("debug adapter stderr is unavailable".into()))?;
        let (message_tx, messages) = mpsc::unbounded_channel();
        let read_task = tokio::spawn(read_dap_stdout(
            stdout,
            session_id,
            message_tx,
            self.update_tx.clone(),
        ));
        let stderr_task = tokio::spawn(drain_debug_stderr(
            stderr,
            session_id,
            self.update_tx.clone(),
        ));

        Ok(SpawnedDebugAdapter {
            writer: DapWriter::Stdio(stdin),
            child,
            read_task,
            stderr_task,
            messages,
        })
    }

    async fn spawn_tcp_server_adapter_process(
        &self,
        adapter: &DebugAdapterInfo,
        workspace_root: &Path,
        session_id: Uuid,
    ) -> AppResult<SpawnedDebugAdapter> {
        let (args, port) = tcp_server_args_and_port(adapter).await?;
        let mut command = Command::new(&adapter.command);
        command
            .args(&args)
            .current_dir(workspace_root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);

        let mut child = command.spawn()?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::Service("debug adapter stderr is unavailable".into()))?;
        let stream = match connect_tcp_debug_adapter(port).await {
            Ok(stream) => stream,
            Err(error) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(error);
            }
        };
        let (reader, writer) = tokio::io::split(stream);
        let (message_tx, messages) = mpsc::unbounded_channel();
        let read_task = tokio::spawn(read_dap_stdout(
            reader,
            session_id,
            message_tx,
            self.update_tx.clone(),
        ));
        let stderr_task = tokio::spawn(drain_debug_stderr(
            stderr,
            session_id,
            self.update_tx.clone(),
        ));

        Ok(SpawnedDebugAdapter {
            writer: DapWriter::Tcp(writer),
            child,
            read_task,
            stderr_task,
            messages,
        })
    }

    fn insert_starting_session(
        &mut self,
        session_id: Uuid,
        adapter: &DebugAdapterInfo,
        configuration: &DebugConfiguration,
        breakpoints: Vec<DebugSourceBreakpoint>,
        workspace_root: &Path,
        spawned: SpawnedDebugAdapter,
    ) {
        let breakpoints_by_path = group_source_breakpoints(breakpoints);
        let info = DebugSessionInfo {
            id: session_id,
            configuration_name: configuration.name.clone(),
            adapter_id: adapter.id.clone(),
            adapter_name: adapter.name.clone(),
            workspace_root: workspace_root.to_path_buf(),
            status: DebugSessionStatus::Starting,
            started_at: Utc::now(),
            stopped_at: None,
            active_thread_id: None,
            last_event: None,
            error: None,
        };
        self.sessions.insert(
            session_id,
            DebugSession {
                info,
                writer: spawned.writer,
                child: spawned.child,
                read_task: spawned.read_task,
                stderr_task: spawned.stderr_task,
                messages: spawned.messages,
                next_seq: 1,
                disconnect_sent: false,
                configuration_done_sent: false,
                breakpoints_by_path,
                resolved_breakpoints_by_path: BTreeMap::new(),
                pending_breakpoint_requests: BTreeMap::new(),
                pending_responses: BTreeMap::new(),
                threads: BTreeMap::new(),
            },
        );
    }

    async fn start_handshake(
        &mut self,
        session_id: Uuid,
        adapter_id: &str,
        configuration: &DebugConfiguration,
    ) -> AppResult<()> {
        let initialize_seq = self.next_request_seq(session_id)?;
        self.send_request(session_id, initialize_request(initialize_seq, adapter_id))
            .await?;
        self.wait_for_response(session_id, initialize_seq, "initialize")
            .await?;
        let configuration_seq = self.next_request_seq(session_id)?;
        self.send_request(
            session_id,
            match configuration.request {
                DebugConfigurationRequest::Launch => {
                    launch_request(configuration_seq, configuration)
                }
                DebugConfigurationRequest::Attach => {
                    attach_request(configuration_seq, configuration)
                }
            },
        )
        .await?;
        self.wait_for_response(
            session_id,
            configuration_seq,
            match configuration.request {
                DebugConfigurationRequest::Launch => "launch",
                DebugConfigurationRequest::Attach => "attach",
            },
        )
        .await?;
        self.wait_for_configuration_done(session_id).await
    }

    pub async fn sessions(&mut self) -> Vec<DebugSessionInfo> {
        self.drain_messages().await;
        self.sessions
            .values()
            .map(|session| session.info.clone())
            .collect()
    }

    pub async fn stack_trace(&mut self, session_id: Uuid) -> AppResult<DebugStackTrace> {
        self.drain_messages().await;
        self.ensure_stack_trace_allowed(session_id)?;
        let thread = self.resolve_stack_thread(session_id).await?;
        let stack_trace_seq = self.next_request_seq(session_id)?;
        self.send_request(
            session_id,
            stack_trace_request(stack_trace_seq, thread.id, 0, 64),
        )
        .await?;
        let response = self
            .wait_for_response_body(session_id, stack_trace_seq, "stackTrace")
            .await?;
        Ok(parse_stack_trace_response(
            session_id,
            thread,
            response.as_ref(),
        ))
    }

    pub async fn scopes(&mut self, session_id: Uuid, frame_id: u64) -> AppResult<DebugFrameScopes> {
        self.drain_messages().await;
        self.ensure_paused(session_id, "scopes")?;
        if frame_id == 0 {
            return Err(AppError::Service(
                "debug stack frame id must be positive".into(),
            ));
        }
        let scopes_seq = self.next_request_seq(session_id)?;
        self.send_request(session_id, scopes_request(scopes_seq, frame_id))
            .await?;
        let response = self
            .wait_for_response_body(session_id, scopes_seq, "scopes")
            .await?;
        Ok(parse_scopes_response(
            session_id,
            frame_id,
            response.as_ref(),
        ))
    }

    pub async fn variables(
        &mut self,
        session_id: Uuid,
        variables_reference: u64,
    ) -> AppResult<DebugVariables> {
        self.drain_messages().await;
        self.ensure_paused(session_id, "variables")?;
        if variables_reference == 0 {
            return Err(AppError::Service(
                "debug variables reference must be positive".into(),
            ));
        }
        let variables_seq = self.next_request_seq(session_id)?;
        self.send_request(
            session_id,
            variables_request(variables_seq, variables_reference, 0, 200),
        )
        .await?;
        let response = self
            .wait_for_response_body(session_id, variables_seq, "variables")
            .await?;
        Ok(parse_variables_response(
            session_id,
            variables_reference,
            response.as_ref(),
        ))
    }

    pub async fn evaluate(
        &mut self,
        session_id: Uuid,
        expression: String,
        frame_id: Option<u64>,
        context: DebugEvaluateContext,
    ) -> AppResult<DebugEvaluateResult> {
        self.drain_messages().await;
        self.ensure_paused(session_id, "expression evaluation")?;
        let expression = non_empty_text(Some(&expression)).ok_or_else(|| {
            AppError::Service("debug evaluate expression must not be empty".into())
        })?;
        if let Some(frame_id) = frame_id {
            if frame_id == 0 {
                return Err(AppError::Service(
                    "debug evaluate frame id must be positive".into(),
                ));
            }
        }
        let evaluate_seq = self.next_request_seq(session_id)?;
        self.send_request(
            session_id,
            evaluate_request(evaluate_seq, &expression, frame_id, context),
        )
        .await?;
        let response = self
            .wait_for_response_body(session_id, evaluate_seq, "evaluate")
            .await?;
        parse_evaluate_response(session_id, expression, response.as_ref())
    }

    pub async fn execute(
        &mut self,
        session_id: Uuid,
        action: DebugExecutionAction,
    ) -> AppResult<DebugSessionInfo> {
        self.drain_messages().await;
        self.ensure_debug_execution_allowed(session_id)?;
        let thread = self.resolve_stack_thread(session_id).await?;
        let command = execution_action_command(action);
        let execute_seq = self.next_request_seq(session_id)?;
        self.send_request(
            session_id,
            execution_request(execute_seq, action, thread.id),
        )
        .await?;
        self.wait_for_response(session_id, execute_seq, command)
            .await?;
        self.with_session_mut(session_id, |session| {
            session.info.status = DebugSessionStatus::Running;
            session.info.last_event = Some(format!("{command} requested"));
        })?;
        let info = self.session_info(session_id)?;
        self.emit_session(session_id)?;
        Ok(info)
    }

    pub async fn set_breakpoints(
        &mut self,
        session_id: Uuid,
        path: PathBuf,
        breakpoints: Vec<DebugSourceBreakpoint>,
    ) -> AppResult<DebugBreakpointsUpdate> {
        self.drain_messages().await;
        validate_breakpoint_session(session_id, self.sessions.get(&session_id))?;
        let path = normalize_breakpoint_path(path)?;
        let breakpoints = sanitize_source_breakpoints(&path, breakpoints);
        self.with_session_mut(session_id, |session| {
            if breakpoints.is_empty() {
                session.breakpoints_by_path.remove(&path);
            } else {
                session
                    .breakpoints_by_path
                    .insert(path.clone(), breakpoints);
            }
        })?;

        if !self.configuration_done_sent(session_id)? {
            let update = self.unverified_breakpoints_update(session_id, &path)?;
            self.emit_breakpoints(update.clone())?;
            return Ok(update);
        }

        self.send_breakpoints_for_path(session_id, &path).await?;
        self.breakpoints_update(session_id, &path)
    }

    pub async fn stop(&mut self, session_id: Uuid) -> AppResult<DebugSessionInfo> {
        self.drain_messages().await;
        let disconnect = self.with_session_mut(session_id, |session| {
            if session.disconnect_sent
                || matches!(
                    session.info.status,
                    DebugSessionStatus::Stopped | DebugSessionStatus::Error
                )
            {
                None
            } else {
                session.disconnect_sent = true;
                session.info.status = DebugSessionStatus::Stopping;
                session.info.last_event = Some("disconnect requested".to_string());
                let seq = session.next_seq;
                session.next_seq += 1;
                Some(disconnect_request(seq, true))
            }
        })?;
        self.emit_session(session_id)?;

        if let Some(request) = disconnect {
            if let Err(error) = self.send_request(session_id, request).await {
                self.with_session_mut(session_id, |session| {
                    session.info.last_event = Some(format!(
                        "disconnect request failed; forcing adapter cleanup: {error}"
                    ));
                })?;
                self.emit_session(session_id)?;
            } else {
                self.wait_for_disconnect_terminal_state(session_id).await?;
            }
        }

        self.force_stop_session(session_id).await
    }

    pub async fn stop_all(&mut self) {
        let session_ids = self.sessions.keys().copied().collect::<Vec<_>>();
        for session_id in session_ids {
            let _result = self.stop(session_id).await;
        }
    }

    async fn wait_for_disconnect_terminal_state(&mut self, session_id: Uuid) -> AppResult<()> {
        let deadline = tokio::time::Instant::now() + DISCONNECT_GRACE_TIMEOUT;
        loop {
            self.drain_messages().await;
            if self.session_is_terminal(session_id)? {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                self.with_session_mut(session_id, |session| {
                    if !is_terminal_status(session.info.status) {
                        session.info.last_event =
                            Some("disconnect timed out; forcing adapter cleanup".to_string());
                    }
                })?;
                self.emit_session(session_id)?;
                return Ok(());
            }
            tokio::time::sleep(DISCONNECT_POLL_DELAY).await;
        }
    }

    async fn force_stop_session(&mut self, session_id: Uuid) -> AppResult<DebugSessionInfo> {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };

        let exited = matches!(session.child.try_wait(), Ok(Some(_)));
        if !exited {
            let _ = session.child.start_kill();
            let _ = session.child.wait().await;
        }
        session.read_task.abort();
        session.stderr_task.abort();

        if !is_terminal_status(session.info.status) {
            session.info.status = DebugSessionStatus::Stopped;
            session.info.stopped_at.get_or_insert_with(Utc::now);
            session.info.last_event = Some(if exited {
                "session stopped".to_string()
            } else {
                "session force stopped".to_string()
            });
        }

        let info = session.info.clone();
        self.emit_session(session_id)?;
        Ok(info)
    }

    async fn drain_messages(&mut self) {
        let session_ids = self.sessions.keys().copied().collect::<Vec<_>>();
        for session_id in session_ids {
            while let Some(message) = self
                .sessions
                .get_mut(&session_id)
                .and_then(|session| session.messages.try_recv().ok())
            {
                if let Err(error) = self.apply_message(session_id, message).await {
                    self.mark_session_error(session_id, &error);
                }
            }
        }
        self.poll_exited_adapters();
    }

    fn poll_exited_adapters(&mut self) {
        let session_ids = self.sessions.keys().copied().collect::<Vec<_>>();
        for session_id in session_ids {
            let changed = self
                .sessions
                .get_mut(&session_id)
                .is_some_and(mark_session_if_adapter_exited);
            if changed {
                let _result = self.emit_session(session_id);
            }
        }
    }

    async fn wait_for_response(
        &mut self,
        session_id: Uuid,
        request_seq: u64,
        command: &str,
    ) -> AppResult<()> {
        let _body = self
            .wait_for_response_body(session_id, request_seq, command)
            .await?;
        Ok(())
    }

    async fn wait_for_response_body(
        &mut self,
        session_id: Uuid,
        request_seq: u64,
        command: &str,
    ) -> AppResult<Option<Value>> {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if let Some(response) = self.take_pending_response(session_id, request_seq)? {
                    return self.apply_expected_response(session_id, response, command);
                }
                let message = self.recv_message(session_id).await?;
                match message {
                    DapMessage::Response(response) if response.request_seq == request_seq => {
                        return self.apply_expected_response(session_id, response, command);
                    }
                    other => self.apply_message(session_id, other).await?,
                }
            }
        })
        .await
        .map_err(|_| {
            AppError::Service(format!(
                "debug adapter did not respond to {command} within 8 seconds"
            ))
        })?
    }

    fn apply_expected_response(
        &mut self,
        session_id: Uuid,
        response: DapResponse,
        command: &str,
    ) -> AppResult<Option<Value>> {
        let success = response.success;
        let error_message = response.message.clone();
        let body = response.body.clone();
        self.apply_response(session_id, response)?;
        self.emit_session(session_id)?;
        if success {
            return Ok(body);
        }
        Err(AppError::Service(error_message.unwrap_or_else(|| {
            format!("debug adapter rejected {command} request")
        })))
    }

    async fn recv_message(&mut self, session_id: Uuid) -> AppResult<DapMessage> {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        session
            .messages
            .recv()
            .await
            .ok_or_else(|| AppError::Service("debug adapter message stream closed".into()))
    }

    async fn apply_message(&mut self, session_id: Uuid, message: DapMessage) -> AppResult<()> {
        match message {
            DapMessage::Event(event) => self.apply_event(session_id, event).await?,
            DapMessage::Response(response) => self.apply_response(session_id, response)?,
            DapMessage::Request { command } => {
                self.with_session_mut(session_id, |session| {
                    session.info.last_event = Some(format!("adapter request: {command}"));
                })?;
            }
        }
        self.emit_session(session_id)
    }

    async fn apply_event(&mut self, session_id: Uuid, event: DapEvent) -> AppResult<()> {
        let request = self.with_session_mut(session_id, |session| {
            session.info.last_event = Some(event.event.clone());
            match event.event.as_str() {
                "initialized" => {
                    let paths = session
                        .breakpoints_by_path
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>();
                    Some(DebugLifecycleRequest::ConfigureBreakpoints(paths))
                }
                "stopped" => {
                    session.info.status = DebugSessionStatus::Paused;
                    session.info.active_thread_id = stopped_event_thread_id(event.body.as_ref());
                    None
                }
                "continued" => {
                    session.info.status = DebugSessionStatus::Running;
                    None
                }
                "terminated" | "exited" => {
                    session.info.status = DebugSessionStatus::Stopped;
                    session.info.stopped_at.get_or_insert_with(Utc::now);
                    None
                }
                _ => None,
            }
        })?;

        if let Some(request) = request {
            match request {
                DebugLifecycleRequest::ConfigureBreakpoints(paths) => {
                    for path in paths {
                        self.send_breakpoints_for_path(session_id, &path).await?;
                    }
                    self.send_configuration_done(session_id).await?;
                }
            }
        }
        Ok(())
    }

    async fn wait_for_configuration_done(&mut self, session_id: Uuid) -> AppResult<()> {
        if self.configuration_done_sent(session_id)? {
            return Ok(());
        }

        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let message = self.recv_message(session_id).await?;
                self.apply_message(session_id, message).await?;
                if self.configuration_done_sent(session_id)? {
                    return Ok(());
                }
            }
        })
        .await
        .map_err(|_| {
            AppError::Service(
                "debug adapter did not emit initialized before configurationDone within 8 seconds"
                    .into(),
            )
        })?
    }

    fn apply_response(&mut self, session_id: Uuid, response: DapResponse) -> AppResult<()> {
        let breakpoint_update = self.with_session_mut(session_id, |session| {
            session.info.last_event = Some(format!("{} response", response.command));
            if response.command == "setBreakpoints" {
                return apply_breakpoints_response(session_id, session, response);
            }
            if !response.success {
                session.info.status = DebugSessionStatus::Error;
                session.info.stopped_at.get_or_insert_with(Utc::now);
                session.info.error = Some(response.message.unwrap_or_else(|| {
                    format!("debug adapter rejected {} request", response.command)
                }));
            }
            None
        })?;
        if let Some(update) = breakpoint_update {
            self.emit_breakpoints(update)?;
        }
        Ok(())
    }

    fn store_pending_response(&mut self, session_id: Uuid, response: DapResponse) -> AppResult<()> {
        self.with_session_mut(session_id, |session| {
            store_pending_response_by_seq(&mut session.pending_responses, response);
        })
    }

    fn take_pending_response(
        &mut self,
        session_id: Uuid,
        request_seq: u64,
    ) -> AppResult<Option<DapResponse>> {
        self.with_session_mut(session_id, |session| {
            take_pending_response_by_seq(&mut session.pending_responses, request_seq)
        })
    }

    async fn send_request(&mut self, session_id: Uuid, request: Value) -> AppResult<()> {
        let encoded = encode_dap_message(&request)?;
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        session.writer.write_all(&encoded).await?;
        Ok(())
    }

    fn emit_session(&self, session_id: Uuid) -> AppResult<()> {
        let Some(session) = self.sessions.get(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        self.update_tx
            .send(DebugSessionUpdate::Changed(session.info.clone()))
            .map_err(|error| {
                AppError::Service(format!("debug session event channel closed: {error}"))
            })
    }

    fn emit_breakpoints(&self, update: DebugBreakpointsUpdate) -> AppResult<()> {
        self.update_tx
            .send(DebugSessionUpdate::BreakpointsChanged(update))
            .map_err(|error| {
                AppError::Service(format!("debug breakpoint event channel closed: {error}"))
            })
    }

    async fn send_breakpoints_for_path(&mut self, session_id: Uuid, path: &Path) -> AppResult<()> {
        let breakpoints = self
            .sessions
            .get(&session_id)
            .and_then(|session| session.breakpoints_by_path.get(path).cloned())
            .unwrap_or_default();
        let seq = self.next_request_seq(session_id)?;
        self.with_session_mut(session_id, |session| {
            session
                .pending_breakpoint_requests
                .insert(seq, path.to_path_buf());
        })?;
        self.send_request(session_id, set_breakpoints_request(seq, path, &breakpoints))
            .await?;
        self.wait_for_breakpoints_response(session_id, seq).await?;
        Ok(())
    }

    async fn wait_for_breakpoints_response(
        &mut self,
        session_id: Uuid,
        request_seq: u64,
    ) -> AppResult<()> {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if let Some(response) = self.take_pending_response(session_id, request_seq)? {
                    self.apply_response(session_id, response)?;
                    self.emit_session(session_id)?;
                    return Ok(());
                }
                let message = self.recv_message(session_id).await?;
                match message {
                    DapMessage::Response(response) if response.request_seq == request_seq => {
                        self.apply_response(session_id, response)?;
                        self.emit_session(session_id)?;
                        return Ok(());
                    }
                    DapMessage::Response(response) => {
                        self.store_pending_response(session_id, response)?;
                    }
                    DapMessage::Event(event) => {
                        self.apply_non_initialized_event(session_id, &event)?;
                    }
                    DapMessage::Request { command } => {
                        self.with_session_mut(session_id, |session| {
                            session.info.last_event = Some(format!("adapter request: {command}"));
                        })?;
                    }
                }
            }
        })
        .await
        .map_err(|_| {
            AppError::Service(
                "debug adapter did not respond to setBreakpoints within 8 seconds".into(),
            )
        })?
    }

    fn apply_non_initialized_event(&mut self, session_id: Uuid, event: &DapEvent) -> AppResult<()> {
        self.with_session_mut(session_id, |session| {
            session.info.last_event = Some(event.event.clone());
            match event.event.as_str() {
                "stopped" => {
                    session.info.status = DebugSessionStatus::Paused;
                    session.info.active_thread_id = stopped_event_thread_id(event.body.as_ref());
                }
                "continued" => {
                    session.info.status = DebugSessionStatus::Running;
                }
                "terminated" | "exited" => {
                    session.info.status = DebugSessionStatus::Stopped;
                    session.info.stopped_at.get_or_insert_with(Utc::now);
                }
                _ => {}
            }
        })
    }

    async fn send_configuration_done(&mut self, session_id: Uuid) -> AppResult<()> {
        let request = self.with_session_mut(session_id, |session| {
            if session.configuration_done_sent {
                None
            } else {
                session.configuration_done_sent = true;
                let seq = session.next_seq;
                session.next_seq += 1;
                Some(configuration_done_request(seq))
            }
        })?;
        if let Some(request) = request {
            self.send_request(session_id, request).await?;
        }
        Ok(())
    }

    fn configuration_done_sent(&self, session_id: Uuid) -> AppResult<bool> {
        self.sessions
            .get(&session_id)
            .map(|session| session.configuration_done_sent)
            .ok_or_else(|| AppError::NotFound(format!("debug session {session_id}")))
    }

    fn session_is_terminal(&self, session_id: Uuid) -> AppResult<bool> {
        self.sessions
            .get(&session_id)
            .map(|session| is_terminal_status(session.info.status))
            .ok_or_else(|| AppError::NotFound(format!("debug session {session_id}")))
    }

    fn breakpoints_update(
        &self,
        session_id: Uuid,
        path: &Path,
    ) -> AppResult<DebugBreakpointsUpdate> {
        let Some(session) = self.sessions.get(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        Ok(DebugBreakpointsUpdate {
            session_id,
            path: path.to_path_buf(),
            breakpoints: session
                .resolved_breakpoints_by_path
                .get(path)
                .cloned()
                .unwrap_or_default(),
        })
    }

    fn unverified_breakpoints_update(
        &self,
        session_id: Uuid,
        path: &Path,
    ) -> AppResult<DebugBreakpointsUpdate> {
        let Some(session) = self.sessions.get(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        Ok(DebugBreakpointsUpdate {
            session_id,
            path: path.to_path_buf(),
            breakpoints: session
                .breakpoints_by_path
                .get(path)
                .into_iter()
                .flatten()
                .map(|breakpoint| DebugResolvedBreakpoint {
                    id: None,
                    path: path.to_path_buf(),
                    line: breakpoint.line,
                    column: breakpoint.column,
                    verified: false,
                    message: Some("pending adapter verification".to_string()),
                })
                .collect(),
        })
    }

    fn with_session_mut<T>(
        &mut self,
        session_id: Uuid,
        update: impl FnOnce(&mut DebugSession) -> T,
    ) -> AppResult<T> {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        Ok(update(session))
    }

    fn next_request_seq(&mut self, session_id: Uuid) -> AppResult<u64> {
        self.with_session_mut(session_id, |session| {
            let seq = session.next_seq;
            session.next_seq += 1;
            seq
        })
    }

    fn session_info(&self, session_id: Uuid) -> AppResult<DebugSessionInfo> {
        self.sessions
            .get(&session_id)
            .map(|session| session.info.clone())
            .ok_or_else(|| AppError::NotFound(format!("debug session {session_id}")))
    }

    fn mark_started(
        &mut self,
        session_id: Uuid,
        request: DebugConfigurationRequest,
    ) -> AppResult<()> {
        self.with_session_mut(session_id, |session| {
            mark_session_started(&mut session.info, request);
        })
    }

    fn mark_session_error(&mut self, session_id: Uuid, error: &AppError) {
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.info.status = DebugSessionStatus::Error;
            session.info.stopped_at.get_or_insert_with(Utc::now);
            session.info.error = Some(error.to_string());
            session.info.last_event = Some("session error".to_string());
            let _result = self.emit_session(session_id);
        }
    }

    async fn fail_start(&mut self, session_id: Uuid, error: AppError) -> AppError {
        let message = error.to_string();
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.info.status = DebugSessionStatus::Error;
            session.info.stopped_at.get_or_insert_with(Utc::now);
            session.info.error = Some(message);
            session.info.last_event = Some("session start failed".to_string());
            let _ = session.child.start_kill();
            let _ = session.child.wait().await;
            session.read_task.abort();
            session.stderr_task.abort();
        }
        let _result = self.emit_session(session_id);
        error
    }

    fn ensure_stack_trace_allowed(&self, session_id: Uuid) -> AppResult<()> {
        self.ensure_paused(session_id, "stack trace")
    }

    fn ensure_debug_execution_allowed(&self, session_id: Uuid) -> AppResult<()> {
        self.ensure_paused(session_id, "execution control")
    }

    fn ensure_paused(&self, session_id: Uuid, action: &str) -> AppResult<()> {
        let Some(session) = self.sessions.get(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        if matches!(session.info.status, DebugSessionStatus::Paused) {
            Ok(())
        } else {
            Err(AppError::Service(format!(
                "debug session {} is not paused for {action}: {:?}",
                session.info.configuration_name, session.info.status
            )))
        }
    }

    async fn resolve_stack_thread(&mut self, session_id: Uuid) -> AppResult<DebugThreadInfo> {
        if let Some(thread) = self.active_or_first_thread(session_id) {
            return Ok(thread);
        }

        let threads_seq = self.next_request_seq(session_id)?;
        self.send_request(session_id, threads_request(threads_seq))
            .await?;
        let response = self
            .wait_for_response_body(session_id, threads_seq, "threads")
            .await?;
        let threads = parse_threads_response(response.as_ref());
        if threads.is_empty() {
            return Err(AppError::Service(
                "debug adapter returned no threads for stack trace".into(),
            ));
        }

        self.with_session_mut(session_id, |session| {
            session.threads = threads
                .iter()
                .map(|thread| (thread.id, thread.clone()))
                .collect();
            if session.info.active_thread_id.is_none() {
                session.info.active_thread_id = threads.first().map(|thread| thread.id);
            }
        })?;
        self.active_or_first_thread(session_id).ok_or_else(|| {
            AppError::Service("debug adapter returned no usable thread for stack trace".into())
        })
    }

    fn active_or_first_thread(&self, session_id: Uuid) -> Option<DebugThreadInfo> {
        let session = self.sessions.get(&session_id)?;
        session
            .info
            .active_thread_id
            .and_then(|thread_id| session.threads.get(&thread_id).cloned())
            .or_else(|| session.threads.values().next().cloned())
    }
}

fn validate_start_adapter(adapter: &DebugAdapterInfo) -> AppResult<()> {
    if adapter.status != DebugAdapterStatus::Available {
        return Err(AppError::Service(format!(
            "debug adapter is not available: {}",
            adapter
                .error
                .clone()
                .unwrap_or_else(|| adapter.command.clone())
        )));
    }
    Ok(())
}

async fn connect_tcp_debug_adapter(port: u16) -> AppResult<TcpStream> {
    let address = ("127.0.0.1", port);
    for attempt in 0..TCP_CONNECT_ATTEMPTS {
        match TcpStream::connect(address).await {
            Ok(stream) => return Ok(stream),
            Err(error) if attempt + 1 == TCP_CONNECT_ATTEMPTS => {
                return Err(AppError::Service(format!(
                    "debug adapter TCP server 127.0.0.1:{port} did not accept connections: {error}"
                )));
            }
            Err(_) => tokio::time::sleep(TCP_CONNECT_DELAY).await,
        }
    }
    Err(AppError::Service(format!(
        "debug adapter TCP server 127.0.0.1:{port} did not accept connections"
    )))
}

async fn tcp_server_args_and_port(adapter: &DebugAdapterInfo) -> AppResult<(Vec<String>, u16)> {
    let Some((flag_index, value_index)) = tcp_server_port_arg_indices(&adapter.args) else {
        return Err(AppError::Service(format!(
            "debug adapter {} uses TCP transport without a configured port",
            adapter.name
        )));
    };
    let mut args = adapter.args.clone();
    let configured_port = args[value_index].parse::<u16>().map_err(|error| {
        AppError::Service(format!(
            "debug adapter {} has invalid {} value {}: {error}",
            adapter.name, args[flag_index], args[value_index]
        ))
    })?;
    let port = if configured_port == 0 {
        allocate_loopback_port().await?
    } else {
        configured_port
    };
    args[value_index] = port.to_string();
    Ok((args, port))
}

fn tcp_server_port_arg_indices(args: &[String]) -> Option<(usize, usize)> {
    args.windows(2).enumerate().find_map(|(index, window)| {
        matches!(window, [flag, _] if flag == "--port" || flag == "-p")
            .then_some((index, index + 1))
    })
}

async fn allocate_loopback_port() -> AppResult<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn validate_breakpoint_session(session_id: Uuid, session: Option<&DebugSession>) -> AppResult<()> {
    let Some(session) = session else {
        return Err(AppError::NotFound(format!("debug session {session_id}")));
    };
    if matches!(
        session.info.status,
        DebugSessionStatus::Stopping | DebugSessionStatus::Stopped | DebugSessionStatus::Error
    ) {
        return Err(AppError::Service(format!(
            "debug session {} cannot accept breakpoints while {:?}",
            session.info.configuration_name, session.info.status
        )));
    }
    Ok(())
}

fn normalize_breakpoint_path(path: PathBuf) -> AppResult<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(AppError::InvalidPath("breakpoint path is empty".into()));
    }
    Ok(path)
}

const fn is_terminal_status(status: DebugSessionStatus) -> bool {
    matches!(
        status,
        DebugSessionStatus::Stopped | DebugSessionStatus::Error
    )
}

fn group_source_breakpoints(
    breakpoints: Vec<DebugSourceBreakpoint>,
) -> BTreeMap<PathBuf, Vec<DebugSourceBreakpoint>> {
    let mut grouped = BTreeMap::new();
    for breakpoint in breakpoints {
        let Ok(path) = normalize_breakpoint_path(breakpoint.path.clone()) else {
            continue;
        };
        let breakpoints = sanitize_source_breakpoints(&path, vec![breakpoint]);
        if !breakpoints.is_empty() {
            grouped
                .entry(path)
                .or_insert_with(Vec::new)
                .extend(breakpoints);
        }
    }
    for breakpoints in grouped.values_mut() {
        breakpoints.sort_by_key(|breakpoint| (breakpoint.line, breakpoint.column.unwrap_or(0)));
        breakpoints.dedup_by_key(|breakpoint| (breakpoint.line, breakpoint.column.unwrap_or(0)));
    }
    grouped
}

fn sanitize_source_breakpoints(
    path: &Path,
    breakpoints: Vec<DebugSourceBreakpoint>,
) -> Vec<DebugSourceBreakpoint> {
    let mut sanitized = breakpoints
        .into_iter()
        .filter(|breakpoint| breakpoint.line > 0)
        .map(|breakpoint| DebugSourceBreakpoint {
            path: path.to_path_buf(),
            line: breakpoint.line,
            column: breakpoint.column.filter(|column| *column > 0),
            condition: non_empty_text(breakpoint.condition.as_deref()),
            log_message: non_empty_text(breakpoint.log_message.as_deref()),
        })
        .collect::<Vec<_>>();
    sanitized.sort_by_key(|breakpoint| (breakpoint.line, breakpoint.column.unwrap_or(0)));
    sanitized.dedup_by_key(|breakpoint| (breakpoint.line, breakpoint.column.unwrap_or(0)));
    sanitized
}

fn non_empty_text(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn apply_breakpoints_response(
    session_id: Uuid,
    session: &mut DebugSession,
    response: DapResponse,
) -> Option<DebugBreakpointsUpdate> {
    let path = session
        .pending_breakpoint_requests
        .remove(&response.request_seq)?;
    let requested = session
        .breakpoints_by_path
        .get(&path)
        .cloned()
        .unwrap_or_default();
    let breakpoints = if response.success {
        parse_breakpoints_response(&path, &requested, response.body.as_ref())
    } else {
        let message = response
            .message
            .unwrap_or_else(|| "debug adapter rejected setBreakpoints request".to_string());
        requested
            .iter()
            .map(|breakpoint| unresolved_breakpoint(&path, breakpoint, &message))
            .collect()
    };
    session
        .resolved_breakpoints_by_path
        .insert(path.clone(), breakpoints.clone());
    Some(DebugBreakpointsUpdate {
        session_id,
        path,
        breakpoints,
    })
}

fn store_pending_response_by_seq(
    responses: &mut BTreeMap<u64, DapResponse>,
    response: DapResponse,
) {
    responses.insert(response.request_seq, response);
}

fn take_pending_response_by_seq(
    responses: &mut BTreeMap<u64, DapResponse>,
    request_seq: u64,
) -> Option<DapResponse> {
    responses.remove(&request_seq)
}

fn mark_session_started(info: &mut DebugSessionInfo, request: DebugConfigurationRequest) {
    if matches!(
        info.status,
        DebugSessionStatus::Starting | DebugSessionStatus::Running
    ) {
        info.status = DebugSessionStatus::Running;
        info.last_event = Some(match request {
            DebugConfigurationRequest::Launch => "launch configured".to_string(),
            DebugConfigurationRequest::Attach => "attach configured".to_string(),
        });
    }
}

fn mark_session_if_adapter_exited(session: &mut DebugSession) -> bool {
    if matches!(
        session.info.status,
        DebugSessionStatus::Stopped | DebugSessionStatus::Error
    ) {
        return false;
    }

    match session.child.try_wait() {
        Ok(Some(status)) => {
            let expected_stop = matches!(session.info.status, DebugSessionStatus::Stopping);
            mark_session_adapter_exited(&mut session.info, status, expected_stop);
            session.read_task.abort();
            session.stderr_task.abort();
            true
        }
        Ok(None) => false,
        Err(error) => {
            session.info.status = DebugSessionStatus::Error;
            session.info.stopped_at.get_or_insert_with(Utc::now);
            session.info.last_event = Some("adapter process status failed".to_string());
            session.info.error = Some(format!("debug adapter process status failed: {error}"));
            session.read_task.abort();
            session.stderr_task.abort();
            true
        }
    }
}

fn mark_session_adapter_exited(
    info: &mut DebugSessionInfo,
    status: ExitStatus,
    expected_stop: bool,
) {
    info.status = if expected_stop || status.success() {
        DebugSessionStatus::Stopped
    } else {
        DebugSessionStatus::Error
    };
    info.stopped_at.get_or_insert_with(Utc::now);
    info.last_event = Some(format!("adapter process exited: {status}"));
    if !expected_stop && !status.success() {
        info.error = Some(format!("debug adapter process exited with {status}"));
    }
}

struct BuiltinDebugAdapter {
    id: &'static str,
    name: &'static str,
    command: &'static str,
    args: &'static [&'static str],
    configuration_types: &'static [&'static str],
    transport: DebugAdapterTransport,
    marker_files: &'static [&'static str],
    marker_extensions: &'static [&'static str],
}

const BUILTIN_ADAPTERS: &[BuiltinDebugAdapter] = &[
    BuiltinDebugAdapter {
        id: "codelldb",
        name: "CodeLLDB",
        command: "codelldb",
        args: &["--port", "0"],
        configuration_types: &["codelldb", "lldb"],
        transport: DebugAdapterTransport::TcpServer,
        marker_files: &["Cargo.toml"],
        marker_extensions: &["rs"],
    },
    BuiltinDebugAdapter {
        id: "js-debug",
        name: "JavaScript Debug Terminal",
        command: "js-debug-adapter",
        args: &[],
        configuration_types: &[
            "js-debug",
            "node",
            "pwa-node",
            "node-terminal",
            "extensionHost",
        ],
        transport: DebugAdapterTransport::Stdio,
        marker_files: &["package.json", "tsconfig.json", "jsconfig.json"],
        marker_extensions: &["js", "jsx", "ts", "tsx"],
    },
    BuiltinDebugAdapter {
        id: "debugpy",
        name: "Python Debugpy",
        command: "python",
        args: &["-m", "debugpy.adapter"],
        configuration_types: &["debugpy", "python"],
        transport: DebugAdapterTransport::Stdio,
        marker_files: &["pyproject.toml", "requirements.txt", "setup.py"],
        marker_extensions: &["py"],
    },
];

pub fn workspace_debug_info(root: impl AsRef<Path>) -> AppResult<DebugWorkspaceInfo> {
    let root = root.as_ref().canonicalize()?;
    let adapters = workspace_debug_adapters(&root)?;
    let (launch_json_path, configurations) = read_launch_configurations(&root)?;
    Ok(DebugWorkspaceInfo {
        adapters,
        configurations,
        launch_json_path,
    })
}

pub fn workspace_debug_adapters(root: impl AsRef<Path>) -> AppResult<Vec<DebugAdapterInfo>> {
    let root = root.as_ref().canonicalize()?;
    let detected_files = detect_files(&root)?;
    let detected_extensions = detect_extensions(&root)?;
    let mut adapters = Vec::new();

    for adapter in BUILTIN_ADAPTERS {
        let matches_file = adapter
            .marker_files
            .iter()
            .any(|file| detected_files.contains(*file));
        let matches_extension = adapter
            .marker_extensions
            .iter()
            .any(|extension| detected_extensions.contains(*extension));
        if !matches_file && !matches_extension {
            continue;
        }

        let available = command_available(adapter.command);
        adapters.push(DebugAdapterInfo {
            id: adapter.id.to_string(),
            name: adapter.name.to_string(),
            command: adapter.command.to_string(),
            args: adapter.args.iter().map(|arg| (*arg).to_string()).collect(),
            configuration_types: adapter
                .configuration_types
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            transport: adapter.transport,
            workspace_root: root.clone(),
            status: if available {
                DebugAdapterStatus::Available
            } else {
                DebugAdapterStatus::Missing
            },
            error: if available {
                None
            } else {
                Some(format!("{} was not found in PATH", adapter.command))
            },
        });
    }

    Ok(adapters)
}

#[must_use]
pub fn adapter_matches_configuration(
    adapter: &DebugAdapterInfo,
    configuration: &DebugConfiguration,
) -> bool {
    adapter
        .configuration_types
        .iter()
        .any(|adapter_type| adapter_type.eq_ignore_ascii_case(&configuration.adapter_type))
        || adapter.id.eq_ignore_ascii_case(&configuration.adapter_type)
        || adapter
            .command
            .eq_ignore_ascii_case(&configuration.adapter_type)
}

pub fn workspace_debug_adapter_for_configuration(
    root: impl AsRef<Path>,
    configuration: &DebugConfiguration,
) -> AppResult<Option<DebugAdapterInfo>> {
    Ok(workspace_debug_adapters(root)?
        .into_iter()
        .find(|adapter| adapter_matches_configuration(adapter, configuration)))
}

pub fn encode_dap_message(value: &Value) -> AppResult<Vec<u8>> {
    let content = serde_json::to_vec(value)?;
    let mut message = format!("Content-Length: {}\r\n\r\n", content.len()).into_bytes();
    message.extend_from_slice(&content);
    Ok(message)
}

pub fn drain_dap_frames(buffer: &mut Vec<u8>) -> AppResult<Vec<DapFrame>> {
    let mut frames = Vec::new();

    while let Some(header_end) = find_header_end(buffer) {
        let headers = std::str::from_utf8(&buffer[..header_end])
            .map_err(|error| AppError::Service(format!("invalid DAP header encoding: {error}")))?;
        let content_length = parse_content_length(headers)?;
        let frame_start = header_end + 4;
        let frame_end = frame_start + content_length;

        if buffer.len() < frame_end {
            break;
        }

        let content = buffer[frame_start..frame_end].to_vec();
        buffer.drain(..frame_end);
        frames.push(DapFrame { content });
    }

    Ok(frames)
}

pub fn parse_dap_message(frame: &DapFrame) -> AppResult<Option<DapMessage>> {
    let value: Value = serde_json::from_slice(&frame.content)?;
    Ok(parse_dap_message_value(&value))
}

#[must_use]
pub fn initialize_request(seq: u64, adapter_id: &str) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "initialize",
        "arguments": {
            "clientID": "lux-ide",
            "clientName": "Lux IDE",
            "adapterID": adapter_id,
            "pathFormat": "path",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "supportsVariableType": true,
            "supportsVariablePaging": true,
            "supportsRunInTerminalRequest": true,
            "supportsProgressReporting": true,
            "supportsInvalidatedEvent": true,
        }
    })
}

#[must_use]
pub fn launch_request(seq: u64, configuration: &DebugConfiguration) -> Value {
    debug_configuration_request(seq, "launch", configuration)
}

#[must_use]
pub fn attach_request(seq: u64, configuration: &DebugConfiguration) -> Value {
    debug_configuration_request(seq, "attach", configuration)
}

#[must_use]
pub fn configuration_done_request(seq: u64) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "configurationDone",
        "arguments": {}
    })
}

#[must_use]
pub fn disconnect_request(seq: u64, terminate_debuggee: bool) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "disconnect",
        "arguments": {
            "terminateDebuggee": terminate_debuggee,
        }
    })
}

#[must_use]
pub fn threads_request(seq: u64) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "threads",
        "arguments": {}
    })
}

#[must_use]
pub fn stack_trace_request(seq: u64, thread_id: u64, start_frame: u64, levels: u64) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "stackTrace",
        "arguments": {
            "threadId": thread_id,
            "startFrame": start_frame,
            "levels": levels,
        }
    })
}

#[must_use]
pub fn scopes_request(seq: u64, frame_id: u64) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "scopes",
        "arguments": {
            "frameId": frame_id,
        }
    })
}

#[must_use]
pub fn variables_request(seq: u64, variables_reference: u64, start: u64, count: u64) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "variables",
        "arguments": {
            "variablesReference": variables_reference,
            "start": start,
            "count": count,
        }
    })
}

#[must_use]
pub fn evaluate_request(
    seq: u64,
    expression: &str,
    frame_id: Option<u64>,
    context: DebugEvaluateContext,
) -> Value {
    let mut arguments = serde_json::Map::new();
    arguments.insert(
        "expression".to_string(),
        Value::String(expression.to_string()),
    );
    arguments.insert(
        "context".to_string(),
        Value::String(evaluate_context_name(context).to_string()),
    );
    if let Some(frame_id) = frame_id {
        arguments.insert("frameId".to_string(), Value::from(frame_id));
    }

    json!({
        "seq": seq,
        "type": "request",
        "command": "evaluate",
        "arguments": arguments,
    })
}

#[must_use]
pub fn set_breakpoints_request(
    seq: u64,
    path: &Path,
    breakpoints: &[DebugSourceBreakpoint],
) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "setBreakpoints",
        "arguments": {
            "source": {
                "path": path,
            },
            "breakpoints": breakpoints.iter().map(source_breakpoint_argument).collect::<Vec<_>>(),
            "sourceModified": false,
        }
    })
}

#[must_use]
pub fn execution_request(seq: u64, action: DebugExecutionAction, thread_id: u64) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": execution_action_command(action),
        "arguments": {
            "threadId": thread_id,
        }
    })
}

#[must_use]
pub fn parse_breakpoints_response(
    path: &Path,
    requested: &[DebugSourceBreakpoint],
    body: Option<&Value>,
) -> Vec<DebugResolvedBreakpoint> {
    let Some(items) = body
        .and_then(|value| value.get("breakpoints"))
        .and_then(Value::as_array)
    else {
        return requested
            .iter()
            .map(|breakpoint| {
                unresolved_breakpoint(path, breakpoint, "adapter returned no breakpoint data")
            })
            .collect();
    };

    requested
        .iter()
        .enumerate()
        .map(|(index, requested_breakpoint)| {
            items.get(index).map_or_else(
                || unresolved_breakpoint(path, requested_breakpoint, "adapter omitted breakpoint"),
                |value| parse_resolved_breakpoint(path, requested_breakpoint, value),
            )
        })
        .collect()
}

fn source_breakpoint_argument(breakpoint: &DebugSourceBreakpoint) -> Value {
    let mut value = serde_json::Map::new();
    value.insert("line".to_string(), Value::from(breakpoint.line));
    if let Some(column) = breakpoint.column {
        value.insert("column".to_string(), Value::from(column));
    }
    if let Some(condition) = non_empty_text(breakpoint.condition.as_deref()) {
        value.insert("condition".to_string(), Value::String(condition));
    }
    if let Some(log_message) = non_empty_text(breakpoint.log_message.as_deref()) {
        value.insert("logMessage".to_string(), Value::String(log_message));
    }
    Value::Object(value)
}

fn parse_resolved_breakpoint(
    path: &Path,
    requested: &DebugSourceBreakpoint,
    value: &Value,
) -> DebugResolvedBreakpoint {
    DebugResolvedBreakpoint {
        id: value.get("id").and_then(Value::as_u64),
        path: value
            .get("source")
            .and_then(|source| source.get("path"))
            .and_then(Value::as_str)
            .map_or_else(|| path.to_path_buf(), PathBuf::from),
        line: value
            .get("line")
            .and_then(Value::as_u64)
            .unwrap_or(requested.line),
        column: value
            .get("column")
            .and_then(Value::as_u64)
            .or(requested.column),
        verified: value
            .get("verified")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        message: value
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

fn unresolved_breakpoint(
    path: &Path,
    breakpoint: &DebugSourceBreakpoint,
    message: &str,
) -> DebugResolvedBreakpoint {
    DebugResolvedBreakpoint {
        id: None,
        path: path.to_path_buf(),
        line: breakpoint.line,
        column: breakpoint.column,
        verified: false,
        message: Some(message.to_string()),
    }
}

#[must_use]
pub const fn execution_action_command(action: DebugExecutionAction) -> &'static str {
    match action {
        DebugExecutionAction::Continue => "continue",
        DebugExecutionAction::StepOver => "next",
        DebugExecutionAction::StepIn => "stepIn",
        DebugExecutionAction::StepOut => "stepOut",
    }
}

#[must_use]
pub const fn evaluate_context_name(context: DebugEvaluateContext) -> &'static str {
    match context {
        DebugEvaluateContext::Repl => "repl",
        DebugEvaluateContext::Watch => "watch",
        DebugEvaluateContext::Hover => "hover",
        DebugEvaluateContext::Clipboard => "clipboard",
        DebugEvaluateContext::Variables => "variables",
    }
}

#[must_use]
pub fn parse_threads_response(body: Option<&Value>) -> Vec<DebugThreadInfo> {
    body.and_then(|value| value.get("threads"))
        .and_then(Value::as_array)
        .map(|threads| threads.iter().filter_map(parse_thread_info).collect())
        .unwrap_or_default()
}

#[must_use]
pub fn parse_stack_trace_response(
    session_id: Uuid,
    thread: DebugThreadInfo,
    body: Option<&Value>,
) -> DebugStackTrace {
    let frames = body
        .and_then(|value| value.get("stackFrames"))
        .and_then(Value::as_array)
        .map(|frames| frames.iter().filter_map(parse_stack_frame).collect())
        .unwrap_or_default();
    let total_frames = body
        .and_then(|value| value.get("totalFrames"))
        .and_then(Value::as_u64);

    DebugStackTrace {
        session_id,
        thread,
        frames,
        total_frames,
    }
}

#[must_use]
pub fn parse_scopes_response(
    session_id: Uuid,
    frame_id: u64,
    body: Option<&Value>,
) -> DebugFrameScopes {
    let scopes = body
        .and_then(|value| value.get("scopes"))
        .and_then(Value::as_array)
        .map(|scopes| scopes.iter().filter_map(parse_scope_info).collect())
        .unwrap_or_default();

    DebugFrameScopes {
        session_id,
        frame_id,
        scopes,
    }
}

#[must_use]
pub fn parse_variables_response(
    session_id: Uuid,
    variables_reference: u64,
    body: Option<&Value>,
) -> DebugVariables {
    let variables = body
        .and_then(|value| value.get("variables"))
        .and_then(Value::as_array)
        .map(|variables| variables.iter().filter_map(parse_variable_info).collect())
        .unwrap_or_default();

    DebugVariables {
        session_id,
        variables_reference,
        variables,
    }
}

pub fn parse_evaluate_response(
    session_id: Uuid,
    expression: String,
    body: Option<&Value>,
) -> AppResult<DebugEvaluateResult> {
    let Some(body) = body else {
        return Err(AppError::Service(
            "debug adapter returned no evaluate body".into(),
        ));
    };
    let result = body
        .get("result")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Service("debug adapter returned no evaluate result".into()))?
        .to_string();

    Ok(DebugEvaluateResult {
        session_id,
        expression,
        result,
        type_name: body.get("type").and_then(Value::as_str).map(str::to_string),
        variables_reference: body
            .get("variablesReference")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        named_variables: body.get("namedVariables").and_then(Value::as_u64),
        indexed_variables: body.get("indexedVariables").and_then(Value::as_u64),
        memory_reference: body
            .get("memoryReference")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn parse_scope_info(value: &Value) -> Option<DebugScopeInfo> {
    Some(DebugScopeInfo {
        name: value.get("name")?.as_str()?.to_string(),
        variables_reference: value.get("variablesReference")?.as_u64()?,
        expensive: value
            .get("expensive")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        named_variables: value.get("namedVariables").and_then(Value::as_u64),
        indexed_variables: value.get("indexedVariables").and_then(Value::as_u64),
    })
}

fn parse_variable_info(value: &Value) -> Option<DebugVariableInfo> {
    Some(DebugVariableInfo {
        name: value.get("name")?.as_str()?.to_string(),
        value: value
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        type_name: value
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string),
        variables_reference: value
            .get("variablesReference")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        evaluate_name: value
            .get("evaluateName")
            .and_then(Value::as_str)
            .map(str::to_string),
        named_variables: value.get("namedVariables").and_then(Value::as_u64),
        indexed_variables: value.get("indexedVariables").and_then(Value::as_u64),
    })
}

fn parse_thread_info(value: &Value) -> Option<DebugThreadInfo> {
    Some(DebugThreadInfo {
        id: value.get("id")?.as_u64()?,
        name: value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Thread")
            .to_string(),
    })
}

fn parse_stack_frame(value: &Value) -> Option<DebugStackFrame> {
    Some(DebugStackFrame {
        id: value.get("id")?.as_u64()?,
        name: value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("frame")
            .to_string(),
        source_path: value
            .get("source")
            .and_then(|source| source.get("path"))
            .and_then(Value::as_str)
            .map(PathBuf::from),
        line: value.get("line").and_then(Value::as_u64).unwrap_or(0),
        column: value.get("column").and_then(Value::as_u64).unwrap_or(0),
    })
}

fn stopped_event_thread_id(body: Option<&Value>) -> Option<u64> {
    body.and_then(|value| value.get("threadId"))
        .and_then(Value::as_u64)
}

fn debug_configuration_request(
    seq: u64,
    command: &str,
    configuration: &DebugConfiguration,
) -> Value {
    let mut arguments = serde_json::Map::new();
    for (key, value) in &configuration.raw {
        arguments.insert(key.clone(), value.clone());
    }
    arguments.insert(
        "name".to_string(),
        Value::String(configuration.name.clone()),
    );
    arguments.insert(
        "type".to_string(),
        Value::String(configuration.adapter_type.clone()),
    );
    arguments.insert("request".to_string(), Value::String(command.to_string()));

    json!({
        "seq": seq,
        "type": "request",
        "command": command,
        "arguments": arguments,
    })
}

fn read_launch_configurations(
    root: &Path,
) -> AppResult<(Option<PathBuf>, Vec<DebugConfiguration>)> {
    let path = root.join(".vscode").join("launch.json");
    if !path.is_file() {
        return Ok((None, Vec::new()));
    }

    let contents = std::fs::read_to_string(&path)?;
    let value: Value = serde_json::from_str(&jsonc_to_json(&contents))?;
    let configurations = value
        .get("configurations")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(parse_debug_configuration).collect())
        .unwrap_or_default();
    Ok((Some(path), configurations))
}

fn parse_debug_configuration(value: &Value) -> Option<DebugConfiguration> {
    let object = value.as_object()?;
    let name = object.get("name")?.as_str()?.trim();
    let adapter_type = object.get("type")?.as_str()?.trim();
    let request = parse_debug_request(object.get("request")?.as_str()?)?;
    if name.is_empty() || adapter_type.is_empty() {
        return None;
    }

    Some(DebugConfiguration {
        name: name.to_string(),
        adapter_type: adapter_type.to_string(),
        request,
        raw: object.clone(),
    })
}

fn parse_debug_request(value: &str) -> Option<DebugConfigurationRequest> {
    match value {
        "launch" => Some(DebugConfigurationRequest::Launch),
        "attach" => Some(DebugConfigurationRequest::Attach),
        _ => None,
    }
}

fn jsonc_to_json(value: &str) -> String {
    remove_trailing_commas(&strip_jsonc_comments(value))
}

fn strip_jsonc_comments(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_string {
            result.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            result.push(ch);
            continue;
        }

        if ch == '/' {
            match chars.peek().copied() {
                Some('/') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\n' {
                            result.push('\n');
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    chars.next();
                    let mut previous = '\0';
                    for next in chars.by_ref() {
                        if next == '\n' {
                            result.push('\n');
                        }
                        if previous == '*' && next == '/' {
                            break;
                        }
                        previous = next;
                    }
                    continue;
                }
                _ => {}
            }
        }

        result.push(ch);
    }

    result
}

fn remove_trailing_commas(value: &str) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    let mut result = String::with_capacity(value.len());
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;

    while index < chars.len() {
        let ch = chars[index];
        if in_string {
            result.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }

        if ch == '"' {
            in_string = true;
            result.push(ch);
            index += 1;
            continue;
        }

        if ch == ',' {
            let mut next_index = index + 1;
            while next_index < chars.len() && chars[next_index].is_whitespace() {
                next_index += 1;
            }
            if matches!(chars.get(next_index), Some(']' | '}')) {
                index += 1;
                continue;
            }
        }

        result.push(ch);
        index += 1;
    }

    result
}

fn parse_dap_message_value(value: &Value) -> Option<DapMessage> {
    match value.get("type")?.as_str()? {
        "response" => Some(DapMessage::Response(DapResponse {
            request_seq: value.get("request_seq")?.as_u64()?,
            success: value
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            command: value.get("command")?.as_str()?.to_string(),
            message: value
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string),
            body: value.get("body").cloned(),
        })),
        "event" => Some(DapMessage::Event(DapEvent {
            event: value.get("event")?.as_str()?.to_string(),
            body: value.get("body").cloned(),
        })),
        "request" => Some(DapMessage::Request {
            command: value.get("command")?.as_str()?.to_string(),
        }),
        _ => None,
    }
}

async fn read_dap_stdout<R>(
    mut stdout: R,
    session_id: Uuid,
    message_tx: mpsc::UnboundedSender<DapMessage>,
    update_tx: mpsc::UnboundedSender<DebugSessionUpdate>,
) where
    R: AsyncRead + Unpin,
{
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8192];

    loop {
        let read = match stdout.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) => {
                let _ = update_tx.send(DebugSessionUpdate::Output {
                    session_id,
                    text: format!("debug adapter stdout read failed: {error}\n"),
                });
                break;
            }
        };
        buffer.extend_from_slice(&chunk[..read]);

        let frames = match drain_dap_frames(&mut buffer) {
            Ok(frames) => frames,
            Err(error) => {
                buffer.clear();
                let _ = update_tx.send(DebugSessionUpdate::Output {
                    session_id,
                    text: format!("debug adapter emitted invalid DAP frame: {error}\n"),
                });
                continue;
            }
        };

        for frame in frames {
            match parse_dap_message(&frame) {
                Ok(Some(message)) => {
                    let _ = message_tx.send(message);
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = update_tx.send(DebugSessionUpdate::Output {
                        session_id,
                        text: format!("debug adapter emitted invalid DAP message: {error}\n"),
                    });
                }
            }
        }
    }
}

async fn drain_debug_stderr<R>(
    mut stderr: R,
    session_id: Uuid,
    update_tx: mpsc::UnboundedSender<DebugSessionUpdate>,
) where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 4096];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let text = String::from_utf8_lossy(&buffer[..read]).into_owned();
                if !text.is_empty() {
                    let _ = update_tx.send(DebugSessionUpdate::Output { session_id, text });
                }
            }
        }
    }
}

fn detect_files(root: &Path) -> AppResult<BTreeSet<String>> {
    let mut files = BTreeSet::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            files.insert(entry.file_name().to_string_lossy().to_string());
        }
    }
    Ok(files)
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
        let extensions = env::var_os("PATHEXT").map_or_else(
            || {
                vec![
                    ".COM".to_string(),
                    ".EXE".to_string(),
                    ".BAT".to_string(),
                    ".CMD".to_string(),
                ]
            },
            |value| {
                value
                    .to_string_lossy()
                    .split(';')
                    .filter(|extension| !extension.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            },
        );

        for extension in extensions {
            if dir.join(format!("{command}{extension}")).is_file() {
                return true;
            }
        }
    }

    false
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
                AppError::Service(format!("invalid DAP Content-Length: {error}"))
            });
        }
    }

    Err(AppError::Service(
        "DAP frame is missing Content-Length header".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn drain_dap_frames_extracts_complete_frames_and_keeps_partial_tail() {
        let first = br#"{"seq":1,"type":"event","event":"initialized"}"#;
        let second =
            br#"{"seq":2,"type":"response","request_seq":1,"success":true,"command":"initialize"}"#;
        let mut buffer = Vec::new();
        buffer.extend_from_slice(format!("Content-Length: {}\r\n\r\n", first.len()).as_bytes());
        buffer.extend_from_slice(first);
        buffer.extend_from_slice(format!("Content-Length: {}\r\n\r\n", second.len()).as_bytes());
        buffer.extend_from_slice(&second[..8]);

        let frames = drain_dap_frames(&mut buffer).expect("valid frame should parse");

        assert_eq!(
            frames,
            vec![DapFrame {
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
    fn parse_dap_message_accepts_events_and_responses() {
        let event = DapFrame {
            content:
                br#"{"seq":1,"type":"event","event":"stopped","body":{"reason":"breakpoint"}}"#
                    .to_vec(),
        };
        let response = DapFrame {
            content: br#"{"seq":2,"type":"response","request_seq":1,"success":false,"command":"launch","message":"failed"}"#.to_vec(),
        };

        assert_eq!(
            parse_dap_message(&event).expect("event should parse"),
            Some(DapMessage::Event(DapEvent {
                event: "stopped".to_string(),
                body: Some(json!({"reason":"breakpoint"})),
            }))
        );
        assert_eq!(
            parse_dap_message(&response).expect("response should parse"),
            Some(DapMessage::Response(DapResponse {
                request_seq: 1,
                success: false,
                command: "launch".to_string(),
                message: Some("failed".to_string()),
                body: None,
            }))
        );
    }

    #[test]
    fn request_builders_emit_dap_initialize_launch_disconnect_and_stack_trace() {
        let configuration = DebugConfiguration {
            name: "Run binary".to_string(),
            adapter_type: "codelldb".to_string(),
            request: DebugConfigurationRequest::Launch,
            raw: serde_json::Map::from_iter([
                ("program".to_string(), json!("target/debug/app")),
                ("cwd".to_string(), json!("${workspaceFolder}")),
            ]),
        };

        let initialize = initialize_request(1, "codelldb");
        let launch = launch_request(2, &configuration);
        let disconnect = disconnect_request(3, true);
        let threads = threads_request(4);
        let stack_trace = stack_trace_request(5, 42, 0, 64);
        let continue_request = execution_request(6, DebugExecutionAction::Continue, 42);
        let step_over_request = execution_request(7, DebugExecutionAction::StepOver, 42);
        let step_in_request = execution_request(8, DebugExecutionAction::StepIn, 42);
        let step_out_request = execution_request(9, DebugExecutionAction::StepOut, 42);
        let scopes = scopes_request(10, 100);
        let variables = variables_request(11, 200, 0, 200);
        let evaluate = evaluate_request(12, "items.len()", Some(100), DebugEvaluateContext::Watch);

        assert_eq!(initialize["command"], "initialize");
        assert_eq!(initialize["arguments"]["adapterID"], "codelldb");
        assert_eq!(launch["command"], "launch");
        assert_eq!(launch["arguments"]["name"], "Run binary");
        assert_eq!(launch["arguments"]["type"], "codelldb");
        assert_eq!(launch["arguments"]["request"], "launch");
        assert_eq!(launch["arguments"]["program"], "target/debug/app");
        assert_eq!(disconnect["command"], "disconnect");
        assert_eq!(disconnect["arguments"]["terminateDebuggee"], true);
        assert_eq!(threads["command"], "threads");
        assert_eq!(stack_trace["command"], "stackTrace");
        assert_eq!(stack_trace["arguments"]["threadId"], 42);
        assert_eq!(stack_trace["arguments"]["levels"], 64);
        assert_eq!(continue_request["command"], "continue");
        assert_eq!(step_over_request["command"], "next");
        assert_eq!(step_in_request["command"], "stepIn");
        assert_eq!(step_out_request["command"], "stepOut");
        assert_eq!(step_out_request["arguments"]["threadId"], 42);
        assert_eq!(scopes["command"], "scopes");
        assert_eq!(scopes["arguments"]["frameId"], 100);
        assert_eq!(variables["command"], "variables");
        assert_eq!(variables["arguments"]["variablesReference"], 200);
        assert_eq!(variables["arguments"]["count"], 200);
        assert_eq!(evaluate["command"], "evaluate");
        assert_eq!(evaluate["arguments"]["expression"], "items.len()");
        assert_eq!(evaluate["arguments"]["frameId"], 100);
        assert_eq!(evaluate["arguments"]["context"], "watch");
    }

    #[test]
    fn breakpoint_request_and_parser_map_dap_breakpoints() {
        let path = PathBuf::from("src/main.rs");
        let breakpoints = vec![
            DebugSourceBreakpoint {
                path: path.clone(),
                line: 12,
                column: None,
                condition: Some(" value > 0 ".to_string()),
                log_message: None,
            },
            DebugSourceBreakpoint {
                path: path.clone(),
                line: 18,
                column: Some(3),
                condition: None,
                log_message: Some("hit {value}".to_string()),
            },
        ];
        let request = set_breakpoints_request(10, &path, &breakpoints);

        assert_eq!(request["command"], "setBreakpoints");
        assert_eq!(request["arguments"]["source"]["path"], "src/main.rs");
        assert_eq!(request["arguments"]["breakpoints"][0]["line"], 12);
        assert_eq!(
            request["arguments"]["breakpoints"][0]["condition"],
            "value > 0"
        );
        assert_eq!(request["arguments"]["breakpoints"][1]["column"], 3);
        assert_eq!(
            request["arguments"]["breakpoints"][1]["logMessage"],
            "hit {value}"
        );

        let response_body = json!({
            "breakpoints": [
                {"id": 1, "verified": true, "line": 12},
                {"id": 2, "verified": false, "message": "moved", "line": 20, "column": 1}
            ]
        });
        let resolved = parse_breakpoints_response(&path, &breakpoints, Some(&response_body));

        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].id, Some(1));
        assert!(resolved[0].verified);
        assert_eq!(resolved[0].line, 12);
        assert_eq!(resolved[1].id, Some(2));
        assert!(!resolved[1].verified);
        assert_eq!(resolved[1].line, 20);
        assert_eq!(resolved[1].column, Some(1));
        assert_eq!(resolved[1].message.as_deref(), Some("moved"));
    }

    #[test]
    fn mark_session_started_preserves_terminal_lifecycle_states() {
        let mut info = debug_session_info(DebugSessionStatus::Starting);

        mark_session_started(&mut info, DebugConfigurationRequest::Launch);

        assert_eq!(info.status, DebugSessionStatus::Running);
        assert_eq!(info.last_event.as_deref(), Some("launch configured"));

        let mut paused = debug_session_info(DebugSessionStatus::Paused);
        paused.active_thread_id = Some(42);
        mark_session_started(&mut paused, DebugConfigurationRequest::Attach);

        assert_eq!(paused.status, DebugSessionStatus::Paused);
        assert_eq!(paused.active_thread_id, Some(42));
        assert_eq!(paused.last_event, None);

        let mut stopped = debug_session_info(DebugSessionStatus::Stopped);
        stopped.stopped_at = Some(Utc::now());
        mark_session_started(&mut stopped, DebugConfigurationRequest::Launch);

        assert_eq!(stopped.status, DebugSessionStatus::Stopped);
        assert!(stopped.stopped_at.is_some());
        assert_eq!(stopped.last_event, None);
    }

    #[test]
    fn adapter_exit_after_user_stop_is_not_reported_as_crash() {
        let mut info = debug_session_info(DebugSessionStatus::Stopping);

        mark_session_adapter_exited(&mut info, exit_status(7), true);

        assert_eq!(info.status, DebugSessionStatus::Stopped);
        assert!(info.stopped_at.is_some());
        assert!(info.error.is_none());
        assert!(info
            .last_event
            .as_deref()
            .is_some_and(|event| event.starts_with("adapter process exited:")));
    }

    #[test]
    fn pending_response_store_round_trips_out_of_order_dap_responses() {
        let mut responses = BTreeMap::new();
        let launch = DapResponse {
            request_seq: 2,
            success: true,
            command: "launch".to_string(),
            message: None,
            body: Some(json!({"ok": true})),
        };
        let initialize = DapResponse {
            request_seq: 1,
            success: true,
            command: "initialize".to_string(),
            message: None,
            body: None,
        };

        store_pending_response_by_seq(&mut responses, launch.clone());
        store_pending_response_by_seq(&mut responses, initialize.clone());

        assert_eq!(
            take_pending_response_by_seq(&mut responses, 1),
            Some(initialize)
        );
        assert_eq!(
            take_pending_response_by_seq(&mut responses, 2),
            Some(launch)
        );
        assert!(responses.is_empty());
    }

    #[test]
    fn stack_trace_parser_maps_threads_and_frames() {
        let session_id = Uuid::new_v4();
        let threads_body = json!({
            "threads": [
                {"id": 42, "name": "main"},
                {"id": 7, "name": "worker"}
            ]
        });
        let threads = parse_threads_response(Some(&threads_body));

        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].id, 42);
        assert_eq!(threads[0].name, "main");

        let stack_body = json!({
            "stackFrames": [
                {
                    "id": 100,
                    "name": "handler",
                    "source": {"path": "src/main.rs"},
                    "line": 12,
                    "column": 5
                }
            ],
            "totalFrames": 1
        });
        let trace = parse_stack_trace_response(session_id, threads[0].clone(), Some(&stack_body));

        assert_eq!(trace.session_id, session_id);
        assert_eq!(trace.thread.id, 42);
        assert_eq!(trace.total_frames, Some(1));
        assert_eq!(trace.frames.len(), 1);
        assert_eq!(trace.frames[0].id, 100);
        assert_eq!(trace.frames[0].name, "handler");
        assert_eq!(
            trace.frames[0].source_path.as_deref(),
            Some(Path::new("src/main.rs"))
        );
        assert_eq!(trace.frames[0].line, 12);
    }

    #[test]
    fn scope_and_variable_parsers_map_dap_payloads() {
        let session_id = Uuid::new_v4();
        let scopes_body = json!({
            "scopes": [
                {
                    "name": "Locals",
                    "variablesReference": 10,
                    "expensive": false,
                    "namedVariables": 3,
                    "indexedVariables": 0
                }
            ]
        });
        let scopes = parse_scopes_response(session_id, 100, Some(&scopes_body));

        assert_eq!(scopes.session_id, session_id);
        assert_eq!(scopes.frame_id, 100);
        assert_eq!(scopes.scopes.len(), 1);
        assert_eq!(scopes.scopes[0].name, "Locals");
        assert_eq!(scopes.scopes[0].variables_reference, 10);
        assert_eq!(scopes.scopes[0].named_variables, Some(3));

        let variables_body = json!({
            "variables": [
                {
                    "name": "value",
                    "value": "42",
                    "type": "i32",
                    "variablesReference": 0,
                    "evaluateName": "value"
                },
                {
                    "name": "items",
                    "value": "Vec len=2",
                    "type": "Vec<i32>",
                    "variablesReference": 11,
                    "indexedVariables": 2
                }
            ]
        });
        let variables = parse_variables_response(session_id, 10, Some(&variables_body));

        assert_eq!(variables.session_id, session_id);
        assert_eq!(variables.variables_reference, 10);
        assert_eq!(variables.variables.len(), 2);
        assert_eq!(variables.variables[0].name, "value");
        assert_eq!(variables.variables[0].value, "42");
        assert_eq!(variables.variables[0].type_name.as_deref(), Some("i32"));
        assert_eq!(variables.variables[1].variables_reference, 11);
        assert_eq!(variables.variables[1].indexed_variables, Some(2));
    }

    #[test]
    fn evaluate_parser_maps_dap_payload() {
        let session_id = Uuid::new_v4();
        let body = json!({
            "result": "Vec len=2",
            "type": "Vec<i32>",
            "variablesReference": 11,
            "namedVariables": 1,
            "indexedVariables": 2,
            "memoryReference": "0x1000"
        });
        let evaluated = parse_evaluate_response(session_id, "items".to_string(), Some(&body))
            .expect("evaluate response should parse");

        assert_eq!(evaluated.session_id, session_id);
        assert_eq!(evaluated.expression, "items");
        assert_eq!(evaluated.result, "Vec len=2");
        assert_eq!(evaluated.type_name.as_deref(), Some("Vec<i32>"));
        assert_eq!(evaluated.variables_reference, 11);
        assert_eq!(evaluated.named_variables, Some(1));
        assert_eq!(evaluated.indexed_variables, Some(2));
        assert_eq!(evaluated.memory_reference.as_deref(), Some("0x1000"));

        let error = parse_evaluate_response(session_id, "items".to_string(), Some(&json!({})))
            .expect_err("evaluate result is required");
        assert!(error.to_string().contains("evaluate result"));
    }

    #[test]
    fn launch_json_parser_accepts_jsonc_comments_and_trailing_commas() {
        let root = unique_temp_dir("lux-dap-jsonc");
        std::fs::create_dir_all(root.join(".vscode")).expect("vscode dir should be created");
        std::fs::write(
            root.join(".vscode").join("launch.json"),
            r#"{
                // Cursor and VS Code launch files allow comments.
                "version": "0.2.0",
                "configurations": [
                    {
                        "name": "Run API",
                        "type": "debugpy",
                        "request": "launch",
                        "program": "${workspaceFolder}/app.py",
                    },
                ],
            }"#,
        )
        .expect("launch.json should be written");

        let info = workspace_debug_info(&root).expect("debug info should load");

        assert_eq!(info.configurations.len(), 1);
        assert_eq!(info.configurations[0].name, "Run API");
        assert_eq!(info.configurations[0].adapter_type, "debugpy");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_debug_info_reads_launch_json_and_detects_adapters() {
        let root = unique_temp_dir("lux-dap-workspace");
        std::fs::create_dir_all(root.join(".vscode")).expect("vscode dir should be created");
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"debug-test\"\n",
        )
        .expect("cargo manifest should be written");
        std::fs::write(
            root.join(".vscode").join("launch.json"),
            r#"{
                "version": "0.2.0",
                "configurations": [
                    {"name": "Run app", "type": "codelldb", "request": "launch", "program": "target/debug/app"},
                    {"name": "Attach app", "type": "codelldb", "request": "attach", "pid": 1234},
                    {"name": "Ignored", "type": "codelldb", "request": "unsupported"}
                ]
            }"#,
        )
        .expect("launch.json should be written");

        let info = workspace_debug_info(&root).expect("debug info should load");

        assert_eq!(info.configurations.len(), 2);
        assert_eq!(info.configurations[0].name, "Run app");
        assert_eq!(
            info.configurations[0].request,
            DebugConfigurationRequest::Launch
        );
        assert_eq!(
            info.configurations[1].request,
            DebugConfigurationRequest::Attach
        );
        assert_eq!(
            info.launch_json_path
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str()),
            Some("launch.json")
        );
        assert!(info.adapters.iter().any(|adapter| adapter.id == "codelldb"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn adapters_expose_transport_and_configuration_aliases() {
        let root = unique_temp_dir("lux-dap-adapter-aliases");
        std::fs::create_dir_all(&root).expect("test root should be created");
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"rust-app\"\n")
            .expect("cargo manifest should be written");
        std::fs::write(root.join("pyproject.toml"), "[project]\nname = \"api\"\n")
            .expect("pyproject should be written");

        let adapters = workspace_debug_adapters(&root).expect("adapters should load");
        let debugpy = adapters
            .iter()
            .find(|adapter| adapter.id == "debugpy")
            .expect("debugpy should be detected");

        assert_eq!(debugpy.transport, DebugAdapterTransport::Stdio);
        assert!(debugpy
            .configuration_types
            .iter()
            .any(|adapter_type| adapter_type == "python"));
        let codelldb = adapters
            .iter()
            .find(|adapter| adapter.id == "codelldb")
            .expect("codelldb should be detected");
        assert_eq!(codelldb.transport, DebugAdapterTransport::TcpServer);
        assert_eq!(tcp_server_port_arg_indices(&codelldb.args), Some((0, 1)));

        let configuration = DebugConfiguration {
            name: "Run API".to_string(),
            adapter_type: "python".to_string(),
            request: DebugConfigurationRequest::Launch,
            raw: serde_json::Map::new(),
        };

        assert!(adapter_matches_configuration(debugpy, &configuration));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn tcp_server_port_zero_is_allocated_before_launch() {
        let adapter = DebugAdapterInfo {
            id: "codelldb".to_string(),
            name: "CodeLLDB".to_string(),
            command: "codelldb".to_string(),
            args: vec!["--port".to_string(), "0".to_string()],
            configuration_types: vec!["codelldb".to_string()],
            transport: DebugAdapterTransport::TcpServer,
            workspace_root: PathBuf::from("."),
            status: DebugAdapterStatus::Available,
            error: None,
        };
        let (args, port) = tcp_server_args_and_port(&adapter)
            .await
            .expect("tcp adapter args should receive a concrete port");

        assert_ne!(port, 0);
        assert_eq!(args, vec!["--port".to_string(), port.to_string()]);
    }

    #[tokio::test]
    async fn exited_adapter_process_marks_session_terminal_without_dap_event() {
        let mut child = spawn_exiting_child(7);
        let stdin = child.stdin.take().expect("test child stdin should exist");
        let (_message_tx, messages) = mpsc::unbounded_channel();
        let mut session = DebugSession {
            info: debug_session_info(DebugSessionStatus::Running),
            writer: DapWriter::Stdio(stdin),
            child,
            read_task: tokio::spawn(async {}),
            stderr_task: tokio::spawn(async {}),
            messages,
            next_seq: 1,
            disconnect_sent: false,
            configuration_done_sent: true,
            breakpoints_by_path: BTreeMap::new(),
            resolved_breakpoints_by_path: BTreeMap::new(),
            pending_breakpoint_requests: BTreeMap::new(),
            pending_responses: BTreeMap::new(),
            threads: BTreeMap::new(),
        };

        let mut changed = mark_session_if_adapter_exited(&mut session);
        for _ in 0..50 {
            if changed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
            changed = mark_session_if_adapter_exited(&mut session);
        }

        assert!(changed);
        assert_eq!(session.info.status, DebugSessionStatus::Error);
        assert!(session.info.stopped_at.is_some());
        assert!(session
            .info
            .last_event
            .as_deref()
            .is_some_and(|event| event.starts_with("adapter process exited:")));
        assert!(session
            .info
            .error
            .as_deref()
            .is_some_and(|error| error.contains("debug adapter process exited")));
    }

    fn debug_session_info(status: DebugSessionStatus) -> DebugSessionInfo {
        DebugSessionInfo {
            id: Uuid::new_v4(),
            configuration_name: "Run tests".to_string(),
            adapter_id: "debugpy".to_string(),
            adapter_name: "Python Debugpy".to_string(),
            workspace_root: PathBuf::from("."),
            status,
            started_at: Utc::now(),
            stopped_at: None,
            active_thread_id: None,
            last_event: None,
            error: None,
        }
    }

    fn spawn_exiting_child(code: i32) -> Child {
        let mut command = exiting_command(code);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.spawn().expect("test child should spawn")
    }

    #[cfg(windows)]
    fn exiting_command(code: i32) -> Command {
        let mut command = Command::new("cmd");
        command.args(["/C", "exit", "/B"]).arg(code.to_string());
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }

    #[cfg(not(windows))]
    fn exiting_command(code: i32) -> Command {
        let mut command = Command::new("sh");
        command.arg("-c").arg(format!("exit {code}"));
        command
    }

    #[cfg(windows)]
    fn exit_status(code: u32) -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;

        ExitStatus::from_raw(code)
    }

    #[cfg(unix)]
    fn exit_status(code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;

        ExitStatus::from_raw(code << 8)
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nonce}"))
    }
}
