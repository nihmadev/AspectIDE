use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::FileViewDescriptor;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileInspectionOptions {
    #[ts(type = "number")]
    pub max_text_bytes: u64,
    pub max_rows: usize,
    pub max_columns: usize,
    pub max_archive_entries: usize,
}

impl Default for FileInspectionOptions {
    fn default() -> Self {
        Self {
            max_text_bytes: 1_000_000,
            max_rows: 80,
            max_columns: 24,
            max_archive_entries: 300,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileInspection {
    pub path: PathBuf,
    pub title: String,
    pub descriptor: FileViewDescriptor,
    pub metadata: FileMetadata,
    pub preview: FilePreview,
    pub ai_context: String,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileMetadata {
    #[ts(type = "number")]
    pub size: u64,
    pub modified_at: Option<DateTime<Utc>>,
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum FilePreview {
    Text {
        language_id: String,
        text: String,
        line_count: usize,
        truncated: bool,
    },
    Table {
        delimiter: String,
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
        row_count: usize,
        truncated: bool,
    },
    Spreadsheet {
        sheets: Vec<SpreadsheetSheetPreview>,
        workbook_type: String,
        truncated: bool,
    },
    Database {
        tables: Vec<DatabaseTablePreview>,
        truncated: bool,
    },
    Pdf {
        text: String,
        page_count: Option<usize>,
        truncated: bool,
    },
    Office {
        text: String,
        parts: Vec<ArchiveEntryPreview>,
        truncated: bool,
    },
    Image {
        note: String,
    },
    Audio {
        note: String,
    },
    Video {
        note: String,
    },
    Archive {
        entries: Vec<ArchiveEntryPreview>,
        total_entries: usize,
        truncated: bool,
    },
    Notebook {
        cells: Vec<NotebookCellPreview>,
        cell_count: usize,
        truncated: bool,
    },
    Binary {
        hex: String,
        ascii: String,
        truncated: bool,
    },
    External {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SpreadsheetSheetPreview {
    pub name: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub row_count: usize,
    pub column_count: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DatabaseTablePreview {
    pub name: String,
    pub kind: String,
    pub columns: Vec<DatabaseColumnPreview>,
    pub rows: Vec<Vec<String>>,
    pub row_count: Option<usize>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DatabaseColumnPreview {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub primary_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArchiveEntryPreview {
    pub path: String,
    #[ts(type = "number")]
    pub compressed_size: u64,
    #[ts(type = "number")]
    pub uncompressed_size: u64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NotebookCellPreview {
    pub index: usize,
    pub cell_type: String,
    pub text: String,
    pub output_text: String,
}
