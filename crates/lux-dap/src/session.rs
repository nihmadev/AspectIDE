//! Debug session lifecycle: process/TCP transport, the `DebugSessionManager`
//! state machine, breakpoint state, and the stdout/stderr reader tasks. Drives
//! the protocol layer ([`crate::protocol`]) over a live adapter connection.
#![allow(clippy::module_name_repetitions)]

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    time::Duration,
};

use chrono::Utc;
use lux_core::{
    AppError, AppResult, DebugAdapterInfo, DebugAdapterStatus, DebugAdapterTransport,
    DebugBreakpointsUpdate, DebugConfiguration, DebugConfigurationRequest, DebugEvaluateContext,
    DebugEvaluateResult, DebugExecutionAction, DebugFrameScopes, DebugResolvedBreakpoint,
    DebugSessionInfo, DebugSessionStatus, DebugSourceBreakpoint, DebugStackTrace, DebugThreadInfo,
    DebugVariables,
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

use crate::protocol::{
    attach_request, configuration_done_request, disconnect_request, drain_dap_frames,
    encode_dap_message, evaluate_request, execution_action_command, execution_request,
    initialize_request, launch_request, non_empty_text, parse_breakpoints_response,
    parse_dap_message, parse_evaluate_response, parse_scopes_response, parse_stack_trace_response,
    parse_thread_info, parse_threads_response, parse_variables_response, scopes_request,
    set_breakpoints_request, stack_trace_request, threads_request, variables_request, DapEvent,
    DapMessage, DapRequest, DapResponse,
};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const TCP_CONNECT_ATTEMPTS: u32 = 40;
const TCP_CONNECT_DELAY: Duration = Duration::from_millis(50);
const DISCONNECT_GRACE_TIMEOUT: Duration = Duration::from_secs(2);
const DISCONNECT_POLL_DELAY: Duration = Duration::from_millis(25);

// ── Request-class timeout durations ──────────────────────────────────────
const TIMEOUT_METADATA: Duration = Duration::from_secs(8);
// Launch/attach can legitimately take a minute (interpreter startup, remote
// attach, container boot); the literal seconds value is intentional.
#[allow(clippy::duration_suboptimal_units)]
const TIMEOUT_LAUNCH: Duration = Duration::from_secs(60);
const TIMEOUT_BREAKPOINTS: Duration = Duration::from_secs(8);
const TIMEOUT_EXECUTION: Duration = Duration::from_secs(15);

/// Summarises the `initialize` response capabilities that control
/// IDE-side behaviour during the handshake and session lifetime.
#[derive(Debug, Clone, Default)]
struct Capabilities {
    supports_configuration_done_request: bool,
}

/// Per-thread running/stopped state used for fine-grained
/// `continued` event handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThreadState {
    Running,
    Paused,
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
    /// Terminal sessions whose final state has already been observed during a
    /// drain cycle. They are pruned from `sessions` on the following cycle so
    /// the map cannot grow without bound, while still giving the frontend one
    /// full cycle to read the terminal `Stopped`/`Error` state.
    reaped_sessions: BTreeSet<Uuid>,
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
    /// Stored capabilities from the `initialize` response.
    capabilities: Capabilities,
    /// Sequence number of the outgoing `configurationDone` request (if sent).
    configuration_done_seq: Option<u64>,
    breakpoints_by_path: BTreeMap<PathBuf, Vec<DebugSourceBreakpoint>>,
    resolved_breakpoints_by_path: BTreeMap<PathBuf, Vec<DebugResolvedBreakpoint>>,
    pending_breakpoint_requests: BTreeMap<u64, PathBuf>,
    pending_responses: BTreeMap<u64, DapResponse>,
    threads: BTreeMap<u64, DebugThreadInfo>,
    /// Per-thread running/paused state for fine-grained continued events.
    thread_states: BTreeMap<u64, ThreadState>,
    /// Whether the session was running before the last execute command.
    /// Used for guarded state transitions in `execute`.
    _pre_exec_was_paused: bool,
}

impl Drop for DebugSession {
    /// Guarantees the adapter process (and any debuggee it controls) cannot
    /// outlive the session — when the IDE quits, the session map is dropped and
    /// every adapter receives a kill signal here. `kill_on_drop(true)` on the
    /// spawned [`Command`] provides the same guarantee, but doing it explicitly
    /// keeps teardown deterministic and also stops the stdout/stderr reader
    /// tasks that would otherwise linger until their streams close on their own.
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.start_kill();
        }
        self.read_task.abort();
        self.stderr_task.abort();
    }
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
            reaped_sessions: BTreeSet::new(),
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
            .start_handshake(session_id, &adapter.id, &configuration, &workspace_root)
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
            .stderr(Stdio::piped())
            .kill_on_drop(true);
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
        let (args, port, port_reservation) = tcp_server_args_and_port(adapter).await?;
        let mut command = Command::new(&adapter.command);
        command
            .args(&args)
            .current_dir(workspace_root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);

        // Release the reserved loopback port (if any) at the last possible
        // instant so the adapter can bind it with the smallest possible window
        // for another process to steal it.
        drop(port_reservation);
        let mut child = command.spawn()?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::Service("debug adapter stderr is unavailable".into()))?;
        let stderr_task = tokio::spawn(drain_debug_stderr(
            stderr,
            session_id,
            self.update_tx.clone(),
        ));
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
                capabilities: Capabilities::default(),
                configuration_done_seq: None,
                breakpoints_by_path,
                resolved_breakpoints_by_path: BTreeMap::new(),
                pending_breakpoint_requests: BTreeMap::new(),
                pending_responses: BTreeMap::new(),
                threads: BTreeMap::new(),
                thread_states: BTreeMap::new(),
                _pre_exec_was_paused: false,
            },
        );
    }

    async fn start_handshake(
        &mut self,
        session_id: Uuid,
        adapter_id: &str,
        configuration: &DebugConfiguration,
        workspace_root: &Path,
    ) -> AppResult<()> {
        let initialize_seq = self.next_request_seq(session_id)?;
        self.send_request(session_id, initialize_request(initialize_seq, adapter_id))
            .await?;
        let resp = self
            .wait_for_response_body(session_id, initialize_seq, "initialize")
            .await?;
        // Store capabilities from the initialize response.
        {
            let caps = resp.as_ref();
            self.with_session_mut(session_id, |session| {
                session.capabilities = Capabilities {
                    supports_configuration_done_request: caps
                        .and_then(|v| v.get("supportsConfigurationDoneRequest"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                };
            })?;
        }
        let configuration_seq = self.next_request_seq(session_id)?;
        self.send_request(
            session_id,
            match configuration.request {
                DebugConfigurationRequest::Launch => {
                    launch_request(configuration_seq, configuration, workspace_root)
                }
                DebugConfigurationRequest::Attach => {
                    attach_request(configuration_seq, configuration, workspace_root)
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
        // Guarded state transition: only set Running if the session is still
        // in the expected pre-execution (Paused) state and no paused/terminal
        // event was observed during the wait.
        self.wait_for_response_guarded(session_id, execute_seq, command)
            .await?;
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
        self.prune_terminal_sessions();
    }

    /// Bounds the session map: a session that has been terminal for a full drain
    /// cycle is removed. The one-cycle delay guarantees the frontend has already
    /// received the terminal `Changed` event before the entry disappears, and
    /// dropping the [`DebugSession`] runs its `Drop` (killing any lingering
    /// adapter and aborting its reader tasks).
    fn prune_terminal_sessions(&mut self) {
        let terminal = self
            .sessions
            .iter()
            .filter_map(|(id, session)| is_terminal_status(session.info.status).then_some(*id))
            .collect::<Vec<_>>();

        for session_id in terminal {
            if self.reaped_sessions.insert(session_id) {
                continue;
            }
            self.sessions.remove(&session_id);
            self.reaped_sessions.remove(&session_id);
        }

        self.reaped_sessions
            .retain(|session_id| self.sessions.contains_key(session_id));
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
        let timeout = match command {
            "launch" | "attach" => TIMEOUT_LAUNCH,
            "continue" | "next" | "stepIn" | "stepOut" => TIMEOUT_EXECUTION,
            // "initialize" and all other metadata requests share this timeout.
            _ => TIMEOUT_METADATA,
        };
        tokio::time::timeout(timeout, async {
            loop {
                if let Some(response) = self.take_pending_response(session_id, request_seq)? {
                    return self.apply_expected_response(session_id, response, command);
                }
                // Drain everything already delivered before consulting child
                // liveness: an adapter that answered and then exited must still
                // have its queued answer honored. Only when the queue is empty
                // does an exited child mean the response will never arrive.
                let queued = self.try_recv_message(session_id)?;
                let message = if let Some(message) = queued {
                    message
                } else {
                    if let Some(session) = self.sessions.get_mut(&session_id) {
                        if let Ok(Some(_)) = session.child.try_wait() {
                            return Err(AppError::Service(format!(
                                "debug adapter exited before responding to {command}"
                            )));
                        }
                    }
                    self.recv_message(session_id).await?
                };
                match message {
                    DapMessage::Response(response) if response.request_seq == request_seq => {
                        return self.apply_expected_response(session_id, response, command);
                    }
                    // A response for a different request that arrives first is
                    // parked by `request_seq` rather than dropped, so the
                    // pending waiter for that request can still claim it instead
                    // of timing out on an adapter that answers out of order.
                    DapMessage::Response(response) => {
                        self.store_pending_response(session_id, response)?;
                    }
                    other => self.apply_message(session_id, other).await?,
                }
            }
        })
        .await
        .map_err(|_| {
            AppError::Service(format!(
                "debug adapter did not respond to {command} within {timeout:?}"
            ))
        })?
    }

    /// Like `wait_for_response`, but only transitions to `Running` if the session
    /// is still `Paused` after the response. Preserves `Paused`/`Stopped`/`Error`.
    async fn wait_for_response_guarded(
        &mut self,
        session_id: Uuid,
        request_seq: u64,
        command: &str,
    ) -> AppResult<()> {
        let _body = self
            .wait_for_response_body(session_id, request_seq, command)
            .await?;
        self.with_session_mut(session_id, |session| {
            if session.info.status == DebugSessionStatus::Paused {
                session.info.status = DebugSessionStatus::Running;
                session.info.last_event = Some(format!("{command} requested"));
            }
        })?;
        Ok(())
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

    /// Non-blocking receive: returns a message already sitting in the queue, or
    /// `None` when the queue is currently empty (including after stream close —
    /// the blocking `recv_message` path reports the close as an error).
    fn try_recv_message(&mut self, session_id: Uuid) -> AppResult<Option<DapMessage>> {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        Ok(session.messages.try_recv().ok())
    }

    async fn apply_message(&mut self, session_id: Uuid, message: DapMessage) -> AppResult<()> {
        match message {
            DapMessage::Event(event) => self.apply_event(session_id, event).await?,
            DapMessage::Response(response) => self.apply_response(session_id, response)?,
            DapMessage::Request(request) => self.apply_reverse_request(session_id, request).await?,
        }
        self.emit_session(session_id)
    }

    /// Handle a reverse request from the adapter (type: "request").
    /// We must respond with a response message keyed by the request `seq`,
    /// otherwise the adapter may hang waiting for a reply.
    async fn apply_reverse_request(
        &mut self,
        session_id: Uuid,
        request: DapRequest,
    ) -> AppResult<()> {
        self.with_session_mut(session_id, |session| {
            session.info.last_event = Some(format!("adapter request: {}", request.command));
        })?;
        let response = if request.command == "runInTerminal" {
            // Best-effort: acknowledge with success:true so the adapter does not
            // hang. A real terminal integration is deferred.
            json!({
                "type": "response",
                "request_seq": request.seq,
                "success": true,
                "command": "runInTerminal",
                "body": {}
            })
        } else {
            // Send success:false so the adapter does not hang on unsupported
            // reverse requests.
            json!({
                "type": "response",
                "request_seq": request.seq,
                "success": false,
                "command": request.command,
                "message": "unsupported reverse request"
            })
        };
        let encoded = encode_dap_message(&response)?;
        self.send_raw(session_id, &encoded).await
    }

    async fn send_raw(&mut self, session_id: Uuid, encoded: &[u8]) -> AppResult<()> {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        session.writer.write_all(encoded).await
    }

    #[allow(clippy::too_many_lines)]
    async fn apply_event(&mut self, session_id: Uuid, event: DapEvent) -> AppResult<()> {
        // Handle output events first — they should not overwrite lifecycle state.
        if event.event.as_str() == "output" {
            let text = event
                .body
                .as_ref()
                .and_then(|b| b.get("output"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if !text.is_empty() {
                self.update_tx
                    .send(DebugSessionUpdate::Output {
                        session_id,
                        text: text.to_string(),
                    })
                    .ok();
            }
            return Ok(());
        }

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
                    if let Some(thread_id) = stopped_event_thread_id(event.body.as_ref()) {
                        session.info.active_thread_id = Some(thread_id);
                        session.thread_states.insert(thread_id, ThreadState::Paused);
                    }
                    None
                }
                "continued" => {
                    // Fine-grained thread state: honor body.threadId and
                    // body.allThreadsContinued.
                    let thread_id = stopped_event_thread_id(event.body.as_ref());
                    let all_continued = event
                        .body
                        .as_ref()
                        .and_then(|b| b.get("allThreadsContinued"))
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    if all_continued || thread_id.is_none() {
                        session.thread_states.clear();
                        session.info.status = DebugSessionStatus::Running;
                    } else if let Some(tid) = thread_id {
                        session.thread_states.insert(tid, ThreadState::Running);
                        // Keep session Paused if any thread remains stopped.
                        if session
                            .thread_states
                            .values()
                            .any(|s| *s == ThreadState::Paused)
                        {
                            // Stay paused.
                        } else {
                            session.info.status = DebugSessionStatus::Running;
                        }
                    }
                    None
                }
                "thread" => {
                    // Handle thread events to update/remove cached threads.
                    if let Some(body) = event.body.as_ref() {
                        if let Some(reason) = body.get("reason").and_then(Value::as_str) {
                            match reason {
                                "started" => {
                                    if let Some(thread) = parse_thread_info(body) {
                                        session.threads.insert(thread.id, thread);
                                    }
                                }
                                "exited" => {
                                    if let Some(thread_id) =
                                        body.get("threadId").and_then(Value::as_u64)
                                    {
                                        session.threads.remove(&thread_id);
                                        session.thread_states.remove(&thread_id);
                                        // Invalidate active_thread_id if the
                                        // active thread exited.
                                        if session.info.active_thread_id == Some(thread_id) {
                                            session.info.active_thread_id = None;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
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
        if !self.configuration_done_sent(session_id)? {
            tokio::time::timeout(TIMEOUT_METADATA, async {
                loop {
                    let message = self.recv_message(session_id).await?;
                    self.apply_message(session_id, message).await?;
                    if self.configuration_done_sent(session_id)? {
                        return Ok::<(), AppError>(());
                    }
                }
            })
            .await
            .map_err(|_| {
                AppError::Service(
                    "debug adapter did not emit initialized before configurationDone within 8 \
                     seconds"
                        .into(),
                )
            })??;
        }

        // If a `configurationDone` request was actually sent (the adapter
        // advertised `supportsConfigurationDoneRequest`), wait for and validate
        // its response so a rejected request fails startup instead of being
        // parked as an unrelated pending response while the session is marked
        // started. Adapters that do not support the request leave the seq unset
        // and skip the wait.
        if let Some(seq) = self.configuration_done_seq(session_id)? {
            self.wait_for_response(session_id, seq, "configurationDone")
                .await?;
        }
        Ok(())
    }

    fn configuration_done_seq(&self, session_id: Uuid) -> AppResult<Option<u64>> {
        self.sessions
            .get(&session_id)
            .map(|session| session.configuration_done_seq)
            .ok_or_else(|| AppError::NotFound(format!("debug session {session_id}")))
    }

    fn apply_response(&mut self, session_id: Uuid, response: DapResponse) -> AppResult<()> {
        let breakpoint_update = self.with_session_mut(session_id, |session| {
            session.info.last_event = Some(format!("{} response", response.command));
            if response.command == "setBreakpoints" {
                return apply_breakpoints_response(session_id, session, response);
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
        tokio::time::timeout(TIMEOUT_BREAKPOINTS, async {
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
                    DapMessage::Request(request) => {
                        self.with_session_mut(session_id, |session| {
                            session.info.last_event =
                                Some(format!("adapter request: {}", request.command));
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
                    if let Some(thread_id) = stopped_event_thread_id(event.body.as_ref()) {
                        session.info.active_thread_id = Some(thread_id);
                        session.thread_states.insert(thread_id, ThreadState::Paused);
                    }
                }
                "continued" => {
                    let thread_id = stopped_event_thread_id(event.body.as_ref());
                    let all_continued = event
                        .body
                        .as_ref()
                        .and_then(|b| b.get("allThreadsContinued"))
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    if all_continued || thread_id.is_none() {
                        session.thread_states.clear();
                        session.info.status = DebugSessionStatus::Running;
                    } else if let Some(tid) = thread_id {
                        session.thread_states.insert(tid, ThreadState::Running);
                        if !session
                            .thread_states
                            .values()
                            .any(|s| *s == ThreadState::Paused)
                        {
                            session.info.status = DebugSessionStatus::Running;
                        }
                    }
                }
                "thread" => {
                    if let Some(body) = event.body.as_ref() {
                        if let Some(reason) = body.get("reason").and_then(Value::as_str) {
                            match reason {
                                "started" => {
                                    if let Some(thread) = parse_thread_info(body) {
                                        session.threads.insert(thread.id, thread);
                                    }
                                }
                                "exited" => {
                                    if let Some(thread_id) =
                                        body.get("threadId").and_then(Value::as_u64)
                                    {
                                        session.threads.remove(&thread_id);
                                        session.thread_states.remove(&thread_id);
                                        if session.info.active_thread_id == Some(thread_id) {
                                            session.info.active_thread_id = None;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
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
        let (seq, should_send) = self.with_session_mut(session_id, |session| {
            if session.configuration_done_sent {
                (0, false)
            } else if !session.capabilities.supports_configuration_done_request {
                // Adapter does not support it — mark as done and skip.
                session.configuration_done_sent = true;
                (0, false)
            } else {
                let seq = session.next_seq;
                session.next_seq += 1;
                session.configuration_done_sent = true;
                session.configuration_done_seq = Some(seq);
                (seq, true)
            }
        })?;
        if should_send {
            self.send_request(session_id, configuration_done_request(seq))
                .await?;
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

async fn tcp_server_args_and_port(
    adapter: &DebugAdapterInfo,
) -> AppResult<(Vec<String>, u16, Option<TcpListener>)> {
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
    // For an auto-allocated port (`0`) we keep the OS-assigned listener bound and
    // hand it back so the caller can release it immediately before spawning the
    // adapter, shrinking the bind/hand-off TOCTOU window to a single syscall.
    let (port, reservation) = if configured_port == 0 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        (port, Some(listener))
    } else {
        (configured_port, None)
    };
    args[value_index] = port.to_string();
    Ok((args, port, reservation))
}

pub fn tcp_server_port_arg_indices(args: &[String]) -> Option<(usize, usize)> {
    args.windows(2).enumerate().find_map(|(index, window)| {
        matches!(window, [flag, _] if flag == "--port" || flag == "-p")
            .then_some((index, index + 1))
    })
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

fn stopped_event_thread_id(body: Option<&Value>) -> Option<u64> {
    body.and_then(|value| value.get("threadId"))
        .and_then(Value::as_u64)
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
                // A framing violation (bad/oversized Content-Length or an
                // un-terminated header) means the byte stream is no longer
                // trustworthy. Stop reading instead of looping — continuing
                // would let a hostile peer keep replaying the violation and grow
                // memory again. `buffer` is released as the loop unwinds.
                let _ = update_tx.send(DebugSessionUpdate::Output {
                    session_id,
                    text: format!("debug adapter emitted invalid DAP frame: {error}\n"),
                });
                break;
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let (args, port, reservation) = tcp_server_args_and_port(&adapter)
            .await
            .expect("tcp adapter args should receive a concrete port");

        assert_ne!(port, 0);
        assert_eq!(args, vec!["--port".to_string(), port.to_string()]);
        // An auto-allocated port is held until the adapter is launched, keeping
        // the bind/hand-off window minimal.
        assert!(reservation.is_some());
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
            capabilities: Capabilities::default(),
            configuration_done_seq: None,
            breakpoints_by_path: BTreeMap::new(),
            resolved_breakpoints_by_path: BTreeMap::new(),
            pending_breakpoint_requests: BTreeMap::new(),
            pending_responses: BTreeMap::new(),
            threads: BTreeMap::new(),
            thread_states: BTreeMap::new(),
            _pre_exec_was_paused: false,
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

    #[tokio::test]
    async fn wait_for_response_body_claims_out_of_order_responses() {
        let (update_tx, _update_rx) = mpsc::unbounded_channel();
        let mut manager = DebugSessionManager::new(update_tx);

        let mut child = spawn_exiting_child(0);
        let stdin = child.stdin.take().expect("test child stdin should exist");
        let (message_tx, messages) = mpsc::unbounded_channel();
        let info = debug_session_info(DebugSessionStatus::Running);
        let session_id = info.id;
        manager.sessions.insert(
            session_id,
            DebugSession {
                info,
                writer: DapWriter::Stdio(stdin),
                child,
                read_task: tokio::spawn(async {}),
                stderr_task: tokio::spawn(async {}),
                messages,
                next_seq: 1,
                disconnect_sent: false,
                configuration_done_sent: true,
                capabilities: Capabilities::default(),
                configuration_done_seq: None,
                breakpoints_by_path: BTreeMap::new(),
                resolved_breakpoints_by_path: BTreeMap::new(),
                pending_breakpoint_requests: BTreeMap::new(),
                pending_responses: BTreeMap::new(),
                threads: BTreeMap::new(),
                thread_states: BTreeMap::new(),
                _pre_exec_was_paused: false,
            },
        );

        // The adapter answers request 2 before request 1. The waiter for request
        // 1 must park the seq-2 response instead of dropping it.
        message_tx
            .send(DapMessage::Response(DapResponse {
                request_seq: 2,
                success: true,
                command: "evaluate".to_string(),
                message: None,
                body: Some(json!({"result": "second"})),
            }))
            .expect("seq 2 response should enqueue");
        message_tx
            .send(DapMessage::Response(DapResponse {
                request_seq: 1,
                success: true,
                command: "evaluate".to_string(),
                message: None,
                body: Some(json!({"result": "first"})),
            }))
            .expect("seq 1 response should enqueue");

        let first = manager
            .wait_for_response_body(session_id, 1, "evaluate")
            .await
            .expect("seq 1 response should resolve");
        assert_eq!(first, Some(json!({"result": "first"})));

        let second = manager
            .wait_for_response_body(session_id, 2, "evaluate")
            .await
            .expect("parked seq 2 response should still resolve");
        assert_eq!(second, Some(json!({"result": "second"})));
    }

    #[tokio::test]
    async fn wait_for_configuration_done_fails_when_adapter_rejects_request() {
        let (update_tx, _update_rx) = mpsc::unbounded_channel();
        let mut manager = DebugSessionManager::new(update_tx);
        let (message_tx, session_id) =
            insert_configured_session(&mut manager, /* configuration_done_seq */ Some(7));

        // The adapter rejects the configurationDone request keyed by seq 7.
        message_tx
            .send(DapMessage::Response(DapResponse {
                request_seq: 7,
                success: false,
                command: "configurationDone".to_string(),
                message: Some("not ready".to_string()),
                body: None,
            }))
            .expect("configurationDone rejection should enqueue");

        let result = manager.wait_for_configuration_done(session_id).await;
        let error = result.expect_err("rejected configurationDone must fail startup");
        assert!(
            error.to_string().contains("not ready"),
            "error should surface adapter rejection message: {error}"
        );
    }

    #[tokio::test]
    async fn wait_for_configuration_done_skips_wait_when_unsupported() {
        let (update_tx, _update_rx) = mpsc::unbounded_channel();
        let mut manager = DebugSessionManager::new(update_tx);
        // No configuration_done_seq means the adapter did not advertise support,
        // so no response is expected and the wait resolves immediately.
        let (_message_tx, session_id) =
            insert_configured_session(&mut manager, /* configuration_done_seq */ None);

        manager
            .wait_for_configuration_done(session_id)
            .await
            .expect("unsupported configurationDone should resolve without a response");
    }

    /// Insert a session that has already passed the `initialized` handshake
    /// (`configuration_done_sent == true`) so `wait_for_configuration_done`
    /// proceeds straight to the response-validation step.
    fn insert_configured_session(
        manager: &mut DebugSessionManager,
        configuration_done_seq: Option<u64>,
    ) -> (mpsc::UnboundedSender<DapMessage>, Uuid) {
        let mut child = spawn_sleeping_child();
        let stdin = child.stdin.take().expect("test child stdin should exist");
        let (message_tx, messages) = mpsc::unbounded_channel();
        let info = debug_session_info(DebugSessionStatus::Running);
        let session_id = info.id;
        manager.sessions.insert(
            session_id,
            DebugSession {
                info,
                writer: DapWriter::Stdio(stdin),
                child,
                read_task: tokio::spawn(async {}),
                stderr_task: tokio::spawn(async {}),
                messages,
                next_seq: 8,
                disconnect_sent: false,
                configuration_done_sent: true,
                capabilities: Capabilities {
                    supports_configuration_done_request: configuration_done_seq.is_some(),
                },
                configuration_done_seq,
                breakpoints_by_path: BTreeMap::new(),
                resolved_breakpoints_by_path: BTreeMap::new(),
                pending_breakpoint_requests: BTreeMap::new(),
                pending_responses: BTreeMap::new(),
                threads: BTreeMap::new(),
                thread_states: BTreeMap::new(),
                _pre_exec_was_paused: false,
            },
        );
        (message_tx, session_id)
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
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command.spawn().expect("test child should spawn")
    }

    /// Spawn a child that stays alive (killed on drop) so adapter-exit detection
    /// in `wait_for_response_body` does not race with response delivery.
    fn spawn_sleeping_child() -> Child {
        let mut command = sleeping_command();
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command.spawn().expect("test child should spawn")
    }

    #[cfg(windows)]
    fn sleeping_command() -> Command {
        let mut command = Command::new("cmd");
        // `pause` waits for input that never arrives; the child is killed on drop.
        command.args(["/C", "pause"]);
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }

    #[cfg(not(windows))]
    fn sleeping_command() -> Command {
        let mut command = Command::new("sh");
        command.arg("-c").arg("sleep 30");
        command
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
}
