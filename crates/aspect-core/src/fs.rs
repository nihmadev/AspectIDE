use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::FileViewDescriptor;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FsEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: FsEntryKind,
    #[ts(type = "number")]
    pub size: u64,
    pub modified_at: Option<DateTime<Utc>>,
    pub is_hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FsEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BufferId(pub Uuid);

impl BufferId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for BufferId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DocumentSnapshot {
    pub id: BufferId,
    pub path: Option<PathBuf>,
    pub title: String,
    pub language_id: String,
    pub text: String,
    pub view: FileViewDescriptor,
    #[ts(type = "number")]
    pub version: u64,
    pub is_dirty: bool,
    pub is_untitled: bool,
    pub opened_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TextEdit {
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DocumentEditResult {
    pub id: BufferId,
    pub path: Option<PathBuf>,
    pub title: String,
    #[ts(type = "number")]
    pub version: u64,
    pub is_dirty: bool,
    pub is_untitled: bool,
}

impl From<&DocumentSnapshot> for DocumentEditResult {
    fn from(document: &DocumentSnapshot) -> Self {
        Self {
            id: document.id,
            path: document.path.clone(),
            title: document.title.clone(),
            version: document.version,
            is_dirty: document.is_dirty,
            is_untitled: document.is_untitled,
        }
    }
}
