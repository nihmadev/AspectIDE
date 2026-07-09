use std::{fmt::Write as _, path::Path};

use aspect_core::DocumentSnapshot;

#[must_use]
pub fn path_to_file_uri(path: &Path) -> String {
    let mut normalized = path.to_string_lossy().replace('\\', "/");
    if let Some(rest) = normalized.strip_prefix("//?/UNC/") {
        normalized = format!("//{rest}");
    } else if let Some(rest) = normalized.strip_prefix("//?/") {
        normalized = rest.to_string();
    }
    let absolute_path = if cfg!(windows) && normalized.as_bytes().get(1) == Some(&b':') {
        format!("/{normalized}")
    } else {
        normalized
    };
    format!("file://{}", percent_encode_path(&absolute_path))
}

pub(crate) fn lsp_language_id(document: &DocumentSnapshot) -> &str {
    match document.language_id.as_str() {
        "javascript" => "javascript",
        "typescript" => "typescript",
        other => other,
    }
}

pub(crate) fn document_version(document: &DocumentSnapshot) -> i32 {
    i32::try_from(document.version).unwrap_or(i32::MAX)
}

pub(crate) fn document_path(document: &DocumentSnapshot) -> &Path {
    document
        .path
        .as_deref()
        .unwrap_or_else(|| Path::new(document.title.as_str()))
}

fn percent_encode_path(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                encoded.push(char::from(byte));
            }
            _ => write!(&mut encoded, "%{byte:02X}").expect("writing to String cannot fail"),
        }
    }

    encoded
}
