use aspect_core::{
    LspCompletionItem, LspCompletionItemKind, LspCompletionList, LspDocumentSymbol,
    LspFoldingRange, LspFoldingRangeKind, LspHover, LspInsertTextFormat, LspLocation, LspRange,
    LspSymbolKind, LspWorkspaceSymbol,
};
use lsp_types::Uri;
use serde_json::Value;

use super::common::{
    lsp_location_from_value, lsp_range_from_value, markdown_strings_from_markup,
    one_based_lsp_position_value, uri_to_path,
};

pub fn empty_completion_list() -> LspCompletionList {
    LspCompletionList {
        is_incomplete: false,
        items: Vec::new(),
    }
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

pub fn parse_folding_range_result(value: &Value) -> Vec<LspFoldingRange> {
    value
        .as_array()
        .map(|ranges| ranges.iter().filter_map(parse_folding_range_item).collect())
        .unwrap_or_default()
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
    super::common::markup_to_markdown(value)
}

fn parse_insert_text_format(value: u64) -> LspInsertTextFormat {
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

fn parse_folding_range_kind(value: &str) -> Option<LspFoldingRangeKind> {
    Some(match value {
        "comment" => LspFoldingRangeKind::Comment,
        "imports" => LspFoldingRangeKind::Imports,
        "region" => LspFoldingRangeKind::Region,
        _ => return None,
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
