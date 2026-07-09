use std::path::PathBuf;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiReadFileResult {
    pub path: PathBuf,
    pub text: String,
    pub truncated: bool,
    pub size: u64,
    pub total_lines: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
}
