//! Debug session lifecycle: the `DebugSessionManager` state machine, process/TCP transport,
//! breakpoint state, and the stdout/stderr reader tasks. Drives the protocol layer
//! over a live adapter connection.
#![allow(clippy::module_name_repetitions)]

mod breakpoints;
mod commands;
mod events;
mod handshake;
mod internal;
mod io;
mod messages;
mod spawn;

// Pull free functions into the session module so sibling sub-modules can
// access them via `super::function_name` or `use super::function_name;`.
use breakpoints::{
    apply_breakpoints_response, group_source_breakpoints, normalize_breakpoint_path,
    sanitize_source_breakpoints, store_pending_response_by_seq, take_pending_response_by_seq,
    validate_breakpoint_session,
};
use io::{
    is_terminal_status, mark_session_if_adapter_exited, mark_session_started,
    stopped_event_thread_id,
};
use spawn::validate_start_adapter;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Duration,
};

use aspect_core::{
    DebugBreakpointsUpdate, DebugResolvedBreakpoint, DebugSessionInfo, DebugSourceBreakpoint,
};
use tokio::{
    io::{AsyncWriteExt, WriteHalf},
    net::TcpStream,
    process::{Child, ChildStdin},
    sync::mpsc,
    task::JoinHandle,
};
use uuid::Uuid;

use crate::protocol::{DapMessage, DapResponse};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const TCP_CONNECT_ATTEMPTS: u32 = 40;
const TCP_CONNECT_DELAY: Duration = Duration::from_millis(50);
const DISCONNECT_GRACE_TIMEOUT: Duration = Duration::from_secs(2);
const DISCONNECT_POLL_DELAY: Duration = Duration::from_millis(25);

const TIMEOUT_METADATA: Duration = Duration::from_secs(8);
#[allow(clippy::duration_suboptimal_units)]
const TIMEOUT_LAUNCH: Duration = Duration::from_secs(60);
const TIMEOUT_BREAKPOINTS: Duration = Duration::from_secs(8);
const TIMEOUT_EXECUTION: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Default)]
struct Capabilities {
    supports_configuration_done_request: bool,
}

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

enum DapWriter {
    Stdio(ChildStdin),
    Tcp(WriteHalf<TcpStream>),
}

impl DapWriter {
    async fn write_all(&mut self, encoded: &[u8]) -> aspect_core::AppResult<()> {
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

struct SpawnedDebugAdapter {
    writer: DapWriter,
    child: Child,
    read_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
    messages: mpsc::UnboundedReceiver<DapMessage>,
}

enum DebugLifecycleRequest {
    ConfigureBreakpoints(Vec<PathBuf>),
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
    capabilities: Capabilities,
    configuration_done_seq: Option<u64>,
    breakpoints_by_path: BTreeMap<PathBuf, Vec<DebugSourceBreakpoint>>,
    resolved_breakpoints_by_path: BTreeMap<PathBuf, Vec<DebugResolvedBreakpoint>>,
    pending_breakpoint_requests: BTreeMap<u64, PathBuf>,
    pending_responses: BTreeMap<u64, DapResponse>,
    threads: BTreeMap<u64, aspect_core::DebugThreadInfo>,
    thread_states: BTreeMap<u64, ThreadState>,
    _pre_exec_was_paused: bool,
}

impl Drop for DebugSession {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.start_kill();
        }
        self.read_task.abort();
        self.stderr_task.abort();
    }
}

pub struct DebugSessionManager {
    update_tx: mpsc::UnboundedSender<DebugSessionUpdate>,
    sessions: BTreeMap<Uuid, DebugSession>,
    reaped_sessions: BTreeSet<Uuid>,
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
}
