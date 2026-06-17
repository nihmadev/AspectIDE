use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use lsp_types::{Diagnostic, PublishDiagnosticsParams, Uri};
use lux_core::{
    DiagnosticSeverity, LspCodeAction, LspCompletionItem, LspCompletionItemKind, LspCompletionList,
    LspDocumentSymbol, LspFoldingRange, LspFoldingRangeKind, LspHover, LspInlayHint,
    LspInlayHintKind, LspInsertTextFormat, LspLocation, LspRange, LspSemanticTokens,
    LspSignatureHelp, LspSignatureInformation, LspSignatureParameter, LspSymbolKind, LspTextEdit,
    LspWorkspaceEdit, LspWorkspaceEditFile, LspWorkspaceSymbol, WorkspaceDiagnostic,
};
use serde_json::Value;

use super::{
    DiagnosticsUpdate, SemanticTokenLegend, CLIENT_SEMANTIC_TOKEN_MODIFIERS,
    CLIENT_SEMANTIC_TOKEN_TYPES,
};

pub fn diagnostics_update_from_publish(params: PublishDiagnosticsParams) -> DiagnosticsUpdate {
    let path = uri_to_path(&params.uri).unwrap_or_else(|| PathBuf::from(params.uri.as_str()));
    let diagnostics = workspace_diagnostics_for_path(&path, params.diagnostics);
    DiagnosticsUpdate { path, diagnostics }
}

pub fn workspace_diagnostics_from_publish(
    params: PublishDiagnosticsParams,
) -> Vec<WorkspaceDiagnostic> {
    let path = uri_to_path(&params.uri).unwrap_or_else(|| PathBuf::from(params.uri.as_str()));

    workspace_diagnostics_for_path(&path, params.diagnostics)
}

pub fn parse_hover_result(value: &Value) -> Option<LspHover> {
    if value.is_null() {
        return None;
    }
    let contents_value = value.get("contents").unwrap_or(value);
    let contents = markdown_strings_from_markup(contents_value);
    if contents.is_empty() {
        return None;
    }
    Some(LspHover {
        contents,
        range: value.get("range").and_then(lsp_range_from_value),
    })
}

pub fn parse_definition_result(value: &Value) -> Vec<LspLocation> {
    if value.is_null() {
        return Vec::new();
    }

    let values = match value {
        Value::Array(items) => items.iter().collect::<Vec<_>>(),
        _ => vec![value],
    };

    values
        .into_iter()
        .filter_map(lsp_location_from_value)
        .collect()
}

pub fn parse_completion_result(value: &Value) -> LspCompletionList {
    match value {
        Value::Array(items) => LspCompletionList {
            is_incomplete: false,
            items: items.iter().filter_map(parse_completion_item).collect(),
        },
        Value::Object(object) => LspCompletionList {
            is_incomplete: object
                .get("isIncomplete")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            items: object
                .get("items")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(parse_completion_item).collect())
                .unwrap_or_default(),
        },
        _ => empty_completion_list(),
    }
}

#[must_use]
pub fn parse_document_symbol_result(value: &Value) -> Vec<LspDocumentSymbol> {
    value
        .as_array()
        .map(|symbols| {
            symbols
                .iter()
                .filter_map(parse_document_symbol_item)
                .collect()
        })
        .unwrap_or_default()
}

#[must_use]
pub fn parse_workspace_symbol_result(value: &Value) -> Vec<LspWorkspaceSymbol> {
    value
        .as_array()
        .map(|symbols| {
            symbols
                .iter()
                .filter_map(parse_workspace_symbol_item)
                .collect()
        })
        .unwrap_or_default()
}

#[must_use]
pub fn parse_folding_range_result(value: &Value) -> Vec<LspFoldingRange> {
    value
        .as_array()
        .map(|ranges| ranges.iter().filter_map(parse_folding_range_item).collect())
        .unwrap_or_default()
}

pub fn parse_semantic_token_legend_from_initialize(value: &Value) -> Option<SemanticTokenLegend> {
    let legend = value
        .get("capabilities")?
        .get("semanticTokensProvider")?
        .get("legend")?;
    let token_types = legend
        .get("tokenTypes")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let token_modifiers = legend
        .get("tokenModifiers")
        .and_then(Value::as_array)
        .map(|modifiers| {
            modifiers
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    (!token_types.is_empty()).then_some(SemanticTokenLegend {
        token_types,
        token_modifiers,
    })
}

#[must_use]
pub fn parse_inlay_hint_result(value: &Value) -> Vec<LspInlayHint> {
    value
        .as_array()
        .map(|hints| hints.iter().filter_map(parse_inlay_hint_item).collect())
        .unwrap_or_default()
}

pub fn parse_semantic_tokens_result(
    value: &Value,
    legend: &SemanticTokenLegend,
) -> Option<LspSemanticTokens> {
    let data = value
        .get("data")?
        .as_array()?
        .iter()
        .filter_map(|value| value.as_u64().and_then(|value| u32::try_from(value).ok()))
        .collect::<Vec<_>>();
    if data.is_empty() || data.len() % 5 != 0 {
        return None;
    }

    Some(LspSemanticTokens {
        result_id: value
            .get("resultId")
            .and_then(Value::as_str)
            .map(str::to_string),
        data: remap_semantic_token_data(&data, legend),
    })
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

pub fn parse_workspace_edit_result(value: &Value) -> Option<LspWorkspaceEdit> {
    if value.is_null() {
        return None;
    }

    let mut files = BTreeMap::<PathBuf, Vec<LspTextEdit>>::new();
    // Per the LSP spec, when `documentChanges` is present the `changes` map MUST
    // be ignored. Servers frequently populate both for backward compatibility, so
    // processing both would double-apply every edit and corrupt the file.
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
            // File operations (create/rename/delete) cannot be represented by
            // LspWorkspaceEdit. Applying only the sibling text edits would leave
            // the workspace partially modified, so abort the whole edit instead.
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

#[must_use]
pub fn parse_text_edits_result(value: &Value) -> Vec<LspTextEdit> {
    value
        .as_array()
        .map(|edits| edits.iter().filter_map(lsp_text_edit_from_value).collect())
        .unwrap_or_default()
}

#[must_use]
pub fn parse_code_action_result(value: &Value) -> Vec<LspCodeAction> {
    value
        .as_array()
        .map(|actions| actions.iter().filter_map(parse_code_action).collect())
        .unwrap_or_default()
}

fn parse_code_action(value: &Value) -> Option<LspCodeAction> {
    let title = value.get("title")?.as_str()?.to_string();
    let disabled_reason = value
        .get("disabled")
        .and_then(|disabled| disabled.get("reason"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let edit = value.get("edit").and_then(parse_workspace_edit_result);
    if edit.is_none() && disabled_reason.is_none() {
        return None;
    }

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

fn parse_document_symbol_item(value: &Value) -> Option<LspDocumentSymbol> {
    let name = value.get("name")?.as_str()?.to_string();
    let kind = value
        .get("kind")
        .and_then(parse_symbol_kind)
        .unwrap_or(LspSymbolKind::Variable);

    if let Some(selection_range) = value.get("selectionRange").and_then(lsp_range_from_value) {
        let range = value
            .get("range")
            .and_then(lsp_range_from_value)
            .unwrap_or_else(|| selection_range.clone());
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .map(|children| {
                children
                    .iter()
                    .filter_map(parse_document_symbol_item)
                    .collect()
            })
            .unwrap_or_default();
        return Some(LspDocumentSymbol {
            name,
            detail: value
                .get("detail")
                .and_then(Value::as_str)
                .map(str::to_string),
            kind,
            range,
            selection_range,
            children,
        });
    }

    let location = value.get("location").and_then(lsp_location_from_value)?;
    Some(LspDocumentSymbol {
        name,
        detail: value
            .get("containerName")
            .and_then(Value::as_str)
            .map(str::to_string),
        kind,
        range: location.range.clone(),
        selection_range: location.range,
        children: Vec::new(),
    })
}

fn parse_workspace_symbol_item(value: &Value) -> Option<LspWorkspaceSymbol> {
    let name = value.get("name")?.as_str()?.to_string();
    let kind = value
        .get("kind")
        .and_then(parse_symbol_kind)
        .unwrap_or(LspSymbolKind::Variable);
    let location = value
        .get("location")
        .and_then(lsp_location_from_value)
        .or_else(|| {
            let uri = value
                .get("location")?
                .get("uri")
                .or_else(|| value.get("uri"))?
                .as_str()?
                .parse::<Uri>()
                .ok()?;
            let path = uri_to_path(&uri)?;
            let range = value
                .get("location")?
                .get("range")
                .or_else(|| value.get("range"))
                .and_then(lsp_range_from_value)?;
            Some(LspLocation { path, range })
        })?;
    Some(LspWorkspaceSymbol {
        name,
        kind,
        location,
        container_name: value
            .get("containerName")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn parse_folding_range_item(value: &Value) -> Option<LspFoldingRange> {
    Some(LspFoldingRange {
        start_line: one_based_lsp_position_value(value, "startLine")?,
        end_line: one_based_lsp_position_value(value, "endLine")?,
        start_column: value
            .get("startCharacter")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .map(|value| value.saturating_add(1)),
        end_column: value
            .get("endCharacter")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .map(|value| value.saturating_add(1)),
        kind: value
            .get("kind")
            .and_then(Value::as_str)
            .and_then(parse_folding_range_kind),
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

fn remap_semantic_token_data(data: &[u32], legend: &SemanticTokenLegend) -> Vec<u32> {
    let type_indexes = semantic_token_type_indexes(legend);
    let modifier_masks = semantic_token_modifier_masks(legend);
    let mut remapped = Vec::with_capacity(data.len());

    for chunk in data.chunks_exact(5) {
        remapped.extend_from_slice(&chunk[..3]);
        let token_type = usize::try_from(chunk[3])
            .ok()
            .and_then(|index| type_indexes.get(index).copied())
            .flatten()
            .unwrap_or(0);
        let token_modifiers = remap_semantic_token_modifier_mask(chunk[4], &modifier_masks);
        remapped.push(token_type);
        remapped.push(token_modifiers);
    }

    remapped
}

fn semantic_token_type_indexes(legend: &SemanticTokenLegend) -> Vec<Option<u32>> {
    legend
        .token_types
        .iter()
        .map(|token_type| {
            CLIENT_SEMANTIC_TOKEN_TYPES
                .iter()
                .position(|candidate| candidate == token_type)
                .and_then(|index| u32::try_from(index).ok())
        })
        .collect()
}

fn semantic_token_modifier_masks(legend: &SemanticTokenLegend) -> Vec<u32> {
    legend
        .token_modifiers
        .iter()
        .map(|modifier| {
            CLIENT_SEMANTIC_TOKEN_MODIFIERS
                .iter()
                .position(|candidate| candidate == modifier)
                .and_then(|index| u32::try_from(index).ok())
                .map_or(0, |index| 1_u32.checked_shl(index).unwrap_or(0))
        })
        .collect()
}

fn remap_semantic_token_modifier_mask(source_mask: u32, modifier_masks: &[u32]) -> u32 {
    modifier_masks
        .iter()
        .enumerate()
        .fold(0_u32, |acc, (index, target_mask)| {
            let Ok(index) = u32::try_from(index) else {
                return acc;
            };
            let Some(source_bit) = 1_u32.checked_shl(index) else {
                return acc;
            };
            if source_mask & source_bit != 0 {
                acc | target_mask
            } else {
                acc
            }
        })
}

fn parse_symbol_kind(value: &Value) -> Option<LspSymbolKind> {
    Some(match value.as_u64()? {
        1 => LspSymbolKind::File,
        2 => LspSymbolKind::Module,
        3 => LspSymbolKind::Namespace,
        4 => LspSymbolKind::Package,
        5 => LspSymbolKind::Class,
        6 => LspSymbolKind::Method,
        7 => LspSymbolKind::Property,
        8 => LspSymbolKind::Field,
        9 => LspSymbolKind::Constructor,
        10 => LspSymbolKind::Enum,
        11 => LspSymbolKind::Interface,
        12 => LspSymbolKind::Function,
        13 => LspSymbolKind::Variable,
        14 => LspSymbolKind::Constant,
        15 => LspSymbolKind::String,
        16 => LspSymbolKind::Number,
        17 => LspSymbolKind::Boolean,
        18 => LspSymbolKind::Array,
        19 => LspSymbolKind::Object,
        20 => LspSymbolKind::Key,
        21 => LspSymbolKind::Null,
        22 => LspSymbolKind::EnumMember,
        23 => LspSymbolKind::Struct,
        24 => LspSymbolKind::Event,
        25 => LspSymbolKind::Operator,
        26 => LspSymbolKind::TypeParameter,
        _ => return None,
    })
}

fn parse_folding_range_kind(value: &str) -> Option<LspFoldingRangeKind> {
    Some(match value {
        "comment" => LspFoldingRangeKind::Comment,
        "imports" => LspFoldingRangeKind::Imports,
        "region" => LspFoldingRangeKind::Region,
        _ => return None,
    })
}

pub const fn empty_completion_list() -> LspCompletionList {
    LspCompletionList {
        is_incomplete: false,
        items: Vec::new(),
    }
}

fn workspace_diagnostics_for_path(
    path: &Path,
    diagnostics: Vec<Diagnostic>,
) -> Vec<WorkspaceDiagnostic> {
    diagnostics
        .into_iter()
        .map(|diagnostic| WorkspaceDiagnostic {
            path: path.to_path_buf(),
            line: diagnostic.range.start.line.saturating_add(1),
            column: diagnostic.range.start.character.saturating_add(1),
            severity: diagnostic
                .severity
                .map_or(DiagnosticSeverity::Information, map_lsp_severity),
            source: diagnostic.source.unwrap_or_else(|| "lsp".to_string()),
            message: diagnostic.message,
        })
        .collect()
}

fn lsp_location_from_value(value: &Value) -> Option<LspLocation> {
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

fn lsp_text_edit_from_value(value: &Value) -> Option<LspTextEdit> {
    Some(LspTextEdit {
        range: lsp_range_from_value(value.get("range")?)?,
        text: value.get("newText")?.as_str()?.to_string(),
    })
}

fn parse_completion_item(value: &Value) -> Option<LspCompletionItem> {
    let label = value.get("label")?.as_str()?.to_string();
    let text_edit = value.get("textEdit").and_then(parse_completion_text_edit);
    let insert_text = text_edit
        .as_ref()
        .map(|(_, text)| text.clone())
        .or_else(|| {
            value
                .get("insertText")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| label.clone());

    Some(LspCompletionItem {
        label,
        kind: value.get("kind").and_then(parse_completion_item_kind),
        detail: value
            .get("detail")
            .and_then(Value::as_str)
            .map(str::to_string),
        documentation: value
            .get("documentation")
            .and_then(parse_completion_documentation),
        insert_text,
        insert_text_format: value
            .get("insertTextFormat")
            .and_then(Value::as_u64)
            .map_or(LspInsertTextFormat::PlainText, parse_insert_text_format),
        filter_text: value
            .get("filterText")
            .and_then(Value::as_str)
            .map(str::to_string),
        sort_text: value
            .get("sortText")
            .and_then(Value::as_str)
            .map(str::to_string),
        range: text_edit.map(|(range, _)| range),
        commit_characters: value
            .get("commitCharacters")
            .and_then(Value::as_array)
            .map(|characters| {
                characters
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        preselect: value
            .get("preselect")
            .and_then(Value::as_bool)
            .unwrap_or(false),
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
    // Per the LSP spec, ParameterInformation.label tuple offsets are UTF-16
    // code units, not UTF-8 byte offsets, so slice over the UTF-16 encoding.
    let units: Vec<u16> = signature_label.encode_utf16().collect();
    units.get(start..end).map(String::from_utf16_lossy)
}

fn parse_completion_text_edit(value: &Value) -> Option<(LspRange, String)> {
    let range = value
        .get("range")
        .or_else(|| value.get("replace"))
        .and_then(lsp_range_from_value)?;
    let text = value
        .get("newText")
        .or_else(|| value.get("insertText"))
        .and_then(Value::as_str)?
        .to_string();
    Some((range, text))
}

fn parse_completion_documentation(value: &Value) -> Option<String> {
    markup_to_markdown(value)
}

fn markup_to_markdown(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => non_empty_markdown(text),
        Value::Object(object) => object
            .get("value")
            .and_then(Value::as_str)
            .and_then(non_empty_markdown),
        _ => None,
    }
}

const fn parse_insert_text_format(value: u64) -> LspInsertTextFormat {
    if value == 2 {
        LspInsertTextFormat::Snippet
    } else {
        LspInsertTextFormat::PlainText
    }
}

fn parse_completion_item_kind(value: &Value) -> Option<LspCompletionItemKind> {
    let kind = value.as_u64()?;
    Some(match kind {
        1 => LspCompletionItemKind::Text,
        2 => LspCompletionItemKind::Method,
        3 => LspCompletionItemKind::Function,
        4 => LspCompletionItemKind::Constructor,
        5 => LspCompletionItemKind::Field,
        6 => LspCompletionItemKind::Variable,
        7 => LspCompletionItemKind::Class,
        8 => LspCompletionItemKind::Interface,
        9 => LspCompletionItemKind::Module,
        10 => LspCompletionItemKind::Property,
        11 => LspCompletionItemKind::Unit,
        12 => LspCompletionItemKind::Value,
        13 => LspCompletionItemKind::Enum,
        14 => LspCompletionItemKind::Keyword,
        15 => LspCompletionItemKind::Snippet,
        16 => LspCompletionItemKind::Color,
        17 => LspCompletionItemKind::File,
        18 => LspCompletionItemKind::Reference,
        19 => LspCompletionItemKind::Folder,
        20 => LspCompletionItemKind::EnumMember,
        21 => LspCompletionItemKind::Constant,
        22 => LspCompletionItemKind::Struct,
        23 => LspCompletionItemKind::Event,
        24 => LspCompletionItemKind::Operator,
        25 => LspCompletionItemKind::TypeParameter,
        _ => return None,
    })
}

fn lsp_range_from_value(value: &Value) -> Option<LspRange> {
    let start = value.get("start")?;
    let end = value.get("end")?;
    Some(LspRange {
        start_line: one_based_lsp_position_value(start, "line")?,
        start_column: one_based_lsp_position_value(start, "character")?,
        end_line: one_based_lsp_position_value(end, "line")?,
        end_column: one_based_lsp_position_value(end, "character")?,
    })
}

fn one_based_lsp_position_value(value: &Value, key: &str) -> Option<u32> {
    let raw = value.get(key)?.as_u64()?;
    Some(
        u32::try_from(raw)
            .unwrap_or(u32::MAX.saturating_sub(1))
            .saturating_add(1),
    )
}

fn value_to_u32(value: &Value) -> Option<u32> {
    u32::try_from(value.as_u64()?).ok()
}

fn markdown_strings_from_markup(value: &Value) -> Vec<String> {
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

fn non_empty_markdown(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn map_lsp_severity(severity: lsp_types::DiagnosticSeverity) -> DiagnosticSeverity {
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

fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
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
