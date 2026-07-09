use std::path::PathBuf;
use serde::Serialize;
use aspect_core::DocumentSnapshot;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiFileOperationStats {
    pub lines_added: usize,
    pub lines_removed: usize,
    pub files_changed: usize,
    pub files_created: usize,
    pub files_deleted: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiFileOperationResult {
    pub operation: String,
    pub path: PathBuf,
    pub saved_to_disk: bool,
    pub changed_paths: Vec<PathBuf>,
    pub edited_documents: Vec<DocumentSnapshot>,
    pub stats: AiFileOperationStats,
    pub message: String,
}
