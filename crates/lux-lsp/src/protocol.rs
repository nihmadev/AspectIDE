use std::{fmt::Write as _, path::Path};

use lux_core::{
    DiagnosticSeverity, DocumentSnapshot, LspCodeActionDiagnostic, LspCodeActionTrigger,
    LspFormattingOptions, LspRange, TextEdit,
};
use serde_json::{json, Value};

use super::{
    LanguageServerDefinition, CLIENT_SEMANTIC_TOKEN_MODIFIERS, CLIENT_SEMANTIC_TOKEN_TYPES,
};

const CLIENT_SYMBOL_KIND_VALUE_SET: &[u8] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
];

const CLIENT_COMPLETION_ITEM_KIND_VALUE_SET: &[u8] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
];

#[must_use]
pub fn initialize_request(id: u64, definition: &LanguageServerDefinition) -> Value {
    let root_uri = path_to_file_uri(&definition.workspace_root);
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": initialize_capabilities(),
            "workspaceFolders": [{
                "uri": root_uri,
                "name": definition.workspace_root.file_name().and_then(|name| name.to_str()).unwrap_or("workspace")
            }]
        }
    })
}

fn initialize_capabilities() -> Value {
    json!({
        "textDocument": text_document_capabilities(),
        "workspace": workspace_capabilities(),
    })
}

fn text_document_capabilities() -> Value {
    json!({
        "publishDiagnostics": {
            "relatedInformation": true,
            "versionSupport": true,
            "codeDescriptionSupport": true,
            "dataSupport": true,
        },
        "synchronization": {
            "dynamicRegistration": false,
            "didSave": true,
        },
        "hover": {
            "dynamicRegistration": false,
            "contentFormat": ["markdown", "plaintext"],
        },
        "definition": {
            "dynamicRegistration": false,
            "linkSupport": true,
        },
        "references": {
            "dynamicRegistration": false,
        },
        "documentSymbol": document_symbol_capabilities(),
        "foldingRange": {
            "dynamicRegistration": false,
            "lineFoldingOnly": true,
        },
        "inlayHint": inlay_hint_capabilities(),
        "semanticTokens": semantic_token_capabilities(),
        "rename": {
            "dynamicRegistration": false,
            "prepareSupport": false,
        },
        "codeAction": code_action_capabilities(),
        "formatting": {
            "dynamicRegistration": false,
        },
        "rangeFormatting": {
            "dynamicRegistration": false,
        },
        "completion": completion_capabilities(),
        "signatureHelp": signature_help_capabilities(),
    })
}

fn document_symbol_capabilities() -> Value {
    json!({
        "dynamicRegistration": false,
        "hierarchicalDocumentSymbolSupport": true,
        "symbolKind": {
            "valueSet": CLIENT_SYMBOL_KIND_VALUE_SET,
        },
    })
}

fn inlay_hint_capabilities() -> Value {
    json!({
        "dynamicRegistration": false,
        "resolveSupport": {
            "properties": ["tooltip", "textEdits", "label.tooltip", "label.location", "label.command"],
        },
    })
}

fn semantic_token_capabilities() -> Value {
    json!({
        "dynamicRegistration": false,
        "requests": {
            "range": false,
            "full": true,
        },
        "tokenTypes": CLIENT_SEMANTIC_TOKEN_TYPES,
        "tokenModifiers": CLIENT_SEMANTIC_TOKEN_MODIFIERS,
        "formats": ["relative"],
        "overlappingTokenSupport": false,
        "multilineTokenSupport": true,
        "serverCancelSupport": false,
        "augmentsSyntaxTokens": true,
    })
}

fn code_action_capabilities() -> Value {
    json!({
        "dynamicRegistration": false,
        "isPreferredSupport": true,
        "disabledSupport": true,
        "dataSupport": false,
        "codeActionLiteralSupport": {
            "codeActionKind": {
                "valueSet": ["", "quickfix", "refactor", "refactor.extract", "refactor.inline", "refactor.rewrite", "source", "source.organizeImports", "source.fixAll"],
            },
        },
    })
}

fn completion_capabilities() -> Value {
    json!({
        "dynamicRegistration": false,
        "completionItem": {
            "snippetSupport": true,
            "commitCharactersSupport": true,
            "documentationFormat": ["markdown", "plaintext"],
            "deprecatedSupport": true,
            "preselectSupport": true,
            "tagSupport": {
                "valueSet": [1],
            },
        },
        "completionItemKind": {
            "valueSet": CLIENT_COMPLETION_ITEM_KIND_VALUE_SET,
        },
        "contextSupport": true,
    })
}

fn signature_help_capabilities() -> Value {
    json!({
        "dynamicRegistration": false,
        "signatureInformation": {
            "documentationFormat": ["markdown", "plaintext"],
            "parameterInformation": {
                "labelOffsetSupport": true,
            },
            "activeParameterSupport": true,
        },
        "contextSupport": true,
    })
}

fn workspace_capabilities() -> Value {
    json!({
        "symbol": {
            "dynamicRegistration": false,
            "symbolKind": {
                "valueSet": CLIENT_SYMBOL_KIND_VALUE_SET,
            },
        },
        "workspaceFolders": false,
        "configuration": false,
    })
}

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

fn document_path(document: &DocumentSnapshot) -> &Path {
    document
        .path
        .as_deref()
        .unwrap_or_else(|| Path::new(document.title.as_str()))
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
pub fn hover_request(id: u64, path: &Path, line: u32, column: u32) -> Value {
    text_document_position_request(id, "textDocument/hover", path, line, column)
}

#[must_use]
pub fn definition_request(id: u64, path: &Path, line: u32, column: u32) -> Value {
    text_document_position_request(id, "textDocument/definition", path, line, column)
}

#[must_use]
pub fn references_request(id: u64, path: &Path, line: u32, column: u32) -> Value {
    let mut request =
        text_document_position_request(id, "textDocument/references", path, line, column);
    request["params"]["context"] = json!({
        "includeDeclaration": true
    });
    request
}

#[must_use]
pub fn document_symbol_request(id: u64, path: &Path) -> Value {
    text_document_request(id, "textDocument/documentSymbol", path)
}

#[must_use]
pub fn workspace_symbol_request(id: u64, query: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "workspace/symbol",
        "params": {
            "query": query
        }
    })
}

#[must_use]
pub fn folding_range_request(id: u64, path: &Path) -> Value {
    text_document_request(id, "textDocument/foldingRange", path)
}

#[must_use]
pub fn inlay_hint_request(id: u64, path: &Path, range: &LspRange) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "textDocument/inlayHint",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path)
            },
            "range": lsp_range_value(range)
        }
    })
}

#[must_use]
pub fn semantic_tokens_full_request(id: u64, path: &Path) -> Value {
    text_document_request(id, "textDocument/semanticTokens/full", path)
}

#[must_use]
pub fn rename_request(id: u64, path: &Path, line: u32, column: u32, new_name: &str) -> Value {
    let mut request = text_document_position_request(id, "textDocument/rename", path, line, column);
    request["params"]["newName"] = json!(new_name);
    request
}

#[must_use]
pub fn completion_request(id: u64, path: &Path, line: u32, column: u32) -> Value {
    let mut request =
        text_document_position_request(id, "textDocument/completion", path, line, column);
    request["params"]["context"] = json!({
        "triggerKind": 1
    });
    request
}

#[must_use]
pub fn signature_help_request(id: u64, path: &Path, line: u32, column: u32) -> Value {
    let mut request =
        text_document_position_request(id, "textDocument/signatureHelp", path, line, column);
    request["params"]["context"] = json!({
        "triggerKind": 1,
        "isRetrigger": false
    });
    request
}

#[must_use]
pub fn code_action_request(
    id: u64,
    path: &Path,
    range: &LspRange,
    diagnostics: &[LspCodeActionDiagnostic],
    only: Option<&[String]>,
    trigger: LspCodeActionTrigger,
) -> Value {
    let mut context = json!({
        "diagnostics": diagnostics.iter().map(lsp_diagnostic_for_code_action).collect::<Vec<_>>(),
        "triggerKind": lsp_code_action_trigger_value(trigger),
    });
    if let Some(only) = only.filter(|items| !items.is_empty()) {
        context["only"] = json!(only);
    }

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path)
            },
            "range": lsp_range_value(range),
            "context": context,
        }
    })
}

#[must_use]
pub fn formatting_request(id: u64, path: &Path, options: LspFormattingOptions) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "textDocument/formatting",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path)
            },
            "options": lsp_formatting_options_value(options),
        }
    })
}

#[must_use]
pub fn range_formatting_request(
    id: u64,
    path: &Path,
    range: &LspRange,
    options: LspFormattingOptions,
) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "textDocument/rangeFormatting",
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path)
            },
            "range": lsp_range_value(range),
            "options": lsp_formatting_options_value(options),
        }
    })
}

fn lsp_range_value(range: &LspRange) -> Value {
    json!({
        "start": {
            "line": range.start_line.saturating_sub(1),
            "character": range.start_column.saturating_sub(1),
        },
        "end": {
            "line": range.end_line.saturating_sub(1),
            "character": range.end_column.saturating_sub(1),
        }
    })
}

fn lsp_formatting_options_value(options: LspFormattingOptions) -> Value {
    json!({
        "tabSize": options.tab_size,
        "insertSpaces": options.insert_spaces,
    })
}

const fn lsp_code_action_trigger_value(trigger: LspCodeActionTrigger) -> u8 {
    match trigger {
        LspCodeActionTrigger::Invoke => 1,
        LspCodeActionTrigger::Automatic => 2,
    }
}

fn lsp_diagnostic_for_code_action(diagnostic: &LspCodeActionDiagnostic) -> Value {
    let mut value = json!({
        "range": lsp_range_value(&diagnostic.range),
        "message": diagnostic.message.clone(),
    });
    if let Some(severity) = diagnostic.severity.map(lsp_diagnostic_severity_value) {
        value["severity"] = json!(severity);
    }
    if let Some(source) = &diagnostic.source {
        value["source"] = json!(source);
    }
    value
}

const fn lsp_diagnostic_severity_value(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Error => 1,
        DiagnosticSeverity::Warning => 2,
        DiagnosticSeverity::Information => 3,
        DiagnosticSeverity::Hint => 4,
    }
}

fn text_document_position_request(
    id: u64,
    method: &str,
    path: &Path,
    line: u32,
    column: u32,
) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": {
            "textDocument": {
                "uri": path_to_file_uri(path)
            },
            "position": {
                "line": line.saturating_sub(1),
                "character": column.saturating_sub(1)
            }
        }
    })
}

fn text_document_request(id: u64, method: &str, path: &Path) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
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

#[must_use]
pub fn path_to_file_uri(path: &Path) -> String {
    let mut normalized = path.to_string_lossy().replace('\\', "/");
    // Defense-in-depth: a Windows `\\?\` verbatim path (std::fs::canonicalize's
    // output) must degrade to the plain drive form here — otherwise the `?`
    // percent-encodes into `file:////%3F/E:/...`, a URI no server resolves,
    // and every request against the document silently returns nothing.
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

fn lsp_language_id(document: &DocumentSnapshot) -> &str {
    match document.language_id.as_str() {
        "javascript" => "javascript",
        "typescript" => "typescript",
        other => other,
    }
}

fn document_version(document: &DocumentSnapshot) -> i32 {
    i32::try_from(document.version).unwrap_or(i32::MAX)
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
