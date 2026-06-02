#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{
    fs,
    io::{Cursor, Read},
    path::Path,
};

use calamine::{open_workbook_auto, Data, Reader};
use chrono::{DateTime, Utc};
use lux_core::{
    file_view_descriptor_for_path, supported_file_formats, AppError, AppResult,
    ArchiveEntryPreview, DatabaseColumnPreview, DatabaseTablePreview, FileFormatSupport,
    FileInspection, FileInspectionOptions, FileMetadata, FilePreview, FileViewCategory,
    FileViewStrategy, NotebookCellPreview, SpreadsheetSheetPreview,
};
use quick_xml::{events::Event, Reader as XmlReader};
use rusqlite::{types::ValueRef, Connection, OpenFlags};
use zip::ZipArchive;

const BINARY_SAMPLE_BYTES: usize = 512;
const OFFICE_TEXT_LIMIT: usize = 80_000;
const PDF_TEXT_LIMIT: usize = 120_000;

#[must_use]
pub fn supported_formats() -> Vec<FileFormatSupport> {
    supported_file_formats()
}

pub fn inspect_file(
    path: impl AsRef<Path>,
    options: &FileInspectionOptions,
) -> AppResult<FileInspection> {
    let path = path.as_ref().to_path_buf();
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() {
        return Err(AppError::InvalidPath(format!(
            "path is not a file: {}",
            path.display()
        )));
    }

    let descriptor = file_view_descriptor_for_path(&path);
    let file_metadata = FileMetadata {
        size: metadata.len(),
        modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
        readonly: metadata.permissions().readonly(),
    };
    let title = path
        .file_name()
        .and_then(|value| value.to_str())
        .map_or_else(|| path.to_string_lossy().into_owned(), ToOwned::to_owned);

    let mut warnings = descriptor.notes.clone();
    let preview_result = preview_for_file(&path, &descriptor, options);
    let preview = match preview_result {
        Ok(preview) => preview,
        Err(error) => {
            warnings.push(error.to_string());
            fallback_preview(&path, &descriptor, options)?
        }
    };
    let truncated = preview_truncated(&preview);
    let ai_context = build_ai_context(&path, &descriptor, &preview, &file_metadata, &warnings);

    Ok(FileInspection {
        path,
        title,
        descriptor,
        metadata: file_metadata,
        preview,
        ai_context,
        truncated,
        warnings,
    })
}

fn preview_for_file(
    path: &Path,
    descriptor: &lux_core::FileViewDescriptor,
    options: &FileInspectionOptions,
) -> AppResult<FilePreview> {
    match descriptor.strategy {
        FileViewStrategy::MonacoText
        | FileViewStrategy::MarkdownPreview
        | FileViewStrategy::DiagramPreview => text_preview(path, descriptor, options),
        FileViewStrategy::TablePreview => table_preview(path, options),
        FileViewStrategy::SpreadsheetPreview => spreadsheet_preview(path, options),
        FileViewStrategy::DatabasePreview => database_preview(path, options),
        FileViewStrategy::PdfPreview => pdf_preview(path),
        FileViewStrategy::OfficePreview => office_preview(path, options),
        FileViewStrategy::ArchivePreview => archive_preview(path, options),
        FileViewStrategy::NotebookPreview => notebook_preview(path, options),
        FileViewStrategy::ImagePreview => Ok(FilePreview::Image {
            note: "Image preview is rendered directly by the IDE from the file asset.".to_string(),
        }),
        FileViewStrategy::AudioPreview => Ok(FilePreview::Audio {
            note: "Audio preview is rendered directly by the IDE from the file asset.".to_string(),
        }),
        FileViewStrategy::VideoPreview => Ok(FilePreview::Video {
            note: "Video preview is rendered directly by the IDE from the file asset.".to_string(),
        }),
        FileViewStrategy::BinaryPreview => binary_preview(path),
        FileViewStrategy::ExternalOnly => Ok(FilePreview::External {
            reason: "This format is routed to the system application.".to_string(),
        }),
    }
}

fn fallback_preview(
    path: &Path,
    descriptor: &lux_core::FileViewDescriptor,
    options: &FileInspectionOptions,
) -> AppResult<FilePreview> {
    if descriptor.binary {
        binary_preview(path)
    } else {
        text_preview(path, descriptor, options)
    }
}

fn text_preview(
    path: &Path,
    descriptor: &lux_core::FileViewDescriptor,
    options: &FileInspectionOptions,
) -> AppResult<FilePreview> {
    let metadata = fs::metadata(path)?;
    let limit = usize::try_from(options.max_text_bytes.min(metadata.len())).unwrap_or(usize::MAX);
    let mut file = fs::File::open(path)?;
    let mut buffer = vec![0; limit];
    let read = file.read(&mut buffer)?;
    buffer.truncate(read);
    let text = String::from_utf8_lossy(&buffer).into_owned();
    Ok(FilePreview::Text {
        language_id: language_id_for_path(path, descriptor),
        line_count: text.lines().count(),
        truncated: metadata.len() > options.max_text_bytes,
        text,
    })
}

fn table_preview(path: &Path, options: &FileInspectionOptions) -> AppResult<FilePreview> {
    let delimiter = match extension(path).as_str() {
        "tsv" => b'\t',
        "psv" => b'|',
        _ => b',',
    };
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_path(path)
        .map_err(|error| AppError::Service(error.to_string()))?;
    let headers = reader
        .headers()
        .map(|headers| {
            headers
                .iter()
                .take(options.max_columns)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut rows = Vec::new();
    let mut row_count = 0;
    for record in reader.records() {
        let record = record.map_err(|error| AppError::Service(error.to_string()))?;
        row_count += 1;
        if rows.len() < options.max_rows {
            rows.push(
                record
                    .iter()
                    .take(options.max_columns)
                    .map(ToOwned::to_owned)
                    .collect(),
            );
        }
    }
    Ok(FilePreview::Table {
        delimiter: char::from(delimiter).to_string(),
        headers,
        rows,
        row_count,
        truncated: row_count > options.max_rows,
    })
}

fn spreadsheet_preview(path: &Path, options: &FileInspectionOptions) -> AppResult<FilePreview> {
    let mut workbook =
        open_workbook_auto(path).map_err(|error| AppError::Service(error.to_string()))?;
    let names = workbook.sheet_names();
    let mut sheets = Vec::new();
    let mut truncated = false;
    for name in names.iter().take(12) {
        let range = workbook
            .worksheet_range(name)
            .map_err(|error| AppError::Service(error.to_string()))?;
        let (height, width) = range.get_size();
        let mut row_iter = range.rows();
        let headers = row_iter
            .next()
            .map(|row| {
                row.iter()
                    .take(options.max_columns)
                    .map(cell_to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let rows = row_iter
            .take(options.max_rows)
            .map(|row| {
                row.iter()
                    .take(options.max_columns)
                    .map(cell_to_string)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let sheet_truncated = height.saturating_sub(1) > rows.len() || width > options.max_columns;
        truncated |= sheet_truncated;
        sheets.push(SpreadsheetSheetPreview {
            name: name.clone(),
            headers,
            rows,
            row_count: height,
            column_count: width,
            truncated: sheet_truncated,
        });
    }
    truncated |= names.len() > sheets.len();
    Ok(FilePreview::Spreadsheet {
        sheets,
        workbook_type: extension(path),
        truncated,
    })
}

fn database_preview(path: &Path, options: &FileInspectionOptions) -> AppResult<FilePreview> {
    if extension(path) == "duckdb" {
        return Ok(FilePreview::External {
            reason: "DuckDB files are identified, but direct DuckDB inspection is not bundled yet."
                .to_string(),
        });
    }

    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| AppError::Service(error.to_string()))?;
    let mut statement = connection
        .prepare(
            "SELECT name, type FROM sqlite_schema \
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' \
             ORDER BY type, name",
        )
        .map_err(|error| AppError::Service(error.to_string()))?;
    let schema_rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| AppError::Service(error.to_string()))?;

    let mut tables = Vec::new();
    let mut truncated = false;
    for schema_row in schema_rows.take(40) {
        let (name, kind) = schema_row.map_err(|error| AppError::Service(error.to_string()))?;
        let columns = table_columns(&connection, &name)?;
        let (rows, row_count, rows_truncated) = table_rows(&connection, &name, options)?;
        truncated |= rows_truncated;
        tables.push(DatabaseTablePreview {
            name,
            kind,
            columns,
            rows,
            row_count,
            truncated: rows_truncated,
        });
    }

    Ok(FilePreview::Database { tables, truncated })
}

fn pdf_preview(path: &Path) -> AppResult<FilePreview> {
    let text =
        pdf_extract::extract_text(path).map_err(|error| AppError::Service(error.to_string()))?;
    let truncated = text.len() > PDF_TEXT_LIMIT;
    let text = truncate_chars(&text, PDF_TEXT_LIMIT);
    let page_count = text.matches('\x0C').count().checked_add(1);
    Ok(FilePreview::Pdf {
        text,
        page_count,
        truncated,
    })
}

fn office_preview(path: &Path, options: &FileInspectionOptions) -> AppResult<FilePreview> {
    let ext = extension(path);
    if !matches!(
        ext.as_str(),
        "docx" | "docm" | "dotx" | "pptx" | "pptm" | "potx" | "ppsx"
    ) {
        return Ok(FilePreview::External {
            reason: "Legacy Office/OpenDocument rendering is routed to the system application; metadata remains available in Lux.".to_string(),
        });
    }

    let file = fs::File::open(path)?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| AppError::Service(error.to_string()))?;
    let mut text = String::new();
    let mut parts = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| AppError::Service(error.to_string()))?;
        let name = entry.name().to_string();
        if parts.len() < options.max_archive_entries {
            parts.push(ArchiveEntryPreview {
                path: name.clone(),
                compressed_size: entry.compressed_size(),
                uncompressed_size: entry.size(),
                is_dir: entry.is_dir(),
            });
        }
        if Path::new(&name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
            && office_xml_is_content_part(&name)
            && text.len() < OFFICE_TEXT_LIMIT
        {
            let mut xml = String::new();
            entry
                .read_to_string(&mut xml)
                .map_err(|error| AppError::Service(error.to_string()))?;
            text.push_str(&extract_xml_text(
                &xml,
                OFFICE_TEXT_LIMIT.saturating_sub(text.len()),
            ));
            text.push('\n');
        }
    }
    let truncated = text.len() >= OFFICE_TEXT_LIMIT || archive.len() > parts.len();
    Ok(FilePreview::Office {
        text: truncate_chars(&text, OFFICE_TEXT_LIMIT),
        parts,
        truncated,
    })
}

fn archive_preview(path: &Path, options: &FileInspectionOptions) -> AppResult<FilePreview> {
    if !matches!(
        extension(path).as_str(),
        "zip" | "jar" | "war" | "ear" | "vsix" | "nupkg" | "whl" | "crate" | "apk" | "aab"
    ) {
        return Ok(FilePreview::External {
            reason: "Archive format is identified; direct listing is currently bundled for ZIP-compatible archives.".to_string(),
        });
    }
    let file = fs::File::open(path)?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| AppError::Service(error.to_string()))?;
    let total_entries = archive.len();
    let mut entries = Vec::new();
    for index in 0..total_entries.min(options.max_archive_entries) {
        let entry = archive
            .by_index(index)
            .map_err(|error| AppError::Service(error.to_string()))?;
        entries.push(ArchiveEntryPreview {
            path: entry.name().to_string(),
            compressed_size: entry.compressed_size(),
            uncompressed_size: entry.size(),
            is_dir: entry.is_dir(),
        });
    }
    Ok(FilePreview::Archive {
        entries,
        total_entries,
        truncated: total_entries > options.max_archive_entries,
    })
}

fn notebook_preview(path: &Path, options: &FileInspectionOptions) -> AppResult<FilePreview> {
    let text = fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&text)?;
    let cells_value = value
        .get("cells")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let cell_count = cells_value.len();
    let cells = cells_value
        .iter()
        .take(options.max_rows)
        .enumerate()
        .map(|(index, cell)| NotebookCellPreview {
            index,
            cell_type: cell
                .get("cell_type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            text: json_text_array(cell.get("source")),
            output_text: cell
                .get("outputs")
                .and_then(serde_json::Value::as_array)
                .map(|outputs| {
                    outputs
                        .iter()
                        .map(|output| {
                            json_text_array(output.get("text"))
                                + &json_text_array(
                                    output.get("data").and_then(|data| data.get("text/plain")),
                                )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    Ok(FilePreview::Notebook {
        cells,
        cell_count,
        truncated: cell_count > options.max_rows,
    })
}

fn binary_preview(path: &Path) -> AppResult<FilePreview> {
    let mut file = fs::File::open(path)?;
    let mut buffer = vec![0; BINARY_SAMPLE_BYTES];
    let read = file.read(&mut buffer)?;
    buffer.truncate(read);
    let hex = buffer
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .chunks(16)
        .map(|chunk| chunk.join(" "))
        .collect::<Vec<_>>()
        .join("\n");
    let ascii = buffer
        .iter()
        .map(|byte| match *byte {
            0x20..=0x7e => char::from(*byte),
            _ => '.',
        })
        .collect();
    Ok(FilePreview::Binary {
        hex,
        ascii,
        truncated: fs::metadata(path)?.len() > BINARY_SAMPLE_BYTES as u64,
    })
}

fn table_columns(connection: &Connection, table: &str) -> AppResult<Vec<DatabaseColumnPreview>> {
    let quoted = quote_sqlite_ident(table);
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({quoted})"))
        .map_err(|error| AppError::Service(error.to_string()))?;
    let rows = statement
        .query_map([], |row| {
            Ok(DatabaseColumnPreview {
                name: row.get::<_, String>(1)?,
                type_name: row.get::<_, String>(2)?,
                nullable: row.get::<_, i64>(3)? == 0,
                primary_key: row.get::<_, i64>(5)? > 0,
            })
        })
        .map_err(|error| AppError::Service(error.to_string()))?;
    rows.map(|row| row.map_err(|error| AppError::Service(error.to_string())))
        .collect()
}

fn table_rows(
    connection: &Connection,
    table: &str,
    options: &FileInspectionOptions,
) -> AppResult<(Vec<Vec<String>>, Option<usize>, bool)> {
    let quoted = quote_sqlite_ident(table);
    let count = connection
        .query_row(&format!("SELECT COUNT(*) FROM {quoted}"), [], |row| {
            row.get::<_, i64>(0)
        })
        .ok()
        .and_then(|value| usize::try_from(value).ok());
    let mut statement = connection
        .prepare(&format!(
            "SELECT * FROM {quoted} LIMIT {}",
            options.max_rows.saturating_add(1)
        ))
        .map_err(|error| AppError::Service(error.to_string()))?;
    let column_count = statement.column_count().min(options.max_columns);
    let mut rows_cursor = statement
        .query([])
        .map_err(|error| AppError::Service(error.to_string()))?;
    let mut rows = Vec::new();
    while let Some(row) = rows_cursor
        .next()
        .map_err(|error| AppError::Service(error.to_string()))?
    {
        if rows.len() >= options.max_rows {
            break;
        }
        let mut values = Vec::with_capacity(column_count);
        for index in 0..column_count {
            values.push(sqlite_value_to_string(
                row.get_ref(index)
                    .map_err(|error| AppError::Service(error.to_string()))?,
            ));
        }
        rows.push(values);
    }
    let truncated = count.is_some_and(|value| value > rows.len());
    Ok((rows, count, truncated))
}

fn build_ai_context(
    path: &Path,
    descriptor: &lux_core::FileViewDescriptor,
    preview: &FilePreview,
    metadata: &FileMetadata,
    warnings: &[String],
) -> String {
    let mut parts = vec![format!(
        "File: {}\nType: {} ({:?}/{:?})\nSize: {} bytes",
        path.display(),
        descriptor.display_name,
        descriptor.category,
        descriptor.strategy,
        metadata.size
    )];
    if !warnings.is_empty() {
        parts.push(format!("Warnings:\n{}", warnings.join("\n")));
    }
    parts.push(match preview {
        FilePreview::Text { text, .. }
        | FilePreview::Pdf { text, .. }
        | FilePreview::Office { text, .. } => text.clone(),
        FilePreview::Table { headers, rows, .. } => table_context(headers, rows),
        FilePreview::Spreadsheet { sheets, .. } => sheets
            .iter()
            .map(|sheet| {
                format!(
                    "Sheet: {}\n{}",
                    sheet.name,
                    table_context(&sheet.headers, &sheet.rows)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        FilePreview::Database { tables, .. } => tables
            .iter()
            .map(|table| {
                let columns = table
                    .columns
                    .iter()
                    .map(|column| format!("{} {}", column.name, column.type_name))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{} {} ({columns})\n{}",
                    table.kind,
                    table.name,
                    table
                        .rows
                        .iter()
                        .map(|row| row.join(" | "))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        FilePreview::Archive {
            entries,
            total_entries,
            ..
        } => format!(
            "Archive entries: {total_entries}\n{}",
            entries
                .iter()
                .map(|entry| format!("{}\t{} bytes", entry.path, entry.uncompressed_size))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        FilePreview::Notebook { cells, .. } => cells
            .iter()
            .map(|cell| {
                format!(
                    "Cell {} [{}]\n{}\n{}",
                    cell.index, cell.cell_type, cell.text, cell.output_text
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        FilePreview::Binary { hex, ascii, .. } => {
            format!("Hex sample:\n{hex}\nASCII sample:\n{ascii}")
        }
        FilePreview::Image { note } | FilePreview::Audio { note } | FilePreview::Video { note } => {
            note.clone()
        }
        FilePreview::External { reason } => reason.clone(),
    });
    truncate_chars(&parts.join("\n\n"), 24_000)
}

fn table_context(headers: &[String], rows: &[Vec<String>]) -> String {
    let mut lines = Vec::new();
    if !headers.is_empty() {
        lines.push(headers.join(" | "));
    }
    lines.extend(rows.iter().map(|row| row.join(" | ")));
    lines.join("\n")
}

const fn preview_truncated(preview: &FilePreview) -> bool {
    match preview {
        FilePreview::Text { truncated, .. }
        | FilePreview::Table { truncated, .. }
        | FilePreview::Spreadsheet { truncated, .. }
        | FilePreview::Database { truncated, .. }
        | FilePreview::Pdf { truncated, .. }
        | FilePreview::Office { truncated, .. }
        | FilePreview::Archive { truncated, .. }
        | FilePreview::Notebook { truncated, .. }
        | FilePreview::Binary { truncated, .. } => *truncated,
        FilePreview::Image { .. }
        | FilePreview::Audio { .. }
        | FilePreview::Video { .. }
        | FilePreview::External { .. } => false,
    }
}

fn language_id_for_path(path: &Path, descriptor: &lux_core::FileViewDescriptor) -> String {
    if descriptor.category == FileViewCategory::Markdown {
        return "markdown".to_string();
    }
    match extension(path).as_str() {
        "rs" => "rust",
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "json" | "jsonc" | "json5" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "css" | "scss" | "sass" | "less" => "css",
        "html" | "htm" => "html",
        "sql" => "sql",
        "xml" | "svg" => "xml",
        "py" | "pyw" => "python",
        other if !other.is_empty() => other,
        _ => "plaintext",
    }
    .to_string()
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

fn sqlite_value_to_string(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => String::new(),
        ValueRef::Integer(value) => value.to_string(),
        ValueRef::Real(value) => value.to_string(),
        ValueRef::Text(value) => String::from_utf8_lossy(value).into_owned(),
        ValueRef::Blob(value) => format!("<blob {} bytes>", value.len()),
    }
}

fn quote_sqlite_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn office_xml_is_content_part(path: &str) -> bool {
    path == "word/document.xml"
        || path.starts_with("word/header")
        || path.starts_with("word/footer")
        || path.starts_with("ppt/slides/slide")
        || path.starts_with("ppt/notesSlides/notesSlide")
}

fn extract_xml_text(xml: &str, limit: usize) -> String {
    let mut reader = XmlReader::from_reader(Cursor::new(xml.as_bytes()));
    reader.config_mut().trim_text(true);
    let mut text = String::new();
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Text(event)) => {
                if let Ok(value) = event.decode() {
                    if !text.is_empty() {
                        text.push(' ');
                    }
                    text.push_str(&value);
                    if text.len() >= limit {
                        return truncate_chars(&text, limit);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
        buffer.clear();
    }
    text
}

fn json_text_array(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>()
            .join(""),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn extension(path: &Path) -> String {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if file_name.ends_with(".d.ts") {
        return "d.ts".to_string();
    }
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().take(max_chars).collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_formats_has_more_than_one_hundred_extensions() {
        assert!(supported_formats().len() > 100);
    }

    #[test]
    fn binary_preview_formats_hex_and_ascii() {
        let root = std::env::temp_dir().join("lux-file-intel-binary.bin");
        fs::write(&root, [0x41, 0, 0x7a]).unwrap();
        let preview = binary_preview(&root).unwrap();
        let _ = fs::remove_file(&root);

        match preview {
            FilePreview::Binary { hex, ascii, .. } => {
                assert!(hex.contains("41 00 7a"));
                assert_eq!(ascii, "A.z");
            }
            other => panic!("unexpected preview: {other:?}"),
        }
    }
}
