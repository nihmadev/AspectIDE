use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DebugAdapterStatus {
    Available,
    Missing,
    NotConfigured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DebugConfigurationRequest {
    Launch,
    Attach,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DebugAdapterTransport {
    Stdio,
    TcpServer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DebugSessionStatus {
    Starting,
    Running,
    Paused,
    Stopping,
    Stopped,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DebugExecutionAction {
    Continue,
    StepOver,
    StepIn,
    StepOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DebugEvaluateContext {
    Repl,
    Watch,
    Hover,
    Clipboard,
    Variables,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugSourceBreakpoint {
    pub path: PathBuf,
    #[ts(type = "number")]
    pub line: u64,
    #[ts(type = "number | null")]
    pub column: Option<u64>,
    pub condition: Option<String>,
    pub log_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugResolvedBreakpoint {
    #[ts(type = "number | null")]
    pub id: Option<u64>,
    pub path: PathBuf,
    #[ts(type = "number")]
    pub line: u64,
    #[ts(type = "number | null")]
    pub column: Option<u64>,
    pub verified: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugBreakpointsUpdate {
    pub session_id: Uuid,
    pub path: PathBuf,
    pub breakpoints: Vec<DebugResolvedBreakpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugAdapterInfo {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub configuration_types: Vec<String>,
    pub transport: DebugAdapterTransport,
    pub workspace_root: PathBuf,
    pub status: DebugAdapterStatus,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugConfiguration {
    pub name: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub adapter_type: String,
    pub request: DebugConfigurationRequest,
    #[ts(type = "Record<string, unknown>")]
    pub raw: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugWorkspaceInfo {
    pub adapters: Vec<DebugAdapterInfo>,
    pub configurations: Vec<DebugConfiguration>,
    pub launch_json_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugSessionInfo {
    pub id: Uuid,
    pub configuration_name: String,
    pub adapter_id: String,
    pub adapter_name: String,
    pub workspace_root: PathBuf,
    pub status: DebugSessionStatus,
    pub started_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
    #[ts(type = "number | null")]
    pub active_thread_id: Option<u64>,
    pub last_event: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugThreadInfo {
    #[ts(type = "number")]
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugStackFrame {
    #[ts(type = "number")]
    pub id: u64,
    pub name: String,
    pub source_path: Option<PathBuf>,
    #[ts(type = "number")]
    pub line: u64,
    #[ts(type = "number")]
    pub column: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugStackTrace {
    pub session_id: Uuid,
    pub thread: DebugThreadInfo,
    pub frames: Vec<DebugStackFrame>,
    #[ts(type = "number | null")]
    pub total_frames: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugScopeInfo {
    pub name: String,
    #[ts(type = "number")]
    pub variables_reference: u64,
    pub expensive: bool,
    #[ts(type = "number | null")]
    pub named_variables: Option<u64>,
    #[ts(type = "number | null")]
    pub indexed_variables: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugVariableInfo {
    pub name: String,
    pub value: String,
    pub type_name: Option<String>,
    #[ts(type = "number")]
    pub variables_reference: u64,
    pub evaluate_name: Option<String>,
    #[ts(type = "number | null")]
    pub named_variables: Option<u64>,
    #[ts(type = "number | null")]
    pub indexed_variables: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugFrameScopes {
    pub session_id: Uuid,
    #[ts(type = "number")]
    pub frame_id: u64,
    pub scopes: Vec<DebugScopeInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugVariables {
    pub session_id: Uuid,
    #[ts(type = "number")]
    pub variables_reference: u64,
    pub variables: Vec<DebugVariableInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DebugEvaluateResult {
    pub session_id: Uuid,
    pub expression: String,
    pub result: String,
    pub type_name: Option<String>,
    #[ts(type = "number")]
    pub variables_reference: u64,
    #[ts(type = "number | null")]
    pub named_variables: Option<u64>,
    #[ts(type = "number | null")]
    pub indexed_variables: Option<u64>,
    pub memory_reference: Option<String>,
}
