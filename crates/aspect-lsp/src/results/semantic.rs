use aspect_core::LspSemanticTokens;
use serde_json::Value;

use crate::types::{SemanticTokenLegend, TextDocumentSyncKind, CLIENT_SEMANTIC_TOKEN_MODIFIERS, CLIENT_SEMANTIC_TOKEN_TYPES};

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

pub fn parse_text_document_sync_kind(value: &Value) -> TextDocumentSyncKind {
    let sync = value
        .get("capabilities")
        .and_then(|caps| caps.get("textDocumentSync"));
    let change = match sync {
        Some(Value::Number(number)) => number.as_i64(),
        Some(Value::Object(_)) => sync
            .and_then(|sync| sync.get("change"))
            .and_then(Value::as_i64),
        _ => None,
    };
    match change {
        Some(0) => TextDocumentSyncKind::None,
        Some(2) => TextDocumentSyncKind::Incremental,
        _ => TextDocumentSyncKind::Full,
    }
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
