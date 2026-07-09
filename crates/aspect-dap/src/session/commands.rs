use std::path::PathBuf;

use chrono::Utc;

use aspect_core::{
    AppError, AppResult, DebugBreakpointsUpdate, DebugEvaluateContext, DebugEvaluateResult,
    DebugExecutionAction, DebugFrameScopes, DebugSessionInfo, DebugSessionStatus,
    DebugStackTrace, DebugThreadInfo, DebugVariables, DebugSourceBreakpoint,
};
use uuid::Uuid;

use crate::protocol::{
    disconnect_request, evaluate_request, execution_action_command, execution_request,
    non_empty_text, parse_evaluate_response, parse_scopes_response, parse_stack_trace_response,
    parse_threads_response, parse_variables_response, scopes_request, stack_trace_request,
    threads_request, variables_request,
};

use super::{
    is_terminal_status, normalize_breakpoint_path, sanitize_source_breakpoints,
    validate_breakpoint_session, DebugSessionManager,
};

impl DebugSessionManager {
    pub async fn sessions(&mut self) -> Vec<DebugSessionInfo> {
        self.drain_messages().await;
        self.sessions
            .values()
            .map(|session| session.info.clone())
            .collect()
    }

    pub async fn stack_trace(&mut self, session_id: Uuid) -> AppResult<DebugStackTrace> {
        self.drain_messages().await;
        self.ensure_paused(session_id, "stack trace")?;
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

    pub async fn scopes(
        &mut self,
        session_id: Uuid,
        frame_id: u64,
    ) -> AppResult<DebugFrameScopes> {
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
        self.ensure_paused(session_id, "execution control")?;
        let thread = self.resolve_stack_thread(session_id).await?;
        let command = execution_action_command(action);
        let execute_seq = self.next_request_seq(session_id)?;
        self.send_request(
            session_id,
            execution_request(execute_seq, action, thread.id),
        )
        .await?;
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

    async fn wait_for_disconnect_terminal_state(
        &mut self,
        session_id: Uuid,
    ) -> AppResult<()> {
        let deadline = tokio::time::Instant::now() + super::DISCONNECT_GRACE_TIMEOUT;
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
            tokio::time::sleep(super::DISCONNECT_POLL_DELAY).await;
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
        session.info.status = aspect_core::DebugSessionStatus::Stopped;
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

    fn ensure_paused(&self, session_id: Uuid, action: &str) -> AppResult<()> {
        let Some(session) = self.sessions.get(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        if matches!(
            session.info.status,
            aspect_core::DebugSessionStatus::Paused
        ) {
            Ok(())
        } else {
            Err(AppError::Service(format!(
                "debug session {} is not paused for {action}: {:?}",
                session.info.configuration_name, session.info.status
            )))
        }
    }

    async fn resolve_stack_thread(
        &mut self,
        session_id: Uuid,
    ) -> AppResult<DebugThreadInfo> {
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
