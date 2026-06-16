//! Token estimation utilities — Stage 1 of TS→Rust migration.
//!
//! Core building block used by the context-usage meter. Faithfully ports the TS
//! `estimateTokens(value: string)` heuristic (`ceil(trimmed_len / 4)`) so token
//! estimates match the existing behavior.

use serde::Serialize;

/// Estimate tokens from text length (same heuristic as the TS `estimateTokens`).
#[must_use]
pub fn estimate_tokens(value: &str) -> usize {
    // Match JS `String.length` (UTF-16 code units), not UTF-8 byte length, so
    // non-ASCII input (CJK, emoji, accented Latin) yields the same estimate as TS.
    let trimmed_len = value.trim().encode_utf16().count();
    if trimmed_len == 0 {
        return 0;
    }
    trimmed_len.div_ceil(4)
}

/// Format compact token counts like `1.2K`, `3.5M`, matching TS `formatCompactTokens`.
#[must_use]
pub fn format_compact_tokens(tokens: usize) -> String {
    if tokens >= 1_000_000 {
        if tokens >= 10_000_000 {
            format!("{}M", tokens / 1_000_000)
        } else {
            let value = f64::from(u32::try_from(tokens).unwrap_or(u32::MAX)) / 1_000_000.0;
            format!("{value:.1}M")
        }
    } else if tokens >= 1_000 {
        if tokens >= 10_000 {
            format!("{}K", tokens / 1_000)
        } else {
            let value = f64::from(u32::try_from(tokens).unwrap_or(u32::MAX)) / 1_000.0;
            format!("{value:.1}K")
        }
    } else {
        tokens.to_string()
    }
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
        let expected = text.trim().len().div_ceil(4);
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
}
