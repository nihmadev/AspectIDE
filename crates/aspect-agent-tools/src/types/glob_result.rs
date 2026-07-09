use std::path::PathBuf;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiGlobResult {
    pub pattern: String,
    pub count: usize,
    pub files: Vec<PathBuf>,
    #[serde(default)]
    pub truncated: bool,
}
