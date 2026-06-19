//! Pure scoring helpers for memory retrieval: FTS5 query construction, recency
//! decay, cosine similarity, and the blend that combines lexical relevance,
//! importance, recency, and a pinned boost into a single rank.

/// Relative weights of the blended score. They need not sum to 1; the pinned
/// boost is additive on top so a pinned memory always outranks an unpinned one
/// of equal relevance.
const W_LEXICAL: f64 = 0.60;
const W_IMPORTANCE: f64 = 0.25;
const W_RECENCY: f64 = 0.15;
const PINNED_BOOST: f64 = 0.5;
const MILLIS_PER_DAY: f64 = 86_400_000.0;

/// Build a safe FTS5 `MATCH` expression from free user text: lowercase alnum
/// tokens (length ≥ 2), each quoted and prefix-matched, OR-joined. Returns
/// `None` when the query has no usable tokens (caller should fall back to a
/// plain listing).
pub fn fts_query(raw: &str) -> Option<String> {
    let tokens: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect();
    if tokens.is_empty() {
        return None;
    }
    // Quote each token (stripping any stray quotes) and prefix-match it so partial
    // words still hit; OR-join so any token can satisfy the match.
    let parts: Vec<String> = tokens
        .iter()
        .map(|token| format!("\"{}\"*", token.replace('"', "")))
        .collect();
    Some(parts.join(" OR "))
}

/// Recency weight in `(0, 1]`: 1.0 for "just now", halving every `half_life_days`.
#[must_use]
pub fn recency_decay(age_millis: i64, half_life_days: f64) -> f64 {
    if half_life_days <= 0.0 {
        return 1.0;
    }
    let age_days = (age_millis.max(0) as f64) / MILLIS_PER_DAY;
    0.5_f64.powf(age_days / half_life_days)
}

/// Min-max normalize a slice into `[0, 1]`. When all values are equal (or the
/// slice is degenerate) every entry maps to `1.0` so a uniform candidate set is
/// not zeroed out.
#[must_use]
pub fn min_max_normalize(values: &[f64]) -> Vec<f64> {
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = max - min;
    if !span.is_finite() || span <= f64::EPSILON {
        return vec![1.0; values.len()];
    }
    values.iter().map(|value| (value - min) / span).collect()
}

/// Blend the sub-scores into a final rank.
#[must_use]
pub fn blend(lexical: f64, importance: f64, recency: f64, pinned: bool) -> f64 {
    let base =
        W_LEXICAL * lexical + W_IMPORTANCE * importance.clamp(0.0, 1.0) + W_RECENCY * recency;
    if pinned {
        base + PINNED_BOOST
    } else {
        base
    }
}

/// Cosine similarity in `[-1, 1]`; `0.0` when either vector is empty, the lengths
/// differ, or a vector has zero magnitude.
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

/// Encode an `f32` embedding as little-endian bytes for BLOB storage.
#[must_use]
pub fn encode_embedding(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Decode a little-endian BLOB back into an `f32` embedding; trailing partial
/// bytes (corrupt/legacy rows) are ignored.
#[must_use]
pub fn decode_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_query_tokenizes_and_prefixes() {
        assert_eq!(
            fts_query("Hello, world!"),
            Some("\"hello\"* OR \"world\"*".to_string())
        );
        assert_eq!(fts_query("a — !"), None);
        assert_eq!(fts_query(""), None);
    }

    #[test]
    fn recency_decay_halves_at_half_life() {
        let day = 86_400_000;
        assert!((recency_decay(0, 30.0) - 1.0).abs() < 1e-9);
        assert!((recency_decay(30 * day, 30.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn min_max_handles_uniform_input() {
        assert_eq!(min_max_normalize(&[5.0, 5.0, 5.0]), vec![1.0, 1.0, 1.0]);
        assert_eq!(min_max_normalize(&[0.0, 10.0]), vec![0.0, 1.0]);
    }

    #[test]
    fn pinned_outranks_equal_unpinned() {
        assert!(blend(0.5, 0.5, 0.5, true) > blend(0.5, 0.5, 0.5, false));
    }

    #[test]
    fn cosine_roundtrip_and_edges() {
        let v = vec![1.0_f32, 2.0, 3.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
        assert_eq!(cosine_similarity(&v, &[]), 0.0);
        assert_eq!(decode_embedding(&encode_embedding(&v)), v);
    }
}
