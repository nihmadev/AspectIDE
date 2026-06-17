use std::path::{Path, PathBuf};

use calamine::{open_workbook_auto, Data, Reader};
use icu_locale_core::locale;
use lux_core::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use spreadsheet_ods::{write_ods, Sheet, WorkBook};
use umya_spreadsheet::{new_file_empty_worksheet, writer};

pub const SPREADSHEET_EDIT_FORMAT: &str = "lux-spreadsheet/v1";

const MAX_EDIT_SHEETS: usize = 32;
const MAX_EDIT_ROWS: usize = 2_000;
const MAX_EDIT_COLS: usize = 128;

/// Upper bound on the *declared* total uncompressed size of a zip-container
/// spreadsheet (xlsx/xlsm/xlsb/ods). calamine fully materializes a sheet into a
/// `Range<Data>` before any row/column cap applies, so a crafted small archive
/// that inflates to gigabytes would OOM the process. We reject such files
/// before `open_workbook_auto` ever decompresses them.
const MAX_DECOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;

/// Per-entry compression-ratio ceiling. DEFLATE's theoretical maximum is
/// ~1032:1; legitimate spreadsheet XML stays well below this, so a higher ratio
/// signals a crafted decompression bomb whose header understates its size.
const MAX_COMPRESSION_RATIO: u64 = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetEditDocument {
    pub format: String,
    pub workbook_type: String,
    pub truncated: bool,
    pub sheets: Vec<SpreadsheetEditSheet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetEditSheet {
    pub name: String,
    pub rows: Vec<Vec<String>>,
}

pub fn spreadsheet_edit_text(path: &Path) -> AppResult<String> {
    let document = load_spreadsheet_document(path)?;
    serde_json::to_string_pretty(&document).map_err(AppError::Json)
}

pub fn spreadsheet_write_from_text(path: &Path, text: &str) -> AppResult<PathBuf> {
    let document: SpreadsheetEditDocument = serde_json::from_str(text)?;
    if document.format != SPREADSHEET_EDIT_FORMAT {
        return Err(AppError::Service(format!(
            "unsupported spreadsheet buffer format: {}",
            document.format
        )));
    }
    let save_path = resolve_spreadsheet_save_path(path);
    if save_path.as_path() != path && save_path.exists() {
        return Err(AppError::Service(format!(
            "cannot save: {} already exists",
            save_path.display()
        )));
    }
    write_spreadsheet_document(&save_path, &document)?;
    Ok(save_path)
}

fn resolve_spreadsheet_save_path(path: &Path) -> PathBuf {
    if matches!(extension(path).as_str(), "xls" | "xlsm" | "xlsb") {
        path.with_extension("xlsx")
    } else {
        path.to_path_buf()
    }
}

/// Reject zip-container spreadsheets whose declared uncompressed payload would
/// exhaust memory once calamine materializes a sheet. Non-zip formats (.xls
/// CFB, flat .fods) carry no decompression amplification and are left to the
/// loader.
fn guard_decompression_bomb(path: &Path) -> AppResult<()> {
    if !matches!(extension(path).as_str(), "xlsx" | "xlsm" | "xlsb" | "ods") {
        return Ok(());
    }
    let file = std::fs::File::open(path)?;
    // Not a readable zip; let open_workbook_auto surface the real error.
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return Ok(());
    };
    let mut total_uncompressed: u64 = 0;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| AppError::Service(error.to_string()))?;
        let uncompressed = entry.size();
        let compressed = entry.compressed_size();
        total_uncompressed = total_uncompressed.saturating_add(uncompressed);
        if total_uncompressed > MAX_DECOMPRESSED_BYTES {
            return Err(AppError::Service(format!(
                "spreadsheet rejected: declared uncompressed size exceeds {} MiB (possible decompression bomb)",
                MAX_DECOMPRESSED_BYTES / (1024 * 1024)
            )));
        }
        if compressed > 0 && uncompressed / compressed > MAX_COMPRESSION_RATIO {
            return Err(AppError::Service(format!(
                "spreadsheet rejected: entry '{}' expands {}:1, exceeding the {MAX_COMPRESSION_RATIO}:1 limit (possible decompression bomb)",
                entry.name(),
                uncompressed / compressed
            )));
        }
    }
    Ok(())
}

fn load_spreadsheet_document(path: &Path) -> AppResult<SpreadsheetEditDocument> {
    guard_decompression_bomb(path)?;
    let mut workbook =
        open_workbook_auto(path).map_err(|error| AppError::Service(error.to_string()))?;
    let sheet_count = workbook.sheet_names().len();
    let mut sheets = Vec::new();
    let mut truncated = false;

    for name in workbook.sheet_names().iter().take(MAX_EDIT_SHEETS) {
        let range = workbook
            .worksheet_range(name)
            .map_err(|error| AppError::Service(error.to_string()))?;
        let (height, width) = range.get_size();
        let rows = range
            .rows()
            .take(MAX_EDIT_ROWS)
            .map(|row| {
                row.iter()
                    .take(MAX_EDIT_COLS)
                    .map(cell_to_string)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let sheet_truncated =
            height > MAX_EDIT_ROWS || width > MAX_EDIT_COLS || sheet_count > MAX_EDIT_SHEETS;
        truncated |= sheet_truncated;
        sheets.push(SpreadsheetEditSheet {
            name: name.clone(),
            rows,
        });
    }
    truncated |= sheet_count > sheets.len();

    Ok(SpreadsheetEditDocument {
        format: SPREADSHEET_EDIT_FORMAT.to_string(),
        workbook_type: extension(path),
        truncated,
        sheets,
    })
}

fn write_spreadsheet_document(path: &Path, document: &SpreadsheetEditDocument) -> AppResult<()> {
    let ext = extension(path);
    if matches!(ext.as_str(), "ods" | "fods") {
        return write_ods_document(path, document);
    }
    if ext.as_str() != "xlsx" {
        return Err(AppError::Service(format!(
            "unsupported spreadsheet save format: {ext}"
        )));
    }

    let mut book = new_file_empty_worksheet();
    for (index, sheet) in document.sheets.iter().enumerate() {
        let sheet_name = if sheet.name.trim().is_empty() {
            format!("Sheet{}", index + 1)
        } else {
            sheet.name.clone()
        };
        let worksheet = book
            .new_sheet(&sheet_name)
            .map_err(|error| AppError::Service(error.to_string()))?;
        for (row_index, row) in sheet.rows.iter().enumerate() {
            for (column_index, value) in row.iter().enumerate() {
                if value.is_empty() {
                    continue;
                }
                let address = to_cell_address(row_index, column_index);
                worksheet.get_cell_mut(address).set_value(value);
            }
        }
    }

    if document.sheets.is_empty() {
        book.new_sheet("Sheet1")
            .map_err(|error| AppError::Service(error.to_string()))?;
    }

    writer::xlsx::write(&book, path).map_err(|error| AppError::Service(error.to_string()))
}

fn write_ods_document(path: &Path, document: &SpreadsheetEditDocument) -> AppResult<()> {
    let mut workbook = WorkBook::new(locale!("en-US"));
    for (index, sheet) in document.sheets.iter().enumerate() {
        let sheet_name = if sheet.name.trim().is_empty() {
            format!("Sheet{}", index + 1)
        } else {
            sheet.name.clone()
        };
        let mut ods_sheet = Sheet::new(&sheet_name);
        for (row_index, row) in sheet.rows.iter().enumerate() {
            for (column_index, value) in row.iter().enumerate() {
                if value.is_empty() {
                    continue;
                }
                ods_sheet.set_value(
                    u32::try_from(row_index)
                        .map_err(|error| AppError::Service(error.to_string()))?,
                    u32::try_from(column_index)
                        .map_err(|error| AppError::Service(error.to_string()))?,
                    value.as_str(),
                );
            }
        }
        workbook.push_sheet(ods_sheet);
    }
    if document.sheets.is_empty() {
        workbook.push_sheet(Sheet::new("Sheet1"));
    }
    write_ods(&mut workbook, path).map_err(|error| AppError::Service(error.to_string()))
}

fn to_cell_address(row_index: usize, column_index: usize) -> String {
    let mut column = column_index + 1;
    let mut label = String::new();
    while column > 0 {
        let remainder = (column - 1) % 26;
        label.insert(0, (b'A' + u8::try_from(remainder).unwrap_or(0)) as char);
        column = (column - 1) / 26;
    }
    format!("{label}{}", row_index + 1)
}

fn cell_to_string(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(value) | Data::DateTimeIso(value) | Data::DurationIso(value) => value.clone(),
        Data::Float(value) => value.to_string(),
        Data::Int(value) => value.to_string(),
        Data::Bool(value) => value.to_string(),
        Data::Error(value) => format!("{value:?}"),
        Data::DateTime(value) => value.to_string(),
    }
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use umya_spreadsheet::{new_file, writer as umya_writer};

    #[test]
    fn spreadsheet_roundtrip_preserves_sheet_and_cells() {
        let path = std::env::temp_dir().join("lux_spreadsheet_edit_test.xlsx");
        let seed = build_minimal_xlsx_bytes();
        std::fs::write(&path, seed).expect("seed workbook");

        let text = spreadsheet_edit_text(&path).expect("load edit text");
        let mut document: SpreadsheetEditDocument =
            serde_json::from_str(&text).expect("parse edit json");
        document.sheets[0].rows[0][0] = "Lux".to_string();
        let updated = serde_json::to_string_pretty(&document).expect("serialize");
        let saved_path = spreadsheet_write_from_text(&path, &updated).expect("write workbook");
        assert_eq!(saved_path, path);

        let reloaded = spreadsheet_edit_text(&path).expect("reload");
        let parsed: SpreadsheetEditDocument = serde_json::from_str(&reloaded).expect("parse");
        assert_eq!(parsed.sheets[0].rows[0][0], "Lux");

        let _ = std::fs::remove_file(path);
    }

    fn build_minimal_xlsx_bytes() -> Vec<u8> {
        let mut book = new_file();
        book.get_sheet_mut(&0)
            .expect("default sheet")
            .get_cell_mut("A1")
            .set_value("before");
        let mut buffer = Vec::new();
        umya_writer::xlsx::write_writer(&book, &mut Cursor::new(&mut buffer))
            .expect("write buffer");
        buffer
    }
}
