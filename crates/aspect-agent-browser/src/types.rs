use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub struct AgentBrowserStatusRequest {
    pub command_path: Option<String>,
    pub skip_auto_update: Option<bool>,
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
    pub command_path: Option<String>,
    pub session_name: Option<String>,
    pub profile: Option<String>,
    pub state_path: Option<String>,
    pub content_boundaries: Option<bool>,
    pub ignore_https_errors: Option<bool>,
    pub allow_file_access: Option<bool>,
    pub provider: Option<String>,
    pub proxy: Option<String>,
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
    pub command_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserSkillsResponse {
    pub success: bool,
    pub content: String,
    pub data: serde_json::Value,
}

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
    pub cwd: Option<String>,
}

pub struct ParsedCliResponse {
    pub success: bool,
    pub data: serde_json::Value,
    pub text: String,
    pub elapsed_ms: u128,
    pub truncated: bool,
    pub exit_code: Option<i32>,
}
