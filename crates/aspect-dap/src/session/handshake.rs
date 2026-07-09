use std::path::{Path, PathBuf};

use aspect_core::{
    AppError, AppResult, DebugAdapterInfo, DebugConfiguration, DebugConfigurationRequest,
    DebugSessionInfo, DebugSourceBreakpoint,
};
use serde_json::Value;
use uuid::Uuid;

use crate::protocol::{attach_request, launch_request};

use super::{validate_start_adapter, DebugSessionManager};

impl DebugSessionManager {
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
        self.emit_session(session_id)?;
        self.session_info(session_id)
    }

    async fn start_handshake(
        &mut self,
        session_id: Uuid,
        adapter_id: &str,
        configuration: &DebugConfiguration,
        workspace_root: &Path,
    ) -> AppResult<()> {
        let initialize_seq = self.next_request_seq(session_id)?;
        self.send_request(
            session_id,
            crate::protocol::initialize_request(initialize_seq, adapter_id),
        )
        .await?;
        let resp = self
            .wait_for_response_body(session_id, initialize_seq, "initialize")
            .await?;
        {
            let caps = resp.as_ref();
            self.with_session_mut(session_id, |session| {
                session.capabilities = super::Capabilities {
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

    async fn wait_for_configuration_done(&mut self, session_id: Uuid) -> AppResult<()> {
        if !self.configuration_done_sent(session_id)? {
            tokio::time::timeout(super::TIMEOUT_METADATA, async {
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

    pub(super) async fn send_configuration_done(&mut self, session_id: Uuid) -> AppResult<()> {
        let (seq, should_send) = self.with_session_mut(session_id, |session| {
            if session.configuration_done_sent {
                (0, false)
            } else if !session.capabilities.supports_configuration_done_request {
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
            self.send_request(session_id, crate::protocol::configuration_done_request(seq))
                .await?;
        }
        Ok(())
    }

    pub(super) fn configuration_done_sent(&self, session_id: Uuid) -> AppResult<bool> {
        self.sessions
            .get(&session_id)
            .map(|session| session.configuration_done_sent)
            .ok_or_else(|| AppError::NotFound(format!("debug session {session_id}")))
    }

    fn mark_started(
        &mut self,
        session_id: Uuid,
        request: DebugConfigurationRequest,
    ) -> AppResult<()> {
        self.with_session_mut(session_id, |session| {
            super::mark_session_started(&mut session.info, request);
        })
    }

    async fn fail_start(&mut self, session_id: Uuid, error: AppError) -> AppError {
        let message = error.to_string();
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.info.status = aspect_core::DebugSessionStatus::Error;
            session.info.stopped_at.get_or_insert_with(chrono::Utc::now);
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
}
