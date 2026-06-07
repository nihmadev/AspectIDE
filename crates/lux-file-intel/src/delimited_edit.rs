use std::path::Path;

use csv::{ReaderBuilder, WriterBuilder};
use lux_core::{AppError, AppResult};
use serde::{Deserialize, Serialize};

pub const TABLE_EDIT_FORMAT: &str = "lux-table/v1";

const MAX_EDIT_ROWS: usize = 5_000;
const MAX_EDIT_COLS: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableEditDocument {
    pub format: String,
    pub delimiter: String,
    pub file_type: String,
    pub truncated: bool,
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
    let file_type = extension(path);
    let delimiter = delimiter_for_extension(&file_type);
    let mut reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_path(path)
        .map_err(|error| AppError::Service(error.to_string()))?;

    let headers = reader
        .headers()
        .map_err(|error| AppError::Service(error.to_string()))?
        .iter()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    let mut truncated = false;
    for (index, row) in reader.records().enumerate() {
        if index >= MAX_EDIT_ROWS {
            truncated = true;
            break;
        }
        let record = row.map_err(|error| AppError::Service(error.to_string()))?;
        rows.push(
            record
                .iter()
                .take(MAX_EDIT_COLS)
                .map(ToOwned::to_owned)
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
        headers,
        rows,
    })
}

fn write_table_document(path: &Path, document: &TableEditDocument) -> AppResult<()> {
    let delimiter = document.delimiter.chars().next().unwrap_or(',') as u8;
    let mut writer = WriterBuilder::new()
        .delimiter(delimiter)
        .from_path(path)
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
