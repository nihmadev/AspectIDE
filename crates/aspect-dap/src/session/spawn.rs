use std::path::Path;

use aspect_core::{
    AppError, AppResult, DebugAdapterInfo, DebugAdapterStatus, DebugAdapterTransport,
    DebugConfiguration, DebugSessionInfo, DebugSessionStatus, DebugSourceBreakpoint,
};
use std::process::Stdio;

use tokio::{
    net::{TcpListener, TcpStream},
    process::Command,
    sync::mpsc,
};
use uuid::Uuid;

use super::{
    group_source_breakpoints, Capabilities, DapWriter, DebugSession, DebugSessionManager,
    SpawnedDebugAdapter, CREATE_NO_WINDOW, TCP_CONNECT_ATTEMPTS, TCP_CONNECT_DELAY,
};

impl DebugSessionManager {
    pub(super) async fn spawn_adapter_process(
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
        let read_task = tokio::spawn(super::io::read_dap_stdout(
            stdout,
            session_id,
            message_tx,
            self.update_tx.clone(),
        ));
        let stderr_task = tokio::spawn(super::io::drain_debug_stderr(
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

        drop(port_reservation);
        let mut child = command.spawn()?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::Service("debug adapter stderr is unavailable".into()))?;
        let stderr_task = tokio::spawn(super::io::drain_debug_stderr(
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
        let read_task = tokio::spawn(super::io::read_dap_stdout(
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

    pub(super) fn insert_starting_session(
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
            started_at: chrono::Utc::now(),
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
                resolved_breakpoints_by_path: std::collections::BTreeMap::new(),
                pending_breakpoint_requests: std::collections::BTreeMap::new(),
                pending_responses: std::collections::BTreeMap::new(),
                threads: std::collections::BTreeMap::new(),
                thread_states: std::collections::BTreeMap::new(),
                _pre_exec_was_paused: false,
            },
        );
    }
}

pub(super) fn validate_start_adapter(adapter: &DebugAdapterInfo) -> AppResult<()> {
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
