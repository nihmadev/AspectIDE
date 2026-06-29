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
    file_view_descriptor_for_path, monaco_language_id_for_path, supported_file_formats, AppError,
    AppResult, ArchiveEntryPreview, FileFormatSupport, FileInspection, FileInspectionOptions,
    FileMetadata, FilePreview, FileViewStrategy, NotebookCellPreview, SpreadsheetSheetPreview,
};
use quick_xml::{events::Event, Reader as XmlReader};

use zip::ZipArchive;

mod archive_list;
mod database_edit;
mod delimited_edit;
mod office_extract;

pub use database_edit::{
    database_execute, database_tables, database_update_cell, DatabaseCellUpdate,
    DatabaseExecuteRequest, DatabaseExecuteResult,
};
pub use delimited_edit::{
    table_edit_text, table_write_from_text, TableEditDocument, TABLE_EDIT_FORMAT,
};

const BINARY_SAMPLE_BYTES: usize = 512;
const OFFICE_TEXT_LIMIT: usize = 80_000;
const PDF_TEXT_LIMIT: usize = 120_000;
/// Hard ceiling on bytes decompressed from a single office archive entry, to
/// stop a crafted docx/pptx (zip bomb) from expanding to gigabytes. Generously
/// above `OFFICE_TEXT_LIMIT` so legitimate `document.xml` text still extracts.
const OFFICE_ENTRY_BYTE_CEILING: u64 = 8 * 1024 * 1024;
/// Maximum on-disk size of a notebook we will read+JSON-parse for preview.
const NOTEBOOK_BYTE_CEILING: u64 = 32 * 1024 * 1024;

/// Maximum on-disk size for a PDF we will pass to `pdf_extract`. The extractor
/// is synchronous and materialises the full text, so we gate on file size to
/// avoid stalling the turn loop on huge or adversarial PDFs.
const PDF_BYTE_CEILING: u64 = 64 * 1024 * 1024;

/// Per-cell source/output character cap for notebook preview to prevent a
/// single cell with huge embedded outputs from monopolising the context budget.
const NOTEBOOK_CELL_CHAR_CAP: usize = 8_000;

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
            if let Some(preview) = preview_for_inspect_error(&path, descriptor.strategy) {
                preview
            } else {
                fallback_preview(&path, &descriptor, options)?
            }
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
    match extension(path).as_str() {
        "ipynb" => return notebook_preview(path, options),
        "csv" | "tsv" | "psv" => return table_preview(path, options),
        _ => {}
    }

    match descriptor.strategy {
        FileViewStrategy::MonacoText
        | FileViewStrategy::MarkdownPreview
        | FileViewStrategy::DiagramPreview => text_preview(path, descriptor, options),
        FileViewStrategy::TablePreview | FileViewStrategy::TableEditor => {
            table_preview(path, options)
        }
        FileViewStrategy::SpreadsheetPreview | FileViewStrategy::SpreadsheetEditor => {
            spreadsheet_preview(path, options)
        }
        FileViewStrategy::DatabasePreview | FileViewStrategy::DatabaseEditor => {
            Ok(database_preview(path, options))
        }
        FileViewStrategy::PdfPreview => pdf_preview(path),
        FileViewStrategy::OfficePreview => office_preview(path, options),
        FileViewStrategy::ArchivePreview => Ok(archive_preview(path, options)),
        FileViewStrategy::NotebookPreview => notebook_preview(path, options),
        FileViewStrategy::ImagePreview => Ok(image_preview(path, descriptor)),
        FileViewStrategy::AudioPreview => Ok(audio_preview(path, descriptor)),
        FileViewStrategy::VideoPreview => Ok(video_preview(path, descriptor)),
        FileViewStrategy::BinaryPreview => binary_preview(path),
        FileViewStrategy::ExternalOnly => Ok(external_preview(path, descriptor)),
    }
}

fn preview_for_inspect_error(path: &Path, strategy: FileViewStrategy) -> Option<FilePreview> {
    match strategy {
        FileViewStrategy::PdfPreview => Some(FilePreview::Pdf {
            text: String::new(),
            page_count: None,
            truncated: false,
        }),
        FileViewStrategy::ImagePreview => Some(FilePreview::Image {
            note: "Image preview is rendered directly by the IDE from the file asset.".to_string(),
        }),
        FileViewStrategy::AudioPreview => Some(FilePreview::Audio {
            note: "Audio preview is rendered directly by the IDE from the file asset.".to_string(),
        }),
        FileViewStrategy::VideoPreview => Some(FilePreview::Video {
            note: "Video preview is rendered directly by the IDE from the file asset.".to_string(),
        }),
        FileViewStrategy::OfficePreview => Some(FilePreview::Office {
            text: String::new(),
            parts: Vec::new(),
            truncated: false,
        }),
        FileViewStrategy::ArchivePreview => Some(FilePreview::Archive {
            entries: Vec::new(),
            total_entries: 0,
            truncated: false,
        }),
        FileViewStrategy::DatabasePreview | FileViewStrategy::DatabaseEditor => {
            Some(FilePreview::Database {
                tables: Vec::new(),
                truncated: false,
            })
        }
        FileViewStrategy::SpreadsheetPreview | FileViewStrategy::SpreadsheetEditor => {
            Some(FilePreview::Spreadsheet {
                sheets: Vec::new(),
                workbook_type: extension(path),
                truncated: false,
            })
        }
        FileViewStrategy::NotebookPreview => Some(FilePreview::Notebook {
            cells: Vec::new(),
            cell_count: 0,
            truncated: false,
        }),
        _ => None,
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
    _descriptor: &lux_core::FileViewDescriptor,
    options: &FileInspectionOptions,
) -> AppResult<FilePreview> {
    let metadata = fs::metadata(path)?;
    let limit = usize::try_from(options.max_text_bytes.min(metadata.len())).unwrap_or(usize::MAX);
    let mut buffer = Vec::with_capacity(limit);
    fs::File::open(path)?
        .take(limit as u64)
        .read_to_end(&mut buffer)?;
    let text = String::from_utf8_lossy(&buffer).into_owned();
    Ok(FilePreview::Text {
        language_id: monaco_language_id_for_path(path),
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
    let mut truncated = false;
    // Stop iterating once we have one more record than max_rows so we can
    // detect truncation without scanning multi-GB files end-to-end.  Exact
    // total row count is not required for the preview; callers that need it
    // should use a dedicated background task.
    for record in reader.records() {
        let record = record.map_err(|error| AppError::Service(error.to_string()))?;
        if rows.len() < options.max_rows {
            rows.push(
                record
                    .iter()
                    .take(options.max_columns)
                    .map(ToOwned::to_owned)
                    .collect(),
            );
        } else {
            // One extra record confirms there is more data beyond max_rows.
            truncated = true;
            break;
        }
    }
    // When `truncated`, this is a lower bound: the flag signals the real row
    // count exceeds what we collected. Either way the value is `rows.len()`.
    let row_count = rows.len();
    Ok(FilePreview::Table {
        delimiter: char::from(delimiter).to_string(),
        headers,
        rows,
        row_count,
        truncated,
    })
}

fn spreadsheet_preview(path: &Path, options: &FileInspectionOptions) -> AppResult<FilePreview> {
    // Reject zip-bomb workbooks before `open_workbook_auto`/`worksheet_range`
    // materializes a sheet into memory: a ~1 KB crafted xlsx can otherwise
    // inflate to gigabytes and OOM the process during `InspectFile`.
    spreadsheet_edit::guard_decompression_bomb(path)?;
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

fn database_preview(path: &Path, options: &FileInspectionOptions) -> FilePreview {
    if extension(path) == "duckdb" {
        return FilePreview::External {
            reason: "DuckDB files are identified, but direct DuckDB inspection is not bundled yet."
                .to_string(),
        };
    }

    match database_edit::database_tables(path, options) {
        Ok(tables) => {
            let truncated = tables.iter().any(|table| table.truncated);
            FilePreview::Database { tables, truncated }
        }
        Err(error) => FilePreview::External {
            reason: format!("Cannot open SQLite database: {error}"),
        },
    }
}

fn pdf_preview(path: &Path) -> AppResult<FilePreview> {
    // Reject oversized PDFs before we hand them to pdf-extract, which is
    // synchronous and materialises the full decoded text. A large or adversarial
    // PDF can otherwise stall the turn loop for seconds or exhaust memory.
    let file_size = fs::metadata(path).map_or(0, |m| m.len());
    if file_size > PDF_BYTE_CEILING {
        return Ok(FilePreview::Pdf {
            text: format!(
                "[PDF too large to preview inline ({}). Open externally or use a \
                 dedicated PDF reader.]",
                format_mib(file_size)
            ),
            page_count: None,
            truncated: true,
        });
    }

    // pdf-extract 0.10 panics (not just `Err`s) on some malformed PDFs. The
    // extraction is synchronous, so catch the unwind here and turn it into a
    // clean error rather than tearing down the caller. `&Path` is unwind-safe,
    // so no `AssertUnwindSafe` is required.
    let extracted = std::panic::catch_unwind(|| pdf_extract::extract_text(path))
        .map_err(|_| AppError::Service("unable to extract PDF text".to_string()))?
        .unwrap_or_default();
    let truncated = extracted.len() > PDF_TEXT_LIMIT;
    let text = truncate_chars(&extracted, PDF_TEXT_LIMIT);
    // Page count is derived from the full extracted text (form-feed `\x0C`
    // delimits pages), so it reports the document's true page count regardless
    // of text truncation. The `truncated` flag above already signals that the
    // previewed `text` is a partial slice; page_count intentionally describes
    // the whole document, not just the displayed content.
    let page_count = if extracted.is_empty() {
        None
    } else {
        extracted.matches('\x0C').count().checked_add(1)
    };
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
        return office_extract::office_preview_for_path(path, options);
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
            let mut raw = Vec::new();
            entry
                .by_ref()
                .take(OFFICE_ENTRY_BYTE_CEILING)
                .read_to_end(&mut raw)
                .map_err(|error| AppError::Service(error.to_string()))?;
            let xml = String::from_utf8_lossy(&raw);
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

fn archive_preview(path: &Path, options: &FileInspectionOptions) -> FilePreview {
    match archive_list::list_archive_entries(path, options.max_archive_entries) {
        Ok(listing) => FilePreview::Archive {
            entries: listing.entries,
            total_entries: listing.total_entries,
            truncated: listing.truncated,
        },
        Err(_) => FilePreview::External {
            reason: format!(
                "Archive .{} is identified; listing is bundled for zip, tar, tar.gz, tgz, gz, bz2, and xz containers.",
                extension(path)
            ),
        },
    }
}

fn image_preview(path: &Path, descriptor: &lux_core::FileViewDescriptor) -> FilePreview {
    let ext = extension(path);
    let mime = descriptor
        .mime_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let mut lines = vec![
        format!("Raster or vector image (.{ext})."),
        format!("MIME type: {mime}"),
        "The IDE renders this file visually in the image editor pane.".to_string(),
    ];
    if ext == "svg" {
        if let Ok(file) = fs::File::open(path) {
            let mut raw = Vec::new();
            file.take(64 * 1024).read_to_end(&mut raw).ok();
            let markup = String::from_utf8_lossy(&raw);
            lines.push("SVG markup excerpt:".to_string());
            lines.push(truncate_chars(markup.trim(), 12_000));
        }
    } else {
        lines.push(
            "For pixel-level understanding, use chat vision attachment or InspectFile from a vision-capable model."
                .to_string(),
        );
    }
    FilePreview::Image {
        note: lines.join("\n"),
    }
}

fn audio_preview(path: &Path, descriptor: &lux_core::FileViewDescriptor) -> FilePreview {
    let ext = extension(path);
    let mime = descriptor
        .mime_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".to_string());
    FilePreview::Audio {
        note: format!(
            "Audio file (.{ext}, {mime}). Playback is handled by the IDE media viewer; there is no transcript unless you provide one or run speech-to-text separately."
        ),
    }
}

fn video_preview(path: &Path, descriptor: &lux_core::FileViewDescriptor) -> FilePreview {
    let ext = extension(path);
    let mime = descriptor
        .mime_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".to_string());
    FilePreview::Video {
        note: format!(
            "Video file (.{ext}, {mime}). Playback is handled by the IDE media viewer; frame content is not auto-transcribed."
        ),
    }
}

fn external_preview(path: &Path, descriptor: &lux_core::FileViewDescriptor) -> FilePreview {
    FilePreview::External {
        reason: format!(
            "{} (.{}) is opened via the system default application. Extensions: {}.",
            descriptor.display_name,
            extension(path),
            descriptor.extensions.join(", ")
        ),
    }
}

fn notebook_preview(path: &Path, options: &FileInspectionOptions) -> AppResult<FilePreview> {
    let len = fs::metadata(path)?.len();
    if len > NOTEBOOK_BYTE_CEILING {
        return Ok(FilePreview::External {
            reason: format!("Notebook is {len} bytes; too large to parse for preview."),
        });
    }
    let text = fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&text)?;
    // Borrow the cells array directly to avoid cloning the entire notebook value
    // (which can be large when cells contain embedded image outputs).
    let cells_array = value
        .get("cells")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let cell_count = cells_array.len();
    let cells = cells_array
        .iter()
        .take(options.max_rows)
        .enumerate()
        .map(|(index, cell)| {
            let source =
                truncate_chars(&json_text_array(cell.get("source")), NOTEBOOK_CELL_CHAR_CAP);
            let output_text = truncate_chars(
                &cell
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
                NOTEBOOK_CELL_CHAR_CAP,
            );
            NotebookCellPreview {
                index,
                cell_type: cell
                    .get("cell_type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                text: source,
                output_text,
            }
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
            format!("{note}\n\nUse InspectFile again after the file changes on disk.")
        }
        FilePreview::External { reason } => {
            format!("{reason}\n\nUse InspectFile after exporting or converting to a previewable format when possible.")
        }
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

/// Format a byte count as a human-readable `X.Y MiB` string using integer math,
/// avoiding a lossy `u64 as f64` cast (forbidden under `clippy::pedantic`) while
/// preserving one decimal place for display-only size messages.
pub(crate) fn format_mib(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    let whole = bytes / MIB;
    // Tenths of a MiB: scale the remainder by 10 before dividing to keep one
    // fractional digit without floating point.
    let tenths = (bytes % MIB * 10) / MIB;
    format!("{whole}.{tenths} MiB")
}

mod spreadsheet_edit;

pub use spreadsheet_edit::{spreadsheet_edit_text, spreadsheet_write_from_text};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_formats_has_more_than_one_hundred_extensions() {
        assert!(supported_formats().len() > 100);
    }

    #[test]
    fn pdf_inspection_stays_pdf_preview_not_binary() {
        let root = std::env::temp_dir().join("lux-file-intel-sample.pdf");
        fs::write(&root, b"%PDF-1.4\n1 0 obj<<>>endobj\ntrailer<<>>\n%%EOF\n").unwrap();
        let inspection = inspect_file(&root, &FileInspectionOptions::default()).unwrap();
        let _ = fs::remove_file(&root);

        assert!(matches!(inspection.preview, FilePreview::Pdf { .. }));
    }

    #[test]
    fn spreadsheet_preview_rejects_decompression_bomb() {
        use std::io::Write;
        use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

        // A tiny .xlsx-shaped zip whose single entry inflates ~8 MiB from a few
        // KB: the compression ratio blows past the guard's 1000:1 ceiling, so
        // the bomb is rejected before calamine ever materializes a sheet.
        let root = std::env::temp_dir().join("lux-file-intel-bomb.xlsx");
        {
            let file = fs::File::create(&root).unwrap();
            let mut writer = ZipWriter::new(file);
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            writer
                .start_file("xl/worksheets/sheet1.xml", options)
                .unwrap();
            writer.write_all(&vec![0u8; 8 * 1024 * 1024]).unwrap();
            writer.finish().unwrap();
        }

        let result = spreadsheet_preview(&root, &FileInspectionOptions::default());
        let _ = fs::remove_file(&root);

        match result {
            Err(AppError::Service(message)) => {
                assert!(
                    message.contains("decompression bomb"),
                    "unexpected error message: {message}"
                );
            }
            other => panic!("expected decompression-bomb rejection, got: {other:?}"),
        }
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
