use std::path::Path;

use aspect_core::{AppError, AppResult};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::protocol::{
    encode_dap_message, set_breakpoints_request, DapMessage, DapRequest, DapResponse,
};

use super::{
    is_terminal_status, mark_session_if_adapter_exited, DebugSessionManager,
};

impl DebugSessionManager {
    pub(super) async fn recv_message(&mut self, session_id: Uuid) -> AppResult<DapMessage> {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        session
            .messages
            .recv()
            .await
            .ok_or_else(|| AppError::Service("debug adapter message stream closed".into()))
    }

    pub(super) fn try_recv_message(&mut self, session_id: Uuid) -> AppResult<Option<DapMessage>> {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        Ok(session.messages.try_recv().ok())
    }

    pub(super) async fn drain_messages(&mut self) {
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

    pub(super) async fn wait_for_response(
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

    pub(super) async fn wait_for_response_body(
        &mut self,
        session_id: Uuid,
        request_seq: u64,
        command: &str,
    ) -> AppResult<Option<Value>> {
        let timeout = match command {
            "launch" | "attach" => super::TIMEOUT_LAUNCH,
            "continue" | "next" | "stepIn" | "stepOut" => super::TIMEOUT_EXECUTION,
            _ => super::TIMEOUT_METADATA,
        };
        tokio::time::timeout(timeout, async {
            loop {
                if let Some(response) = self.take_pending_response(session_id, request_seq)? {
                    return self.apply_expected_response(session_id, response, command);
                }
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

    pub(super) async fn wait_for_response_guarded(
        &mut self,
        session_id: Uuid,
        request_seq: u64,
        command: &str,
    ) -> AppResult<()> {
        let _body = self
            .wait_for_response_body(session_id, request_seq, command)
            .await?;
        self.with_session_mut(session_id, |session| {
            if session.info.status == aspect_core::DebugSessionStatus::Paused {
                session.info.status = aspect_core::DebugSessionStatus::Running;
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

    pub(super) async fn apply_message(
        &mut self,
        session_id: Uuid,
        message: DapMessage,
    ) -> AppResult<()> {
        match message {
            DapMessage::Event(event) => self.apply_event(session_id, event).await?,
            DapMessage::Response(response) => self.apply_response(session_id, response)?,
            DapMessage::Request(request) => {
                self.apply_reverse_request(session_id, request).await?;
            }
        }
        self.emit_session(session_id)
    }

    async fn apply_reverse_request(
        &mut self,
        session_id: Uuid,
        request: DapRequest,
    ) -> AppResult<()> {
        self.with_session_mut(session_id, |session| {
            session.info.last_event = Some(format!("adapter request: {}", request.command));
        })?;
        let response = if request.command == "runInTerminal" {
            json!({
                "type": "response",
                "request_seq": request.seq,
                "success": true,
                "command": "runInTerminal",
                "body": {}
            })
        } else {
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

    pub(super) async fn send_raw(
        &mut self,
        session_id: Uuid,
        encoded: &[u8],
    ) -> AppResult<()> {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        session.writer.write_all(encoded).await
    }

    pub(super) async fn send_request(
        &mut self,
        session_id: Uuid,
        request: Value,
    ) -> AppResult<()> {
        let encoded = encode_dap_message(&request)?;
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        session.writer.write_all(&encoded).await
    }

    pub(super) async fn send_breakpoints_for_path(
        &mut self,
        session_id: Uuid,
        path: &Path,
    ) -> AppResult<()> {
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
        self.send_request(
            session_id,
            set_breakpoints_request(seq, path, &breakpoints),
        )
        .await?;
        self.wait_for_breakpoints_response(session_id, seq).await?;
        Ok(())
    }

    async fn wait_for_breakpoints_response(
        &mut self,
        session_id: Uuid,
        request_seq: u64,
    ) -> AppResult<()> {
        tokio::time::timeout(super::TIMEOUT_BREAKPOINTS, async {
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
}
