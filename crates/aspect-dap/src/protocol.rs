//! DAP wire protocol: message types, frame encoding/decoding, request builders,
//! response parsers, and VS Code launch-variable resolution. This module is the
//! transport-agnostic protocol layer вЂ” it has no knowledge of session state.

use std::{
    env,
    path::{Path, PathBuf},
};

use aspect_core::{
    AppError, AppResult, DebugConfiguration, DebugEvaluateContext, DebugEvaluateResult,
    DebugExecutionAction, DebugFrameScopes, DebugResolvedBreakpoint, DebugScopeInfo,
    DebugSourceBreakpoint, DebugStackFrame, DebugStackTrace, DebugThreadInfo, DebugVariableInfo,
    DebugVariables,
};
use serde_json::{json, Value};
use uuid::Uuid;

const MAX_DAP_CONTENT_LENGTH: usize = 64 * 1024 * 1024;
const MAX_DAP_HEADER_LENGTH: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DapFrame {
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DapResponse {
    pub request_seq: u64,
    pub success: bool,
    pub command: String,
    pub message: Option<String>,
    pub body: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DapEvent {
    pub event: String,
    pub body: Option<Value>,
}

/// A reverse request from the adapter (type: "request"). Tracks
/// the `seq` the adapter expects in the response, plus the command
/// and optional arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DapRequest {
    pub seq: u64,
    pub command: String,
    pub arguments: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DapMessage {
    Response(DapResponse),
    Event(DapEvent),
    Request(DapRequest),
}

pub fn encode_dap_message(value: &Value) -> AppResult<Vec<u8>> {
    let content = serde_json::to_vec(value)?;
    let mut message = format!("Content-Length: {}\r\n\r\n", content.len()).into_bytes();
    message.extend_from_slice(&content);
    Ok(message)
}

pub fn drain_dap_frames(buffer: &mut Vec<u8>) -> AppResult<Vec<DapFrame>> {
    let mut frames = Vec::new();

    while let Some(header_end) = find_header_end(buffer) {
        let headers = std::str::from_utf8(&buffer[..header_end])
            .map_err(|error| AppError::Service(format!("invalid DAP header encoding: {error}")))?;
        let content_length = parse_content_length(headers)?;
        if content_length > MAX_DAP_CONTENT_LENGTH {
            return Err(AppError::Service(format!(
                "DAP Content-Length {content_length} exceeds maximum"
            )));
        }
        let frame_start = header_end + 4;
        let Some(frame_end) = frame_start.checked_add(content_length) else {
            return Err(AppError::Service("DAP frame length overflow".into()));
        };

        if buffer.len() < frame_end {
            break;
        }

        let content = buffer[frame_start..frame_end].to_vec();
        buffer.drain(..frame_end);
        frames.push(DapFrame { content });
    }

    // The loop only exits without a complete header when `find_header_end`
    // returns `None`. If the buffer has nonetheless grown past the header bound,
    // the peer is streaming an un-terminated header вЂ” refuse it instead of
    // letting the read buffer grow without limit. A buffer that still contains a
    // header terminator here is a legitimately large frame body in transit and
    // is bounded separately by `MAX_DAP_CONTENT_LENGTH`.
    if buffer.len() > MAX_DAP_HEADER_LENGTH && find_header_end(buffer).is_none() {
        return Err(AppError::Service(format!(
            "DAP header exceeded {MAX_DAP_HEADER_LENGTH} bytes without a terminator"
        )));
    }

    Ok(frames)
}

pub fn parse_dap_message(frame: &DapFrame) -> AppResult<Option<DapMessage>> {
    let value: Value = serde_json::from_slice(&frame.content)?;
    Ok(parse_dap_message_value(&value))
}

#[must_use]
pub fn initialize_request(seq: u64, adapter_id: &str) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "initialize",
        "arguments": {
            "clientID": "aspect-ide",
            "clientName": "AspectIDE",
            "adapterID": adapter_id,
            "pathFormat": "path",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "supportsVariableType": true,
            "supportsVariablePaging": true,
            "supportsRunInTerminalRequest": false,
            "supportsProgressReporting": true,
            "supportsInvalidatedEvent": true,
        }
    })
}

#[must_use]
pub fn launch_request(
    seq: u64,
    configuration: &DebugConfiguration,
    workspace_root: &Path,
) -> Value {
    debug_configuration_request(seq, "launch", configuration, workspace_root)
}

#[must_use]
pub fn attach_request(
    seq: u64,
    configuration: &DebugConfiguration,
    workspace_root: &Path,
) -> Value {
    debug_configuration_request(seq, "attach", configuration, workspace_root)
}

#[must_use]
pub fn configuration_done_request(seq: u64) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "configurationDone",
        "arguments": {}
    })
}

#[must_use]
pub fn disconnect_request(seq: u64, terminate_debuggee: bool) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "disconnect",
        "arguments": {
            "terminateDebuggee": terminate_debuggee,
        }
    })
}

#[must_use]
pub fn threads_request(seq: u64) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "threads",
        "arguments": {}
    })
}

#[must_use]
pub fn stack_trace_request(seq: u64, thread_id: u64, start_frame: u64, levels: u64) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "stackTrace",
        "arguments": {
            "threadId": thread_id,
            "startFrame": start_frame,
            "levels": levels,
        }
    })
}

#[must_use]
pub fn scopes_request(seq: u64, frame_id: u64) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "scopes",
        "arguments": {
            "frameId": frame_id,
        }
    })
}

#[must_use]
pub fn variables_request(seq: u64, variables_reference: u64, start: u64, count: u64) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "variables",
        "arguments": {
            "variablesReference": variables_reference,
            "start": start,
            "count": count,
        }
    })
}

#[must_use]
pub fn evaluate_request(
    seq: u64,
    expression: &str,
    frame_id: Option<u64>,
    context: DebugEvaluateContext,
) -> Value {
    let mut arguments = serde_json::Map::new();
    arguments.insert(
        "expression".to_string(),
        Value::String(expression.to_string()),
    );
    arguments.insert(
        "context".to_string(),
        Value::String(evaluate_context_name(context).to_string()),
    );
    if let Some(frame_id) = frame_id {
        arguments.insert("frameId".to_string(), Value::from(frame_id));
    }

    json!({
        "seq": seq,
        "type": "request",
        "command": "evaluate",
        "arguments": arguments,
    })
}

#[must_use]
pub fn set_breakpoints_request(
    seq: u64,
    path: &Path,
    breakpoints: &[DebugSourceBreakpoint],
) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "setBreakpoints",
        "arguments": {
            "source": {
                "path": path.to_string_lossy(),
            },
            "breakpoints": breakpoints.iter().map(source_breakpoint_argument).collect::<Vec<_>>(),
            "sourceModified": false,
        }
    })
}

#[must_use]
pub fn execution_request(seq: u64, action: DebugExecutionAction, thread_id: u64) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": execution_action_command(action),
        "arguments": {
            "threadId": thread_id,
        }
    })
}

#[must_use]
pub fn parse_breakpoints_response(
    path: &Path,
    requested: &[DebugSourceBreakpoint],
    body: Option<&Value>,
) -> Vec<DebugResolvedBreakpoint> {
    let Some(items) = body
        .and_then(|value| value.get("breakpoints"))
        .and_then(Value::as_array)
    else {
        return requested
            .iter()
            .map(|breakpoint| {
                unresolved_breakpoint(path, breakpoint, "adapter returned no breakpoint data")
            })
            .collect();
    };

    requested
        .iter()
        .enumerate()
        .map(|(index, requested_breakpoint)| {
            items.get(index).map_or_else(
                || unresolved_breakpoint(path, requested_breakpoint, "adapter omitted breakpoint"),
                |value| parse_resolved_breakpoint(path, requested_breakpoint, value),
            )
        })
        .collect()
}

fn source_breakpoint_argument(breakpoint: &DebugSourceBreakpoint) -> Value {
    let mut value = serde_json::Map::new();
    value.insert("line".to_string(), Value::from(breakpoint.line));
    if let Some(column) = breakpoint.column {
        value.insert("column".to_string(), Value::from(column));
    }
    if let Some(condition) = non_empty_text(breakpoint.condition.as_deref()) {
        value.insert("condition".to_string(), Value::String(condition));
    }
    if let Some(log_message) = non_empty_text(breakpoint.log_message.as_deref()) {
        value.insert("logMessage".to_string(), Value::String(log_message));
    }
    Value::Object(value)
}

fn parse_resolved_breakpoint(
    path: &Path,
    requested: &DebugSourceBreakpoint,
    value: &Value,
) -> DebugResolvedBreakpoint {
    DebugResolvedBreakpoint {
        id: value.get("id").and_then(Value::as_u64),
        path: value
            .get("source")
            .and_then(|source| source.get("path"))
            .and_then(Value::as_str)
            .map_or_else(|| path.to_path_buf(), PathBuf::from),
        line: value
            .get("line")
            .and_then(Value::as_u64)
            .unwrap_or(requested.line),
        column: value
            .get("column")
            .and_then(Value::as_u64)
            .or(requested.column),
        verified: value
            .get("verified")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        message: value
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
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

#[must_use]
pub const fn execution_action_command(action: DebugExecutionAction) -> &'static str {
    match action {
        DebugExecutionAction::Continue => "continue",
        DebugExecutionAction::StepOver => "next",
        DebugExecutionAction::StepIn => "stepIn",
        DebugExecutionAction::StepOut => "stepOut",
    }
}

#[must_use]
pub const fn evaluate_context_name(context: DebugEvaluateContext) -> &'static str {
    match context {
        DebugEvaluateContext::Repl => "repl",
        DebugEvaluateContext::Watch => "watch",
        DebugEvaluateContext::Hover => "hover",
        DebugEvaluateContext::Clipboard => "clipboard",
        DebugEvaluateContext::Variables => "variables",
    }
}

#[must_use]
pub fn parse_threads_response(body: Option<&Value>) -> Vec<DebugThreadInfo> {
    body.and_then(|value| value.get("threads"))
        .and_then(Value::as_array)
        .map(|threads| threads.iter().filter_map(parse_thread_info).collect())
        .unwrap_or_default()
}

#[must_use]
pub fn parse_stack_trace_response(
    session_id: Uuid,
    thread: DebugThreadInfo,
    body: Option<&Value>,
) -> DebugStackTrace {
    let frames = body
        .and_then(|value| value.get("stackFrames"))
        .and_then(Value::as_array)
        .map(|frames| frames.iter().filter_map(parse_stack_frame).collect())
        .unwrap_or_default();
    let total_frames = body
        .and_then(|value| value.get("totalFrames"))
        .and_then(Value::as_u64);

    DebugStackTrace {
        session_id,
        thread,
        frames,
        total_frames,
    }
}

#[must_use]
pub fn parse_scopes_response(
    session_id: Uuid,
    frame_id: u64,
    body: Option<&Value>,
) -> DebugFrameScopes {
    let scopes = body
        .and_then(|value| value.get("scopes"))
        .and_then(Value::as_array)
        .map(|scopes| scopes.iter().filter_map(parse_scope_info).collect())
        .unwrap_or_default();

    DebugFrameScopes {
        session_id,
        frame_id,
        scopes,
    }
}

#[must_use]
pub fn parse_variables_response(
    session_id: Uuid,
    variables_reference: u64,
    body: Option<&Value>,
) -> DebugVariables {
    let variables = body
        .and_then(|value| value.get("variables"))
        .and_then(Value::as_array)
        .map(|variables| variables.iter().filter_map(parse_variable_info).collect())
        .unwrap_or_default();

    DebugVariables {
        session_id,
        variables_reference,
        variables,
    }
}

pub fn parse_evaluate_response(
    session_id: Uuid,
    expression: String,
    body: Option<&Value>,
) -> AppResult<DebugEvaluateResult> {
    let Some(body) = body else {
        return Err(AppError::Service(
            "debug adapter returned no evaluate body".into(),
        ));
    };
    let result = body
        .get("result")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Service("debug adapter returned no evaluate result".into()))?
        .to_string();

    Ok(DebugEvaluateResult {
        session_id,
        expression,
        result,
        type_name: body.get("type").and_then(Value::as_str).map(str::to_string),
        variables_reference: body
            .get("variablesReference")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        named_variables: body.get("namedVariables").and_then(Value::as_u64),
        indexed_variables: body.get("indexedVariables").and_then(Value::as_u64),
        memory_reference: body
            .get("memoryReference")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn parse_scope_info(value: &Value) -> Option<DebugScopeInfo> {
    Some(DebugScopeInfo {
        name: value.get("name")?.as_str()?.to_string(),
        variables_reference: value.get("variablesReference")?.as_u64()?,
        expensive: value
            .get("expensive")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        named_variables: value.get("namedVariables").and_then(Value::as_u64),
        indexed_variables: value.get("indexedVariables").and_then(Value::as_u64),
    })
}

fn parse_variable_info(value: &Value) -> Option<DebugVariableInfo> {
    Some(DebugVariableInfo {
        name: value.get("name")?.as_str()?.to_string(),
        value: value
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        type_name: value
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string),
        variables_reference: value
            .get("variablesReference")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        evaluate_name: value
            .get("evaluateName")
            .and_then(Value::as_str)
            .map(str::to_string),
        named_variables: value.get("namedVariables").and_then(Value::as_u64),
        indexed_variables: value.get("indexedVariables").and_then(Value::as_u64),
    })
}

/// Parse a DAP `thread` payload. Shared with the session layer, which uses it
/// to maintain its per-session thread cache from `thread` events.
pub fn parse_thread_info(value: &Value) -> Option<DebugThreadInfo> {
    Some(DebugThreadInfo {
        id: value.get("id")?.as_u64()?,
        name: value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Thread")
            .to_string(),
    })
}

fn parse_stack_frame(value: &Value) -> Option<DebugStackFrame> {
    Some(DebugStackFrame {
        id: value.get("id")?.as_u64()?,
        name: value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("frame")
            .to_string(),
        source_path: value
            .get("source")
            .and_then(|source| source.get("path"))
            .and_then(Value::as_str)
            .map(PathBuf::from),
        line: value.get("line").and_then(Value::as_u64).unwrap_or(0),
        column: value.get("column").and_then(Value::as_u64).unwrap_or(0),
    })
}

fn debug_configuration_request(
    seq: u64,
    command: &str,
    configuration: &DebugConfiguration,
    workspace_root: &Path,
) -> Value {
    let mut arguments = serde_json::Map::new();
    let workspace_folder = workspace_root.to_string_lossy();
    for (key, value) in &configuration.raw {
        arguments.insert(
            key.clone(),
            resolve_launch_variables_value(value, &workspace_folder),
        );
    }
    arguments.insert(
        "name".to_string(),
        Value::String(configuration.name.clone()),
    );
    arguments.insert(
        "type".to_string(),
        Value::String(configuration.adapter_type.clone()),
    );
    arguments.insert("request".to_string(), Value::String(command.to_string()));

    json!({
        "seq": seq,
        "type": "request",
        "command": command,
        "arguments": arguments,
    })
}

/// Recursively resolve VS Code-style launch variables in a JSON value.
fn resolve_launch_variables_value(value: &Value, workspace_folder: &str) -> Value {
    match value {
        Value::String(s) => Value::String(resolve_launch_variables(s, workspace_folder)),
        Value::Array(arr) => Value::Array(
            arr.iter()
                .map(|v| resolve_launch_variables_value(v, workspace_folder))
                .collect(),
        ),
        Value::Object(obj) => Value::Object(
            obj.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        resolve_launch_variables_value(v, workspace_folder),
                    )
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Expand `${workspaceFolder}`, `${env:VAR}`, and `${file}` variables
/// in a string. Rejects unresolved required variables clearly.
fn resolve_launch_variables(value: &str, workspace_folder: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        result.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            // No closing brace вЂ” leave as literal.
            result.push_str("${");
            rest = after;
            continue;
        };
        let var_name = &after[..end];
        match var_name {
            // `${file}` falls back to the workspace folder (no active-file context here).
            "workspaceFolder" | "file" => result.push_str(workspace_folder),
            v if v.starts_with("env:") => {
                let env_var = &v[4..];
                match env::var(env_var) {
                    Ok(val) => result.push_str(&val),
                    Err(_) => {
                        // Leave unresolved env var as-is to avoid silent failure.
                        push_literal_variable(&mut result, v);
                    }
                }
            }
            _ => {
                // Unknown variable вЂ” leave as literal.
                push_literal_variable(&mut result, var_name);
            }
        }
        rest = &after[end + 1..];
    }
    result.push_str(rest);
    result
}

/// Re-emit an unresolved `${name}` variable literally onto `result`.
fn push_literal_variable(result: &mut String, name: &str) {
    result.push_str("${");
    result.push_str(name);
    result.push('}');
}

fn parse_dap_message_value(value: &Value) -> Option<DapMessage> {
    match value.get("type")?.as_str()? {
        "response" => Some(DapMessage::Response(DapResponse {
            request_seq: value.get("request_seq")?.as_u64()?,
            success: value
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            command: value.get("command")?.as_str()?.to_string(),
            message: value
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string),
            body: value.get("body").cloned(),
        })),
        "event" => Some(DapMessage::Event(DapEvent {
            event: value.get("event")?.as_str()?.to_string(),
            body: value.get("body").cloned(),
        })),
        "request" => Some(DapMessage::Request(DapRequest {
            seq: value.get("seq")?.as_u64()?,
            command: value.get("command")?.as_str()?.to_string(),
            arguments: value.get("arguments").cloned(),
        })),
        _ => None,
    }
}

/// Trim a value and return `Some` only when non-empty. Shared with the session
/// layer's breakpoint sanitisation.
pub fn non_empty_text(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &str) -> AppResult<usize> {
    for line in headers.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse::<usize>().map_err(|error| {
                AppError::Service(format!("invalid DAP Content-Length: {error}"))
            });
        }
    }

    Err(AppError::Service(
        "DAP frame is missing Content-Length header".into(),
    ))
}

