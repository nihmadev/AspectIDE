use std::path::PathBuf;

use aspect_core::{
    DebugBreakpointsUpdate, DebugConfiguration, DebugEvaluateContext, DebugEvaluateResult,
    DebugExecutionAction, DebugFrameScopes, DebugSessionInfo, DebugSourceBreakpoint,
    DebugStackTrace, DebugVariables, DebugWorkspaceInfo,
};
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::{emit_event, lock_error, SharedState};

#[tauri::command]
pub async fn debug_workspace_info(
    state: State<'_, SharedState>,
) -> Result<DebugWorkspaceInfo, String> {
    let root = workspace_root(&state)?;
    tokio::task::spawn_blocking(move || aspect_dap::workspace_debug_info(root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

#[tauri::command]
pub async fn debug_start(
    state: State<'_, SharedState>,
    configuration: DebugConfiguration,
    breakpoints: Vec<DebugSourceBreakpoint>,
) -> Result<DebugSessionInfo, String> {
    let root = workspace_root(&state)?;
    let adapter_root = root.clone();
    let adapter_configuration = configuration.clone();
    let adapter = tokio::task::spawn_blocking(move || {
        aspect_dap::workspace_debug_adapter_for_configuration(adapter_root, &adapter_configuration)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(String::from)?
    .ok_or_else(|| {
        format!(
            "no debug adapter detected for configuration type {}",
            configuration.adapter_type
        )
    })?;

    let mut debug = state.debug.lock().await;
    let manager = debug
        .as_mut()
        .ok_or_else(|| "debug service is not initialized".to_string())?;
    manager
        .start(adapter, configuration, breakpoints, root)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn debug_stop(
    state: State<'_, SharedState>,
    session_id: Uuid,
) -> Result<DebugSessionInfo, String> {
    let mut debug = state.debug.lock().await;
    let manager = debug
        .as_mut()
        .ok_or_else(|| "debug service is not initialized".to_string())?;
    manager.stop(session_id).await.map_err(String::from)
}

#[tauri::command]
pub async fn debug_sessions(
    state: State<'_, SharedState>,
) -> Result<Vec<DebugSessionInfo>, String> {
    let mut debug = state.debug.lock().await;
    let manager = debug
        .as_mut()
        .ok_or_else(|| "debug service is not initialized".to_string())?;
    Ok(manager.sessions().await)
}

#[tauri::command]
pub async fn debug_stack_trace(
    state: State<'_, SharedState>,
    session_id: Uuid,
) -> Result<DebugStackTrace, String> {
    let mut debug = state.debug.lock().await;
    let manager = debug
        .as_mut()
        .ok_or_else(|| "debug service is not initialized".to_string())?;
    manager.stack_trace(session_id).await.map_err(String::from)
}

#[tauri::command]
pub async fn debug_scopes(
    state: State<'_, SharedState>,
    session_id: Uuid,
    frame_id: u64,
) -> Result<DebugFrameScopes, String> {
    let mut debug = state.debug.lock().await;
    let manager = debug
        .as_mut()
        .ok_or_else(|| "debug service is not initialized".to_string())?;
    manager
        .scopes(session_id, frame_id)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn debug_variables(
    state: State<'_, SharedState>,
    session_id: Uuid,
    variables_reference: u64,
) -> Result<DebugVariables, String> {
    let mut debug = state.debug.lock().await;
    let manager = debug
        .as_mut()
        .ok_or_else(|| "debug service is not initialized".to_string())?;
    manager
        .variables(session_id, variables_reference)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn debug_evaluate(
    state: State<'_, SharedState>,
    session_id: Uuid,
    expression: String,
    frame_id: Option<u64>,
    context: DebugEvaluateContext,
) -> Result<DebugEvaluateResult, String> {
    let mut debug = state.debug.lock().await;
    let manager = debug
        .as_mut()
        .ok_or_else(|| "debug service is not initialized".to_string())?;
    manager
        .evaluate(session_id, expression, frame_id, context)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn debug_execute(
    state: State<'_, SharedState>,
    session_id: Uuid,
    action: DebugExecutionAction,
) -> Result<DebugSessionInfo, String> {
    let mut debug = state.debug.lock().await;
    let manager = debug
        .as_mut()
        .ok_or_else(|| "debug service is not initialized".to_string())?;
    manager
        .execute(session_id, action)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn debug_set_breakpoints(
    state: State<'_, SharedState>,
    session_id: Uuid,
    path: PathBuf,
    breakpoints: Vec<DebugSourceBreakpoint>,
) -> Result<DebugBreakpointsUpdate, String> {
    let mut debug = state.debug.lock().await;
    let manager = debug
        .as_mut()
        .ok_or_else(|| "debug service is not initialized".to_string())?;
    manager
        .set_breakpoints(session_id, path, breakpoints)
        .await
        .map_err(String::from)
}

pub async fn stop_all(state: &State<'_, SharedState>) {
    let mut debug = state.debug.lock().await;
    if let Some(manager) = debug.as_mut() {
        manager.stop_all().await;
    }
}

pub fn apply_debug_update(
    app: &AppHandle,
    update: aspect_dap::DebugSessionUpdate,
) -> Result<(), String> {
    match update {
        aspect_dap::DebugSessionUpdate::Changed(session) => {
            emit_event(app, aspect_core::AspectEvent::DebugSessionChanged { session })
        }
        aspect_dap::DebugSessionUpdate::BreakpointsChanged(update) => {
            emit_event(app, aspect_core::AspectEvent::DebugBreakpointsChanged { update })
        }
        aspect_dap::DebugSessionUpdate::Output { session_id, text } => {
            tracing::info!(%session_id, "{text}");
            Ok(())
        }
    }
}

fn workspace_root(state: &State<'_, SharedState>) -> Result<PathBuf, String> {
    state
        .workspace
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .map(|workspace| workspace.root.clone())
        .ok_or_else(|| "no workspace is open".to_string())
}
