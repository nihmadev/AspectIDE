use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use aspect_core::{
    AppError, AppResult, DebugBreakpointsUpdate, DebugResolvedBreakpoint, DebugSessionStatus,
    DebugSourceBreakpoint,
};
use uuid::Uuid;

use crate::protocol::{non_empty_text, parse_breakpoints_response, DapResponse};

use super::DebugSession;

pub(super) fn validate_breakpoint_session(
    session_id: Uuid,
    session: Option<&DebugSession>,
) -> AppResult<()> {
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

pub(super) fn normalize_breakpoint_path(path: PathBuf) -> AppResult<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(AppError::InvalidPath("breakpoint path is empty".into()));
    }
    Ok(path)
}

pub(super) fn group_source_breakpoints(
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

pub(super) fn sanitize_source_breakpoints(
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

pub(super) fn apply_breakpoints_response(
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

pub(super) fn store_pending_response_by_seq(
    responses: &mut BTreeMap<u64, DapResponse>,
    response: DapResponse,
) {
    responses.insert(response.request_seq, response);
}

pub(super) fn take_pending_response_by_seq(
    responses: &mut BTreeMap<u64, DapResponse>,
    request_seq: u64,
) -> Option<DapResponse> {
    responses.remove(&request_seq)
}
