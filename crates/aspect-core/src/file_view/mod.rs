mod formats;
mod preview;

pub use formats::*;
pub use preview::*;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Typed IPC descriptor mirrors independent frontend capabilities."
)]
pub struct FileViewDescriptor {
    pub category: FileViewCategory,
    pub strategy: FileViewStrategy,
    pub mode: FileOpenMode,
    pub display_name: String,
    pub mime_type: Option<String>,
    pub extensions: Vec<String>,
    pub editable: bool,
    pub previewable: bool,
    pub ai_readable: bool,
    pub binary: bool,
    #[ts(type = "number | null")]
    pub max_inline_bytes: Option<u64>,
    pub notes: Vec<String>,
}

impl Default for FileViewDescriptor {
    fn default() -> Self {
        Self {
            category: FileViewCategory::Text,
            strategy: FileViewStrategy::MonacoText,
            mode: FileOpenMode::EditableText,
            display_name: "Text".to_string(),
            mime_type: Some("text/plain".to_string()),
            extensions: Vec::new(),
            editable: true,
            previewable: true,
            ai_readable: true,
            binary: false,
            max_inline_bytes: Some(1_000_000),
            notes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FileViewCategory {
    Text,
    Code,
    Markdown,
    Config,
    Data,
    Spreadsheet,
    Database,
    Pdf,
    Office,
    Image,
    Audio,
    Video,
    Archive,
    Notebook,
    Diagram,
    Font,
    Executable,
    Binary,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FileViewStrategy {
    MonacoText,
    MarkdownPreview,
    TablePreview,
    SpreadsheetPreview,
    SpreadsheetEditor,
    TableEditor,
    DatabasePreview,
    DatabaseEditor,
    PdfPreview,
    OfficePreview,
    ImagePreview,
    AudioPreview,
    VideoPreview,
    ArchivePreview,
    NotebookPreview,
    DiagramPreview,
    BinaryPreview,
    ExternalOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FileOpenMode {
    EditableText,
    ReadOnlyText,
    Preview,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Typed IPC catalog row exposes independent frontend capability flags."
)]
pub struct FileFormatSupport {
    pub extension: String,
    pub category: FileViewCategory,
    pub strategy: FileViewStrategy,
    pub mode: FileOpenMode,
    pub display_name: String,
    pub mime_type: Option<String>,
    pub editable: bool,
    pub previewable: bool,
    pub ai_readable: bool,
    pub binary: bool,
}
