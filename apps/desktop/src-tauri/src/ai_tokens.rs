//! Token estimation and context-budget utilities — Stage 1 of TS→Rust migration.
//!
//! Core building block used by compaction, context-usage meter, and context-budget
//! ranking. Faithfully ports the TS `estimateTokens(value: string)` heuristic
//! (`ceil(trimmed_len / 4)`) so token estimates match the existing behavior.

use serde::Serialize;

/// Estimate tokens from text length (same heuristic as the TS `estimateTokens`).
#[must_use]
pub fn estimate_tokens(value: &str) -> usize {
    let trimmed_len = value.trim().len();
    if trimmed_len == 0 {
        return 0;
    }
    (trimmed_len + 3) / 4
}

/// Estimate tokens for a message (content + reasoning + tool calls).
#[must_use]
pub fn estimate_message_tokens(content: &str, reasoning: &str, tool_calls: &[ToolCallEstimate]) -> usize {
    let content_tokens = estimate_tokens(content);
    let reasoning_tokens = estimate_tokens(reasoning);
    let tool_tokens: usize = tool_calls.iter().map(|call| {
        estimate_tokens(&call.tool)
            + estimate_tokens(&call.input)
            + estimate_tokens(&call.output)
            + estimate_tokens(&call.error)
    }).sum();
    content_tokens + reasoning_tokens + tool_tokens
}

/// Minimal tool-call shape for token estimation.
pub struct ToolCallEstimate {
    pub tool: String,
    pub input: String,
    pub output: String,
    pub error: String,
}

/// Format compact token counts like `1.2K`, `3.5M`, matching TS `formatCompactTokens`.
#[must_use]
pub fn format_compact_tokens(tokens: usize) -> String {
    if tokens >= 1_000_000 {
        let value = tokens as f64 / 1_000_000.0;
        if tokens >= 10_000_000 { format!("{}M", value as u64) }
        else { format!("{:.1}M", value) }
    } else if tokens >= 1_000 {
        let value = tokens as f64 / 1_000.0;
        if tokens >= 10_000 { format!("{}K", value as u64) }
        else { format!("{:.1}K", value) }
    } else {
        tokens.to_string()
    }
}

/// Context-budget auto-compact: should we trigger compaction?
#[must_use]
pub fn should_compact(total_tokens: usize, trigger_tokens: usize, min_messages: usize, message_count: usize, auto_enabled: bool) -> bool {
    if !auto_enabled { return false; }
    if message_count < min_messages { return false; }
    total_tokens >= trigger_tokens
}

// ── Model context resolution (ports aiModelContext.ts) ──

const DEFAULT_MODEL_CONTEXT_TOKENS: usize = 200_000;
const MIN_MODEL_CONTEXT_TOKENS: usize = 8_000;
const MAX_MODEL_CONTEXT_TOKENS: usize = 2_000_000;
const DEFAULT_AUTO_COMPACT_THRESHOLD: f64 = 0.8;
const MIN_AUTO_COMPACT_THRESHOLD: f64 = 0.5;
const MAX_AUTO_COMPACT_THRESHOLD: f64 = 0.95;

const MODEL_CONTEXT_HINTS: &[(&str, usize)] = &[
    ("gpt-5.5", 400_000), ("gpt-5-pro", 400_000), ("gpt-5-mini", 400_000), ("gpt-5-nano", 400_000),
    ("gpt-4.1", 128_000), ("gpt-4o", 128_000), ("o3", 128_000), ("o4", 128_000),
    ("claude-opus-4", 200_000), ("claude-sonnet-4", 200_000), ("claude-3-7", 200_000), ("claude-3-5", 200_000),
    ("claude-haiku", 200_000),
    ("gemini-2.5-pro", 1_048_576), ("gemini-2.5-flash", 1_048_576), ("gemini-2.0", 1_048_576), ("gemini-1.5-pro", 1_048_576),
    ("gemini", 128_000),
    ("deepseek", 128_000),
    ("mistral-large", 128_000), ("codestral", 128_000),
    ("llama-3.3", 128_000), ("llama3", 128_000),
    ("qwen", 128_000),
];

/// Infer model context tokens from a model alias/id string.
#[must_use]
pub fn infer_context_tokens(model_ref: &str) -> Option<usize> {
    let haystack = model_ref.trim().to_lowercase();
    if haystack.is_empty() { return None; }
    MODEL_CONTEXT_HINTS.iter().find(|(pattern, _)| haystack.contains(pattern)).map(|(_, tokens)| *tokens)
}

/// Resolve the effective context-token budget for a model.
#[must_use]
pub fn resolve_model_context_tokens(model_alias: &str, explicit_tokens: Option<usize>) -> usize {
    if let Some(explicit) = explicit_tokens {
        if explicit > 0 {
            return explicit.clamp(MIN_MODEL_CONTEXT_TOKENS, MAX_MODEL_CONTEXT_TOKENS);
        }
    }
    infer_context_tokens(model_alias).unwrap_or(DEFAULT_MODEL_CONTEXT_TOKENS)
}

/// Clamp auto-compact threshold to the valid range.
#[must_use]
pub fn clamp_compact_threshold(value: f64) -> f64 {
    if !value.is_finite() { return DEFAULT_AUTO_COMPACT_THRESHOLD; }
    value.clamp(MIN_AUTO_COMPACT_THRESHOLD, MAX_AUTO_COMPACT_THRESHOLD)
}

/// Resolve the token count that triggers auto-compaction.
#[must_use]
pub fn compact_trigger_tokens(model_alias: &str, explicit_tokens: Option<usize>, threshold: f64) -> usize {
    let budget = resolve_model_context_tokens(model_alias, explicit_tokens);
    let ratio = clamp_compact_threshold(threshold);
    (budget as f64 * ratio) as usize
}

/// Tauri command: estimate tokens for a text string (exposed for TS context-usage meter).
#[tauri::command]
pub fn ai_estimate_tokens(text: String) -> usize {
    estimate_tokens(&text)
}

/// Tauri command: format a token count compactly.
#[tauri::command]
pub fn ai_format_tokens(tokens: usize) -> String {
    format_compact_tokens(tokens)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenEstimateBatch {
    pub estimates: Vec<usize>,
    pub total: usize,
}

/// Tauri command: batch-estimate tokens for multiple texts at once (avoids N IPC calls).
#[tauri::command]
pub fn ai_estimate_tokens_batch(texts: Vec<String>) -> TokenEstimateBatch {
    let estimates: Vec<usize> = texts.iter().map(|t| estimate_tokens(t)).collect();
    let total = estimates.iter().sum();
    TokenEstimateBatch { estimates, total }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_empty_and_whitespace() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("   "), 0);
    }

    #[test]
    fn estimate_short_text() {
        assert_eq!(estimate_tokens("ab"), 1);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }

    #[test]
    fn estimate_matches_ts_heuristic() {
        // TS: Math.ceil(trimmed.length / 4)
        let text = "Hello, world! This is a test.";
        let expected = (text.trim().len() + 3) / 4;
        assert_eq!(estimate_tokens(text), expected);
    }

    #[test]
    fn format_compact_small() {
        assert_eq!(format_compact_tokens(42), "42");
        assert_eq!(format_compact_tokens(999), "999");
    }

    #[test]
    fn format_compact_thousands() {
        assert_eq!(format_compact_tokens(1_500), "1.5K");
        assert_eq!(format_compact_tokens(12_000), "12K");
    }

    #[test]
    fn format_compact_millions() {
        assert_eq!(format_compact_tokens(1_200_000), "1.2M");
        assert_eq!(format_compact_tokens(15_000_000), "15M");
    }

    #[test]
    fn should_compact_thresholds() {
        assert!(!should_compact(5000, 10000, 8, 10, true));
        assert!(should_compact(12000, 10000, 8, 10, true));
        assert!(!should_compact(12000, 10000, 8, 10, false));
        assert!(!should_compact(12000, 10000, 8, 5, true));
    }

    #[test]
    fn infer_context_tokens_from_model_ref() {
        assert_eq!(infer_context_tokens("claude-sonnet-4-6"), Some(200_000));
        assert_eq!(infer_context_tokens("gpt-5.5-turbo"), Some(400_000));
        assert_eq!(infer_context_tokens("gemini-2.5-pro"), Some(1_048_576));
        assert_eq!(infer_context_tokens("unknown-model"), None);
    }

    #[test]
    fn resolve_model_context_explicit_overrides() {
        assert_eq!(resolve_model_context_tokens("claude-opus-4", Some(500_000)), 500_000);
        assert_eq!(resolve_model_context_tokens("claude-opus-4", None), 200_000);
        assert_eq!(resolve_model_context_tokens("unknown", None), DEFAULT_MODEL_CONTEXT_TOKENS);
    }

    #[test]
    fn compact_trigger_calculation() {
        let trigger = compact_trigger_tokens("claude-sonnet-4-6", None, 0.8);
        assert_eq!(trigger, 160_000); // 200K * 0.8
    }
}
