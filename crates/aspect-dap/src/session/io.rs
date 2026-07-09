use aspect_core::{DebugConfigurationRequest, DebugSessionInfo, DebugSessionStatus};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::protocol::{drain_dap_frames, parse_dap_message, DapMessage};

use super::{DebugSession, DebugSessionUpdate};

pub(super) async fn read_dap_stdout<R>(
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

pub(super) async fn drain_debug_stderr<R>(
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

pub(super) fn mark_session_if_adapter_exited(session: &mut DebugSession) -> bool {
    if matches!(
        session.info.status,
        DebugSessionStatus::Stopped | DebugSessionStatus::Error
    ) {
        return false;
    }

    match session.child.try_wait() {
        Ok(Some(status)) => {
            let expected_stop =
                matches!(session.info.status, DebugSessionStatus::Stopping);
            mark_session_adapter_exited(&mut session.info, status, expected_stop);
            session.read_task.abort();
            session.stderr_task.abort();
            true
        }
        Ok(None) => false,
        Err(error) => {
            session.info.status = DebugSessionStatus::Error;
            session.info.stopped_at.get_or_insert_with(chrono::Utc::now);
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
    status: std::process::ExitStatus,
    expected_stop: bool,
) {
    info.status = if expected_stop || status.success() {
        DebugSessionStatus::Stopped
    } else {
        DebugSessionStatus::Error
    };
    info.stopped_at.get_or_insert_with(chrono::Utc::now);
    info.last_event = Some(format!("adapter process exited: {status}"));
    if !expected_stop && !status.success() {
        info.error = Some(format!("debug adapter process exited with {status}"));
    }
}

pub(super) fn stopped_event_thread_id(body: Option<&serde_json::Value>) -> Option<u64> {
    body.and_then(|value| value.get("threadId"))
        .and_then(serde_json::Value::as_u64)
}

pub(super) const fn is_terminal_status(status: DebugSessionStatus) -> bool {
    matches!(
        status,
        DebugSessionStatus::Stopped | DebugSessionStatus::Error
    )
}

pub(super) fn mark_session_started(
    info: &mut DebugSessionInfo,
    request: DebugConfigurationRequest,
) {
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
