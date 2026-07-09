use std::path::PathBuf;
use serde::Deserialize;

use super::file_result::AiFileOperationStats;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiFilePatchOperation {
    #[serde(alias = "kind", alias = "operation")]
    pub action: String,
    pub path: PathBuf,
    pub text: Option<String>,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
    pub expected_replacements: Option<usize>,
    pub overwrite: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiPreparedPatchKind {
    Create,
    Rewrite,
    Replace,
    Delete,
}

#[derive(Debug, Clone)]
pub struct AiPreparedPatchOperation {
    pub kind: AiPreparedPatchKind,
    pub path: PathBuf,
    pub after_text: Option<String>,
    pub stats: AiFileOperationStats,
}

#[derive(Debug, Clone)]
pub struct AiPatchRollbackEntry {
    pub path: PathBuf,
    pub previous_bytes: Option<Vec<u8>>,
}
