use std::path::PathBuf;

use aspect_core::{DiagnosticSeverity, LspLocation, LspRange, LspTextEdit};
use lsp_types::Uri;
use serde_json::Value;

pub(crate) fn lsp_range_from_value(value: &Value) -> Option<LspRange> {
    let start = value.get("start")?;
    let end = value.get("end")?;
    Some(LspRange {
        start_line: one_based_lsp_position_value(start, "line")?,
        start_column: one_based_lsp_position_value(start, "character")?,
        end_line: one_based_lsp_position_value(end, "line")?,
        end_column: one_based_lsp_position_value(end, "character")?,
    })
}

pub(crate) fn one_based_lsp_position_value(value: &Value, key: &str) -> Option<u32> {
    let raw = value.get(key)?.as_u64()?;
    Some(
        u32::try_from(raw)
            .unwrap_or(u32::MAX.saturating_sub(1))
            .saturating_add(1),
    )
}

pub(crate) fn value_to_u32(value: &Value) -> Option<u32> {
    u32::try_from(value.as_u64()?).ok()
}

pub(crate) fn lsp_location_from_value(value: &Value) -> Option<LspLocation> {
    let candidate = if value.get("targetUri").is_some() {
        (
            value.get("targetUri"),
            value
                .get("targetSelectionRange")
                .or_else(|| value.get("targetRange")),
        )
    } else {
        (value.get("uri"), value.get("range"))
    };
    let uri = candidate.0?.as_str()?.parse::<Uri>().ok()?;
    let path = uri_to_path(&uri)?;
    let range = lsp_range_from_value(candidate.1?)?;
    Some(LspLocation { path, range })
}

pub(crate) fn lsp_text_edit_from_value(value: &Value) -> Option<LspTextEdit> {
    Some(LspTextEdit {
        range: lsp_range_from_value(value.get("range")?)?,
        text: value.get("newText")?.as_str()?.to_string(),
    })
}

pub(crate) fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let value = uri.as_str();
    let path = value.strip_prefix("file://")?;
    let decoded = percent_decode(path);

    #[cfg(windows)]
    {
        let without_leading_slash = decoded
            .strip_prefix('/')
            .filter(|candidate| candidate.as_bytes().get(1) == Some(&b':'))
            .unwrap_or(&decoded);
        Some(PathBuf::from(without_leading_slash.replace('/', "\\")))
    }

    #[cfg(not(windows))]
    {
        Some(PathBuf::from(decoded))
    }
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = hex_value(bytes[index + 1]);
            let low = hex_value(bytes[index + 2]);
            if let (Some(high), Some(low)) = (high, low) {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

const fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn markdown_strings_from_markup(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => non_empty_markdown(text).into_iter().collect(),
        Value::Array(items) => items
            .iter()
            .flat_map(markdown_strings_from_markup)
            .collect(),
        Value::Object(object) => {
            if let Some((language, text)) = object
                .get("language")
                .and_then(Value::as_str)
                .zip(object.get("value").and_then(Value::as_str))
            {
                non_empty_markdown(&format!("```{language}\n{text}\n```"))
                    .into_iter()
                    .collect()
            } else if let Some(value) = object.get("value").and_then(Value::as_str) {
                non_empty_markdown(value).into_iter().collect()
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

pub(crate) fn markup_to_markdown(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => non_empty_markdown(text),
        Value::Object(object) => object
            .get("value")
            .and_then(Value::as_str)
            .and_then(non_empty_markdown),
        _ => None,
    }
}

pub(crate) fn non_empty_markdown(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(crate) fn map_lsp_severity(severity: lsp_types::DiagnosticSeverity) -> DiagnosticSeverity {
    if severity == lsp_types::DiagnosticSeverity::ERROR {
        DiagnosticSeverity::Error
    } else if severity == lsp_types::DiagnosticSeverity::WARNING {
        DiagnosticSeverity::Warning
    } else if severity == lsp_types::DiagnosticSeverity::HINT {
        DiagnosticSeverity::Hint
    } else {
        DiagnosticSeverity::Information
    }
}
