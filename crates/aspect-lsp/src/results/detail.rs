use std::{
    collections::BTreeMap,
    path::PathBuf,
};

use aspect_core::{
    LspCodeAction, LspInlayHint, LspInlayHintKind, LspSignatureHelp, LspSignatureInformation,
    LspSignatureParameter, LspTextEdit, LspWorkspaceEdit, LspWorkspaceEditFile,
};
use lsp_types::Uri;
use serde_json::Value;

use super::common::{
    lsp_text_edit_from_value, markup_to_markdown, one_based_lsp_position_value,
    uri_to_path, value_to_u32,
};

pub fn parse_inlay_hint_result(value: &Value) -> Vec<LspInlayHint> {
    value
        .as_array()
        .map(|hints| hints.iter().filter_map(parse_inlay_hint_item).collect())
        .unwrap_or_default()
}

pub fn parse_signature_help_result(value: &Value) -> Option<LspSignatureHelp> {
    if value.is_null() {
        return None;
    }
    let signatures = value
        .get("signatures")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(parse_signature_information)
        .collect::<Vec<_>>();
    if signatures.is_empty() {
        return None;
    }

    Some(LspSignatureHelp {
        signatures,
        active_signature: value.get("activeSignature").and_then(value_to_u32),
        active_parameter: value.get("activeParameter").and_then(value_to_u32),
    })
}

pub fn parse_text_edits_result(value: &Value) -> Vec<LspTextEdit> {
    value
        .as_array()
        .map(|edits| edits.iter().filter_map(lsp_text_edit_from_value).collect())
        .unwrap_or_default()
}

pub fn parse_workspace_edit_result(value: &Value) -> Option<LspWorkspaceEdit> {
    if value.is_null() {
        return None;
    }

    let mut files = BTreeMap::<PathBuf, Vec<LspTextEdit>>::new();
    if value
        .get("documentChanges")
        .and_then(Value::as_array)
        .is_none()
    {
        if let Some(changes) = value.get("changes").and_then(Value::as_object) {
            for (uri, edits) in changes {
                let Ok(uri) = uri.parse::<Uri>() else {
                    continue;
                };
                let Some(path) = uri_to_path(&uri) else {
                    continue;
                };
                let Some(edits) = edits.as_array() else {
                    continue;
                };
                files
                    .entry(path)
                    .or_default()
                    .extend(edits.iter().filter_map(lsp_text_edit_from_value));
            }
        }
    }

    if let Some(document_changes) = value.get("documentChanges").and_then(Value::as_array) {
        for change in document_changes {
            if change
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|kind| matches!(kind, "create" | "rename" | "delete"))
            {
                return None;
            }
            let Some(text_document) = change.get("textDocument") else {
                continue;
            };
            let Some(uri) = text_document
                .get("uri")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<Uri>().ok())
            else {
                continue;
            };
            let Some(path) = uri_to_path(&uri) else {
                continue;
            };
            let Some(edits) = change.get("edits").and_then(Value::as_array) else {
                continue;
            };
            files
                .entry(path)
                .or_default()
                .extend(edits.iter().filter_map(lsp_text_edit_from_value));
        }
    }

    let files = files
        .into_iter()
        .filter_map(|(path, edits)| {
            (!edits.is_empty()).then_some(LspWorkspaceEditFile { path, edits })
        })
        .collect::<Vec<_>>();

    (!files.is_empty()).then_some(LspWorkspaceEdit { files })
}

pub fn parse_code_action_result(value: &Value) -> Vec<LspCodeAction> {
    value
        .as_array()
        .map(|actions| actions.iter().filter_map(parse_code_action).collect())
        .unwrap_or_default()
}

const COMMAND_ONLY_ACTION_REASON: &str =
    "This action runs a language-server command, which AspectIDE can't execute yet — apply it from the server's own UI.";

fn parse_code_action(value: &Value) -> Option<LspCodeAction> {
    let title = value.get("title")?.as_str()?.to_string();
    let disabled_reason = value
        .get("disabled")
        .and_then(|disabled| disabled.get("reason"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let edit = value.get("edit").and_then(parse_workspace_edit_result);
    let has_command = value
        .get("command")
        .is_some_and(|command| !command.is_null());
    let disabled_reason = match (&disabled_reason, &edit, has_command) {
        (Some(_), _, _) => disabled_reason,
        (None, None, true) => Some(COMMAND_ONLY_ACTION_REASON.to_string()),
        (None, None, false) => return None,
        (None, Some(_), _) => None,
    };

    Some(LspCodeAction {
        title,
        kind: value
            .get("kind")
            .and_then(Value::as_str)
            .map(str::to_string),
        is_preferred: value
            .get("isPreferred")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        disabled_reason,
        edit,
    })
}

fn parse_inlay_hint_item(value: &Value) -> Option<LspInlayHint> {
    let position = value.get("position")?;
    Some(LspInlayHint {
        label: inlay_hint_label(value.get("label")?)?,
        tooltip: value.get("tooltip").and_then(markup_to_markdown),
        line: one_based_lsp_position_value(position, "line")?,
        column: one_based_lsp_position_value(position, "character")?,
        kind: value.get("kind").and_then(parse_inlay_hint_kind),
        padding_left: value
            .get("paddingLeft")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        padding_right: value
            .get("paddingRight")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn inlay_hint_label(value: &Value) -> Option<String> {
    match value {
        Value::String(label) => Some(label.clone()),
        Value::Array(parts) => {
            let label = parts
                .iter()
                .filter_map(|part| {
                    part.get("value")
                        .or_else(|| part.get("label"))
                        .and_then(Value::as_str)
                })
                .collect::<String>();
            (!label.is_empty()).then_some(label)
        }
        _ => None,
    }
}

fn parse_inlay_hint_kind(value: &Value) -> Option<LspInlayHintKind> {
    Some(match value.as_u64()? {
        1 => LspInlayHintKind::Type,
        2 => LspInlayHintKind::Parameter,
        _ => return None,
    })
}

fn parse_signature_information(value: &Value) -> Option<LspSignatureInformation> {
    let label = value.get("label")?.as_str()?.to_string();
    let parameters = value
        .get("parameters")
        .and_then(Value::as_array)
        .map(|parameters| {
            parameters
                .iter()
                .filter_map(|parameter| parse_signature_parameter(parameter, &label))
                .collect()
        })
        .unwrap_or_default();

    Some(LspSignatureInformation {
        label,
        documentation: value.get("documentation").and_then(markup_to_markdown),
        parameters,
        active_parameter: value.get("activeParameter").and_then(value_to_u32),
    })
}

fn parse_signature_parameter(
    value: &Value,
    signature_label: &str,
) -> Option<LspSignatureParameter> {
    let label = value
        .get("label")
        .and_then(|label| signature_parameter_label(label, signature_label))?;
    Some(LspSignatureParameter {
        label,
        documentation: value.get("documentation").and_then(markup_to_markdown),
    })
}

fn signature_parameter_label(value: &Value, signature_label: &str) -> Option<String> {
    if let Some(label) = value.as_str() {
        return Some(label.to_string());
    }
    let range = value.as_array()?;
    if range.len() != 2 {
        return None;
    }
    let start = usize::try_from(range[0].as_u64()?).ok()?;
    let end = usize::try_from(range[1].as_u64()?).ok()?;
    let units: Vec<u16> = signature_label.encode_utf16().collect();
    units.get(start..end).map(String::from_utf16_lossy)
}
