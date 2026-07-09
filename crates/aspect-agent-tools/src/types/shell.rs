use std::path::PathBuf;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiShellResponse {
    pub workspace_root: PathBuf,
    pub cwd: PathBuf,
    pub command: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u128,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub stdout_truncated: bool,
    #[serde(default)]
    pub stderr_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiShellClassification {
    pub blocked: Option<String>,
    pub warnings: Vec<String>,
    pub read_only: bool,
}
