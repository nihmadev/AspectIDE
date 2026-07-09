use std::path::Path;

use aspect_core::{
    DiagnosticSeverity, LspCodeActionDiagnostic, LspCodeActionTrigger, LspFormattingOptions,
    LspRange,
};
use serde_json::{json, Value};

use super::uri::path_to_file_uri;

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
