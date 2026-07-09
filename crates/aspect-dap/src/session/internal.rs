use std::path::Path;

use aspect_core::{AppError, AppResult, DebugBreakpointsUpdate, DebugResolvedBreakpoint, DebugSessionInfo};
use uuid::Uuid;

use crate::protocol::DapResponse;

use super::{
    apply_breakpoints_response, is_terminal_status, store_pending_response_by_seq,
    take_pending_response_by_seq, DebugSession, DebugSessionManager, DebugSessionUpdate,
};

impl DebugSessionManager {
    pub(super) fn emit_session(&self, session_id: Uuid) -> AppResult<()> {
        let Some(session) = self.sessions.get(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        self.update_tx
            .send(DebugSessionUpdate::Changed(session.info.clone()))
            .map_err(|error| {
                AppError::Service(format!("debug session event channel closed: {error}"))
            })
    }

    pub(super) fn emit_breakpoints(&self, update: DebugBreakpointsUpdate) -> AppResult<()> {
        self.update_tx
            .send(DebugSessionUpdate::BreakpointsChanged(update))
            .map_err(|error| {
                AppError::Service(format!("debug breakpoint event channel closed: {error}"))
            })
    }

    pub(super) fn with_session_mut<T>(
        &mut self,
        session_id: Uuid,
        update: impl FnOnce(&mut DebugSession) -> T,
    ) -> AppResult<T> {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Err(AppError::NotFound(format!("debug session {session_id}")));
        };
        Ok(update(session))
    }

    pub(super) fn next_request_seq(&mut self, session_id: Uuid) -> AppResult<u64> {
        self.with_session_mut(session_id, |session| {
            let seq = session.next_seq;
            session.next_seq += 1;
            seq
        })
    }

    pub(super) fn session_info(&self, session_id: Uuid) -> AppResult<DebugSessionInfo> {
        self.sessions
            .get(&session_id)
            .map(|session| session.info.clone())
            .ok_or_else(|| AppError::NotFound(format!("debug session {session_id}")))
    }

    pub(super) fn session_is_terminal(&self, session_id: Uuid) -> AppResult<bool> {
        self.sessions
            .get(&session_id)
            .map(|session| is_terminal_status(session.info.status))
            .ok_or_else(|| AppError::NotFound(format!("debug session {session_id}")))
    }

    pub(super) fn apply_response(
        &mut self,
        session_id: Uuid,
        response: DapResponse,
    ) -> AppResult<()> {
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

    pub(super) fn store_pending_response(
        &mut self,
        session_id: Uuid,
        response: DapResponse,
    ) -> AppResult<()> {
        self.with_session_mut(session_id, |session| {
            store_pending_response_by_seq(&mut session.pending_responses, response);
        })
    }

    pub(super) fn take_pending_response(
        &mut self,
        session_id: Uuid,
        request_seq: u64,
    ) -> AppResult<Option<DapResponse>> {
        self.with_session_mut(session_id, |session| {
            take_pending_response_by_seq(&mut session.pending_responses, request_seq)
        })
    }

    pub(super) fn mark_session_error(&mut self, session_id: Uuid, error: &AppError) {
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.info.status = aspect_core::DebugSessionStatus::Error;
            session.info.stopped_at.get_or_insert_with(chrono::Utc::now);
            session.info.error = Some(error.to_string());
            session.info.last_event = Some("session error".to_string());
            let _result = self.emit_session(session_id);
        }
    }

    pub(super) fn breakpoints_update(
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

    pub(super) fn unverified_breakpoints_update(
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
}
