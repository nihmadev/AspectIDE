use std::borrow::Cow;
use std::path::{Path, PathBuf};

use csv::{ReaderBuilder, WriterBuilder};
use lux_core::{AppError, AppResult};
use serde::{Deserialize, Serialize};

pub const TABLE_EDIT_FORMAT: &str = "lux-table/v1";

const MAX_EDIT_ROWS: usize = 5_000;
const MAX_EDIT_COLS: usize = 256;
const MAX_EDIT_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableEditDocument {
    pub format: String,
    pub delimiter: String,
    pub file_type: String,
    pub truncated: bool,
    pub lossy: bool,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub fn table_edit_text(path: &Path) -> AppResult<String> {
    let document = load_table_document(path)?;
    serde_json::to_string_pretty(&document).map_err(AppError::Json)
}

pub fn table_write_from_text(path: &Path, text: &str) -> AppResult<()> {
    let document: TableEditDocument = serde_json::from_str(text)?;
    if document.format != TABLE_EDIT_FORMAT {
        return Err(AppError::Service(format!(
            "unsupported table buffer format: {}",
            document.format
        )));
    }
    write_table_document(path, &document)
}

fn load_table_document(path: &Path) -> AppResult<TableEditDocument> {
    let metadata =
        std::fs::metadata(path).map_err(|error| AppError::Service(error.to_string()))?;
    if metadata.len() > MAX_EDIT_FILE_BYTES {
        return Err(AppError::Service(format!(
            "table file too large to edit ({} bytes; limit {} bytes)",
            metadata.len(),
            MAX_EDIT_FILE_BYTES
        )));
    }
    let file_type = extension(path);
    let delimiter = delimiter_for_extension(&file_type);
    let mut reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_path(path)
        .map_err(|error| AppError::Service(error.to_string()))?;

    let mut lossy = false;
    let headers = reader
        .byte_headers()
        .map_err(|error| AppError::Service(error.to_string()))?
        .iter()
        .map(|field| decode_field(field, &mut lossy))
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    let mut truncated = false;
    for (index, row) in reader.byte_records().enumerate() {
        if index >= MAX_EDIT_ROWS {
            truncated = true;
            break;
        }
        let record = row.map_err(|error| AppError::Service(error.to_string()))?;
        rows.push(
            record
                .iter()
                .take(MAX_EDIT_COLS)
                .map(|field| decode_field(field, &mut lossy))
                .collect(),
        );
        if record.len() > MAX_EDIT_COLS {
            truncated = true;
        }
    }

    Ok(TableEditDocument {
        format: TABLE_EDIT_FORMAT.to_string(),
        delimiter: String::from(delimiter as char),
        file_type,
        truncated,
        lossy,
        headers,
        rows,
    })
}

/// Decode a CSV field as UTF-8, flagging `lossy` if any invalid bytes had to be
/// replaced. `String::from_utf8_lossy` borrows the input unchanged when it is
/// valid UTF-8 and only allocates a new `String` when it inserts U+FFFD
/// replacement characters, so an owned result precisely signals a lossy decode.
fn decode_field(field: &[u8], lossy: &mut bool) -> String {
    match String::from_utf8_lossy(field) {
        Cow::Borrowed(text) => text.to_owned(),
        Cow::Owned(text) => {
            *lossy = true;
            text
        }
    }
}

fn write_table_document(path: &Path, document: &TableEditDocument) -> AppResult<()> {
    if document.truncated {
        return Err(AppError::Service(
            "refusing to overwrite: table was truncated on load".to_string(),
        ));
    }
    if document.lossy {
        return Err(AppError::Service(
            "refusing to overwrite: source was not valid UTF-8 and cannot be round-tripped"
                .to_string(),
        ));
    }
    let delimiter = document.delimiter.chars().next().unwrap_or(',') as u8;
    let temp_path = temp_sibling_path(path);

    // Write the full document to a sibling temp file first, then atomically
    // rename it over the target so a mid-write failure (disk full, I/O error)
    // can never truncate or corrupt the original file.
    if let Err(error) = write_records_to(&temp_path, delimiter, document) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(AppError::Service(error.to_string()));
    }
    Ok(())
}

fn write_records_to(temp_path: &Path, delimiter: u8, document: &TableEditDocument) -> AppResult<()> {
    let mut writer = WriterBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_path(temp_path)
        .map_err(|error| AppError::Service(error.to_string()))?;
    if !document.headers.is_empty() {
        writer
            .write_record(&document.headers)
            .map_err(|error| AppError::Service(error.to_string()))?;
    }
    for row in &document.rows {
        writer
            .write_record(row)
            .map_err(|error| AppError::Service(error.to_string()))?;
    }
    writer
        .flush()
        .map_err(|error| AppError::Service(error.to_string()))?;
    Ok(())
}

fn temp_sibling_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("table");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    parent.join(format!(
        ".{file_name}.lux-tmp-{}-{}",
        std::process::id(),
        nanos
    ))
}

fn delimiter_for_extension(extension: &str) -> u8 {
    match extension {
        "tsv" => b'\t',
        "psv" => b'|',
        _ => b',',
    }
}

fn extension(path: &Path) -> String {
    lux_core::file_extension_for_path(path)
}
