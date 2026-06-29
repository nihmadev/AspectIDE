use std::{
    fs,
    io::{BufReader, Cursor, Read},
    path::Path,
};

use cfb::CompoundFile;
use lux_core::{AppError, AppResult, ArchiveEntryPreview, FileInspectionOptions, FilePreview};
use quick_xml::{events::Event, Reader as XmlReader};
use zip::ZipArchive;

const OFFICE_TEXT_LIMIT: usize = 80_000;

use crate::{extension, truncate_chars};

pub fn office_preview_for_path(
    path: &Path,
    options: &FileInspectionOptions,
) -> AppResult<FilePreview> {
    let ext = extension(path);
    if is_odf_zip(&ext) {
        return odf_zip_office_preview(path, options);
    }
    if ext == "rtf" {
        return Ok(rtf_office_preview(path));
    }
    if matches!(ext.as_str(), "doc" | "dot") {
        return Ok(ole_word_office_preview(path));
    }
    if matches!(ext.as_str(), "ppt" | "pot" | "pps") {
        return Ok(ole_powerpoint_office_preview(path));
    }
    Ok(FilePreview::External {
        reason: format!(
            "Legacy Office format .{ext} is not bundled for direct editing; open externally for full layout fidelity."
        ),
    })
}

fn is_odf_zip(ext: &str) -> bool {
    matches!(ext, "odt" | "ott" | "odp" | "otp")
}

fn odf_zip_office_preview(path: &Path, options: &FileInspectionOptions) -> AppResult<FilePreview> {
    let file = fs::File::open(path)?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| AppError::Service(error.to_string()))?;
    let mut text = String::new();
    let mut parts = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
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
        if name == "content.xml" && text.len() < OFFICE_TEXT_LIMIT {
            let cap = OFFICE_TEXT_LIMIT.saturating_sub(text.len());
            let mut raw = Vec::new();
            entry
                .take((cap as u64) + 4096)
                .read_to_end(&mut raw)
                .map_err(|error| AppError::Service(error.to_string()))?;
            let xml = String::from_utf8_lossy(&raw);
            text.push_str(&extract_xml_text(xml.as_ref(), cap));
        }
    }
    let truncated = text.len() >= OFFICE_TEXT_LIMIT || archive.len() > parts.len();
    Ok(FilePreview::Office {
        text: truncate_chars(text.trim(), OFFICE_TEXT_LIMIT),
        parts,
        truncated,
    })
}

fn rtf_office_preview(path: &Path) -> FilePreview {
    // Read only a bounded prefix so a huge .rtf cannot exhaust memory before the
    // text limit is applied.  We read slightly more bytes than OFFICE_TEXT_LIMIT
    // to give the RTF stripper enough context to produce a full output window;
    // the extra 64 KiB covers any surrounding control words near the cutoff.
    let read_cap = (OFFICE_TEXT_LIMIT as u64).saturating_add(64 * 1024);
    let file_size = fs::metadata(path).map_or(0, |m| m.len());
    let truncated_by_size = file_size > read_cap;

    let raw: String = fs::File::open(path)
        .ok()
        .map(|f| {
            let mut buf = Vec::new();
            BufReader::new(f)
                .take(read_cap)
                .read_to_end(&mut buf)
                .ok();
            // Try UTF-8 first; fall back to lossy decode for legacy encodings.
            match String::from_utf8(buf) {
                Ok(s) => s,
                Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
            }
        })
        .unwrap_or_default();

    let stripped = strip_rtf(&raw);
    let truncated = truncated_by_size || stripped.len() > OFFICE_TEXT_LIMIT;
    let text = truncate_chars(&stripped, OFFICE_TEXT_LIMIT);
    FilePreview::Office {
        text,
        parts: Vec::new(),
        truncated,
    }
}

fn ole_word_office_preview(path: &Path) -> FilePreview {
    let text = truncate_chars(
        &extract_ole_utf16_stream(path, "WordDocument"),
        OFFICE_TEXT_LIMIT,
    );
    FilePreview::Office {
        text,
        parts: Vec::new(),
        truncated: false,
    }
}

fn ole_powerpoint_office_preview(path: &Path) -> FilePreview {
    let mut chunks = Vec::new();
    for stream in [
        "PowerPoint Document",
        "Current User",
        "\u{0005}SummaryInformation",
    ] {
        let chunk = extract_ole_utf16_stream(path, stream);
        if !chunk.trim().is_empty() {
            chunks.push(chunk);
        }
    }
    let text = truncate_chars(&chunks.join("\n\n"), OFFICE_TEXT_LIMIT);
    FilePreview::Office {
        text,
        parts: Vec::new(),
        truncated: false,
    }
}

fn extract_ole_utf16_stream(path: &Path, stream_name: &str) -> String {
    let Ok(file) = fs::File::open(path) else {
        return String::new();
    };
    let Ok(mut comp) = CompoundFile::open(file) else {
        return String::new();
    };
    let mut buffer = Vec::new();
    let Ok(stream) = comp.open_stream(stream_name) else {
        return String::new();
    };
    let cap = (OFFICE_TEXT_LIMIT as u64).saturating_mul(4);
    if stream.take(cap).read_to_end(&mut buffer).is_err() {
        return String::new();
    }
    utf16_le_lossy_text(&buffer)
}

fn utf16_le_lossy_text(bytes: &[u8]) -> String {
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16_lossy(&units)
        .chars()
        .filter(|ch| !ch.is_control() || *ch == '\n' || *ch == '\t')
        .collect::<Vec<_>>()
        .into_iter()
        .collect::<String>()
}

fn strip_rtf(raw: &str) -> String {
    let mut output = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let mut token = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_ascii_alphabetic() {
                    token.push(chars.next().expect("peeked"));
                } else {
                    break;
                }
            }
            if token == "par" || token == "line" {
                output.push('\n');
            }
            if !token.is_empty() {
                if chars.peek() == Some(&'-') {
                    chars.next();
                }
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() {
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            if chars.peek() == Some(&' ') {
                chars.next();
            }
            continue;
        }
        if ch == '{' || ch == '}' {
            continue;
        }
        if !ch.is_control() || ch == '\n' || ch == '\r' {
            output.push(ch);
        }
    }
    output.trim().to_string()
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

#[cfg(test)]
mod tests {
    use super::strip_rtf;

    #[test]
    fn strip_rtf_removes_control_words() {
        let text = strip_rtf(r"{\rtf1\ansi Hello \par World}");
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
    }
}
