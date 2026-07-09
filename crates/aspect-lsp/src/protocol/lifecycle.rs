use std::path::Path;

use aspect_core::{DocumentSnapshot, TextEdit};
use serde_json::{json, Value};

use super::uri::{document_path, document_version, lsp_language_id, path_to_file_uri};

#[must_use]
pub fn initialized_notification() -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    })
}

#[must_use]
pub fn did_open_notification(document: &DocumentSnapshot) -> Value {
    let path = document_path(document);
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path),
                "languageId": lsp_language_id(document),
                "version": document_version(document),
                "text": document.text
            }
        }
    })
}

#[must_use]
pub fn did_change_notification(document: &DocumentSnapshot) -> Value {
    let path = document_path(document);
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path),
                "version": document_version(document)
            },
            "contentChanges": [{
                "text": document.text
            }]
        }
    })
}

#[must_use]
pub fn did_change_edits_notification(document: &DocumentSnapshot, edits: &[TextEdit]) -> Value {
    let path = document_path(document);
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path),
                "version": document_version(document)
            },
            "contentChanges": edits.iter().map(lsp_content_change_for_edit).collect::<Vec<_>>()
        }
    })
}

fn lsp_content_change_for_edit(edit: &TextEdit) -> Value {
    json!({
        "range": {
            "start": {
                "line": edit.start_line.saturating_sub(1),
                "character": edit.start_column.saturating_sub(1)
            },
            "end": {
                "line": edit.end_line.saturating_sub(1),
                "character": edit.end_column.saturating_sub(1)
            }
        },
        "text": edit.text
    })
}

#[must_use]
pub fn did_save_notification(document: &DocumentSnapshot) -> Value {
    let path = document_path(document);
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didSave",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path)
            },
            "text": document.text
        }
    })
}

#[must_use]
pub fn did_close_notification(path: &Path) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didClose",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path)
            }
        }
    })
}

#[must_use]
pub fn shutdown_request(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "shutdown",
        "params": null
    })
}

#[must_use]
pub fn exit_notification() -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "exit",
        "params": null
    })
}
