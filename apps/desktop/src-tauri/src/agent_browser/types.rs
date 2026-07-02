//! Request/response DTOs for the agent-browser bridge, plus the internal
//! invoke-options and parsed-response carriers shared across submodules.

use serde::{Deserialize, Serialize};

// ── Request/Response DTOs ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub struct AgentBrowserStatusRequest {
    /// Per-request `command_path` override. IGNORED by the Rust side -- binary
    /// resolution uses trusted sources only (bundled > env var > PATH).
    /// Kept for backward compat; frontend should remove from status requests.
    pub command_path: Option<String>,
    /// Deprecated -- no longer used. Status is always read-only.
    pub skip_auto_update: Option<bool>,
    /// When true, only resolve CLI + version. Skips doctor, session listing,
    /// and npm-latest check (no network, no Chromium).
    pub lightweight: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserStatusResponse {
    pub available: bool,
    pub command_path: Option<String>,
    pub version: Option<String>,
    pub latest_version: Option<String>,
    pub update_performed: bool,
    pub update_detail: Option<String>,
    pub detail: String,
    pub sessions: Vec<String>,
    pub doctor: Option<serde_json::Value>,
    pub binary_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserInvokeRequest {
    pub session: String,
    pub args: Vec<String>,
    pub headed: Option<bool>,
    pub allowed_domains: Option<String>,
    pub max_output: Option<usize>,
    pub timeout_secs: Option<u64>,
    /// Per-request `command_path` override. IGNORED by the Rust side -- binary
    /// resolution uses trusted sources only (bundled > env var > PATH).
    /// Kept for backward compat; frontend should remove from invoke requests.
    pub command_path: Option<String>,
    pub session_name: Option<String>,
    pub profile: Option<String>,
    pub state_path: Option<String>,
    pub content_boundaries: Option<bool>,
    pub ignore_https_errors: Option<bool>,
    pub allow_file_access: Option<bool>,
    pub provider: Option<String>,
    pub proxy: Option<String>,
    /// Working directory for the CLI process (should be the workspace root) so
    /// relative/default output paths land somewhere writable.
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserInvokeResponse {
    pub session: String,
    pub command: String,
    pub success: bool,
    pub data: serde_json::Value,
    pub text: String,
    pub elapsed_ms: u128,
    pub truncated: bool,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub struct AgentBrowserInstallRequest {
    /// Per-request `command_path` override. IGNORED by the Rust side.
    pub command_path: Option<String>,
    pub with_deps: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserInstallStep {
    pub name: String,
    pub success: bool,
    pub output: String,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserInstallResponse {
    pub success: bool,
    pub command_path: Option<String>,
    pub steps: Vec<AgentBrowserInstallStep>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserReadImageRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserReadImageResponse {
    pub path: String,
    pub data_url: String,
    pub bytes: usize,
    pub mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserStreamStatusRequest {
    pub session: String,
    /// Per-request `command_path` override. IGNORED by the Rust side.
    pub command_path: Option<String>,
    pub enable: Option<bool>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserStreamStatusResponse {
    pub session: String,
    pub enabled: bool,
    pub port: Option<u16>,
    pub websocket_url: Option<String>,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserDashboardRequest {
    pub action: String,
    pub port: Option<u16>,
    /// Per-request `command_path` override. IGNORED by the Rust side.
    pub command_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserDashboardResponse {
    pub action: String,
    pub success: bool,
    pub port: Option<u16>,
    pub url: Option<String>,
    pub detail: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub struct AgentBrowserSkillsRequest {
    pub name: Option<String>,
    pub all: Option<bool>,
    /// Per-request `command_path` override. IGNORED by the Rust side.
    pub command_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserSkillsResponse {
    pub success: bool,
    pub content: String,
    pub data: serde_json::Value,
}

// ── Default implementations for request types ──

// ── Internal carriers shared across submodules ──

/// Options threaded into a single `run_json` CLI invocation.
#[derive(Clone)]
pub struct InvokeOptions {
    pub session: String,
    pub headed: Option<bool>,
    pub allowed_domains: Option<String>,
    pub max_output: usize,
    pub session_name: Option<String>,
    pub profile: Option<String>,
    pub state_path: Option<String>,
    pub content_boundaries: Option<bool>,
    pub ignore_https_errors: Option<bool>,
    pub allow_file_access: Option<bool>,
    pub provider: Option<String>,
    pub proxy: Option<String>,
    /// Working directory for the CLI process. Relative output paths (e.g. a
    /// `screenshot foo.png`) and the CLI's default screenshot location resolve
    /// against this, so it should be the workspace root — otherwise the process
    /// inherits the app's launch dir, which is often read-only ("access denied").
    pub cwd: Option<String>,
}

/// Normalised result of a parsed agent-browser CLI JSON response.
pub struct ParsedCliResponse {
    pub success: bool,
    pub data: serde_json::Value,
    pub text: String,
    pub elapsed_ms: u128,
    pub truncated: bool,
    pub exit_code: Option<i32>,
}
