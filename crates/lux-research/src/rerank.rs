//! Lexical reranking: score each fetched source against the query and order them.
//!
//! Perplexica reranks with embeddings; we use a dependency-free lexical model —
//! query-term coverage + frequency over title/snippet/content, with a title-hit
//! boost and the provider's own score folded in. Good enough to float the most
//! on-topic sources to the top for the agent to cite; the embedding path can slot
//! in later behind the same interface.

use std::collections::{HashMap, HashSet};

use crate::model::{RankedSource, SearchHit};

const W_COVERAGE: f64 = 0.55;
const W_FREQUENCY: f64 = 0.15;
const W_TITLE: f64 = 0.15;
const W_PROVIDER: f64 = 0.15;

/// Hard cap on characters fed to the *scorer* (independent of the citation trim,
/// which runs later). Without this, a single multi-MB or adversarial page would
/// be lowercased + tokenized in full, burning CPU/memory in the AI turn loop
/// before `max_chars_per_source` ever applies.
const MAX_SCORING_CHARS: usize = 16_384;
/// Cap on distinct query terms actually scored, so a pathologically long query
/// can't blow up the per-hit term loop.
const MAX_QUERY_TERMS: usize = 64;

/// Rank `hits` (each paired with its extracted page `content`) against `query`.
/// Returns at most `max_sources`, 1-indexed by final relevance, with content
/// trimmed to `max_chars_per_source`. Pairing is positional: `contents[i]`
/// belongs to `hits[i]` (empty string when the page fetch yielded nothing).
#[must_use]
pub fn rerank(
    query: &str,
    hits: &[SearchHit],
    contents: &[String],
    max_sources: usize,
    max_chars_per_source: usize,
) -> Vec<RankedSource> {
    let query_terms = tokenize_unique(query);
    let has_query = !query_terms.is_empty();
    let provider_max = hits
        .iter()
        .map(|hit| hit.provider_score)
        .fold(0.0_f64, f64::max);

    let mut scored: Vec<(f64, usize, &SearchHit, &str)> = hits
        .iter()
        .enumerate()
        .map(|(index, hit)| {
            let content = contents.get(index).map_or("", String::as_str);
            let (score, matched) = score_hit(&query_terms, hit, content, provider_max);
            (score, matched, hit, content)
        })
        // For a real query, a source that matches NONE of its terms is noise:
        // citing it actively misleads the model and user, so drop it before
        // truncation. Token-free queries keep provider-order fallback.
        .filter(|(_, matched, _, _)| !has_query || *matched > 0)
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(max_sources.max(1));

    scored
        .into_iter()
        .enumerate()
        .map(|(index, (score, _matched, hit, content))| RankedSource {
            rank: index + 1,
            url: hit.url.clone(),
            title: hit.title.clone(),
            snippet: hit.snippet.clone(),
            content: trim_to_chars(content, max_chars_per_source),
            relevance: (score * 1000.0).round() / 1000.0,
            engine: hit.engine.clone(),
        })
        .collect()
}

/// Score one hit against the query. Returns `(blended_score, matched_term_count)`
/// so the caller can both rank and drop zero-coverage sources. Scoring is bounded
/// (capped input, single-pass term counting) to stay cheap on oversized pages.
fn score_hit(
    query_terms: &HashSet<String>,
    hit: &SearchHit,
    content: &str,
    provider_max: f64,
) -> (f64, usize) {
    let provider = if provider_max > 0.0 {
        hit.provider_score / provider_max
    } else {
        0.0
    };

    if query_terms.is_empty() {
        // No usable query terms — fall back to the provider's own ordering.
        let score = if provider_max > 0.0 { provider } else { 0.5 };
        return (score, 0);
    }

    // Cap BEFORE tokenizing: never lowercase/scan more than MAX_SCORING_CHARS of
    // body text, regardless of how large the fetched page is.
    let body = take_chars(content, MAX_SCORING_CHARS);
    let haystack = format!("{} {} {}", hit.title, hit.snippet, body).to_lowercase();
    // One pass over the document → term→count map; lookups are then O(1) per
    // query term instead of re-scanning the whole token list each time.
    let doc_counts = term_counts(&haystack);

    let mut matched = 0_usize;
    let mut occurrences = 0_usize;
    for term in query_terms {
        if let Some(count) = doc_counts.get(term.as_str()) {
            matched += 1;
            occurrences += *count;
        }
    }
    let coverage = matched as f64 / query_terms.len() as f64;
    // Frequency, saturating so a keyword-stuffed page can't dominate.
    let frequency = 1.0 - (1.0 / (1.0 + occurrences as f64));

    let title_lower = hit.title.to_lowercase();
    let title_counts = term_counts(&title_lower);
    let title_hits = query_terms
        .iter()
        .filter(|term| title_counts.contains_key(term.as_str()))
        .count();
    let title = title_hits as f64 / query_terms.len() as f64;

    let score =
        W_COVERAGE * coverage + W_FREQUENCY * frequency + W_TITLE * title + W_PROVIDER * provider;
    (score, matched)
}

/// Take at most `max_chars` characters from `text` on a char boundary (cheap, no
/// allocation when the text already fits).
fn take_chars(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((byte_index, _)) => &text[..byte_index],
        None => text,
    }
}

/// Count occurrences of each ≥2-char alphanumeric token in one pass.
fn term_counts(text: &str) -> HashMap<&str, usize> {
    let mut counts = HashMap::new();
    for token in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
    {
        *counts.entry(token).or_insert(0) += 1;
    }
    counts
}

fn tokenize_unique(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
        .map(str::to_string)
        .take(MAX_QUERY_TERMS)
        .collect()
}

/// Trim to at most `max_chars` on a word boundary (so a citation snippet doesn't
/// end mid-word), appending an ellipsis when truncated.
fn trim_to_chars(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut end = 0;
    for (count, (byte_index, _)) in trimmed.char_indices().enumerate() {
        if count >= max_chars {
            break;
        }
        end = byte_index;
    }
    let cut = trimmed[..end].rfind(char::is_whitespace).unwrap_or(end);
    format!("{}…", trimmed[..cut].trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(url: &str, title: &str, snippet: &str) -> SearchHit {
        SearchHit {
            url: url.to_string(),
            title: title.to_string(),
            snippet: snippet.to_string(),
            engine: "test".to_string(),
            provider_score: 0.0,
        }
    }

    #[test]
    fn ranks_more_relevant_source_first() {
        let hits = vec![
            hit("https://off.com", "Cooking pasta", "recipes and food"),
            hit(
                "https://on.com",
                "Rust async runtime",
                "tokio and async tasks in rust",
            ),
        ];
        let contents = vec![String::new(), "rust async tokio futures".to_string()];
        let ranked = rerank("rust async tokio", &hits, &contents, 5, 500);
        // The on-topic source ranks first; the off-topic one matches no query
        // term and is dropped rather than cited.
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].url, "https://on.com");
        assert_eq!(ranked[0].rank, 1);
    }

    #[test]
    fn drops_sources_with_no_query_term_coverage() {
        let hits = vec![
            hit("https://match.com", "rust guide", "all about rust"),
            hit("https://nomatch.com", "cooking", "pasta recipes"),
        ];
        let contents = vec![String::new(), String::new()];
        let ranked = rerank("rust", &hits, &contents, 5, 200);
        assert_eq!(ranked.len(), 1, "zero-coverage source must be dropped");
        assert_eq!(ranked[0].url, "https://match.com");
    }

    #[test]
    fn scoring_is_bounded_on_oversized_content() {
        // A multi-megabyte page must still score quickly and match correctly:
        // the term appears within the scoring cap.
        let big = format!("rust {}", "x ".repeat(2_000_000));
        let hits = vec![hit("https://big.com", "rust", "rust")];
        let ranked = rerank("rust", &hits, &[big], 1, 100);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].url, "https://big.com");
    }

    #[test]
    fn respects_max_sources_and_indexes_from_one() {
        let hits = vec![hit("a", "a", "a"), hit("b", "b", "b"), hit("c", "c", "c")];
        let contents = vec![String::new(), String::new(), String::new()];
        let ranked = rerank("a", &hits, &contents, 2, 100);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].rank, 1);
        assert_eq!(ranked[1].rank, 2);
    }

    #[test]
    fn trims_content_on_word_boundary() {
        let hits = vec![hit("u", "t", "s")];
        let long = "alpha beta gamma delta epsilon zeta eta theta".to_string();
        let ranked = rerank("alpha", &hits, &[long], 1, 12);
        assert!(ranked[0].content.ends_with('…'));
        assert!(ranked[0].content.chars().count() <= 14);
        assert!(!ranked[0].content.contains("epsilon"));
    }

    #[test]
    fn empty_query_falls_back_without_panicking() {
        let hits = vec![hit("a", "a", "a")];
        let ranked = rerank("", &hits, &[String::new()], 5, 100);
        assert_eq!(ranked.len(), 1);
    }
}
