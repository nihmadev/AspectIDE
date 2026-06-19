//! Lexical reranking: score each fetched source against the query and order them.
//!
//! Perplexica reranks with embeddings; we use a dependency-free lexical model —
//! query-term coverage + frequency over title/snippet/content, with a title-hit
//! boost and the provider's own score folded in. Good enough to float the most
//! on-topic sources to the top for the agent to cite; the embedding path can slot
//! in later behind the same interface.

use std::collections::HashSet;

use crate::model::{RankedSource, SearchHit};

const W_COVERAGE: f64 = 0.55;
const W_FREQUENCY: f64 = 0.15;
const W_TITLE: f64 = 0.15;
const W_PROVIDER: f64 = 0.15;

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
    let provider_max = hits
        .iter()
        .map(|hit| hit.provider_score)
        .fold(0.0_f64, f64::max);

    let mut scored: Vec<(f64, &SearchHit, &str)> = hits
        .iter()
        .enumerate()
        .map(|(index, hit)| {
            let content = contents.get(index).map_or("", String::as_str);
            let score = score_hit(&query_terms, hit, content, provider_max);
            (score, hit, content)
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(max_sources.max(1));

    scored
        .into_iter()
        .enumerate()
        .map(|(index, (score, hit, content))| RankedSource {
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

fn score_hit(
    query_terms: &HashSet<String>,
    hit: &SearchHit,
    content: &str,
    provider_max: f64,
) -> f64 {
    if query_terms.is_empty() {
        // No usable query terms — fall back to the provider's own ordering.
        return if provider_max > 0.0 {
            hit.provider_score / provider_max
        } else {
            0.5
        };
    }
    let haystack = format!("{} {} {}", hit.title, hit.snippet, content).to_lowercase();
    let doc_terms = tokenize_all(&haystack);

    let matched = query_terms
        .iter()
        .filter(|term| doc_terms.contains(*term))
        .count();
    let coverage = matched as f64 / query_terms.len() as f64;

    // Frequency, saturating so a keyword-stuffed page can't dominate.
    let occurrences: usize = query_terms
        .iter()
        .map(|term| doc_terms.iter().filter(|word| *word == term).count())
        .sum();
    let frequency = 1.0 - (1.0 / (1.0 + occurrences as f64));

    let title_terms = tokenize_all(&hit.title.to_lowercase());
    let title_hits = query_terms
        .iter()
        .filter(|term| title_terms.contains(*term))
        .count();
    let title = if query_terms.is_empty() {
        0.0
    } else {
        title_hits as f64 / query_terms.len() as f64
    };

    let provider = if provider_max > 0.0 {
        hit.provider_score / provider_max
    } else {
        0.0
    };

    W_COVERAGE * coverage + W_FREQUENCY * frequency + W_TITLE * title + W_PROVIDER * provider
}

fn tokenize_all(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect()
}

fn tokenize_unique(text: &str) -> HashSet<String> {
    tokenize_all(&text.to_lowercase()).into_iter().collect()
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
        assert_eq!(ranked[0].url, "https://on.com");
        assert_eq!(ranked[0].rank, 1);
        assert!(ranked[0].relevance > ranked[1].relevance);
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
