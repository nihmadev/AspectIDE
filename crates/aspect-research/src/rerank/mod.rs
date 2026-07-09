//! Lexical reranking: score each fetched source against the query and order them.
//!
//! Perplexica reranks with embeddings; we use a dependency-free lexical model —
//! corpus-IDF-weighted term coverage (BM25-shaped: rare, discriminative terms count
//! more than ubiquitous ones) + saturated term frequency over title/snippet/content,
//! a title-hit boost, the provider's own score, and a small authority/junk signal.
//! Cited content is the most query-relevant *passage* of the page (not its head,
//! which is usually nav/boilerplate). Good enough to float the most on-topic
//! sources to the top for the agent to cite; an embedding path can slot in later
//! behind the same interface.

use std::collections::{HashMap, HashSet};

use crate::model::{MultiRankedSource, RankedSource, SearchHit};
use crate::provider::canonical_url_key;

mod passage;
mod util;

use util::{authority_signal, host_of, tokenize_unique};

const W_COVERAGE: f64 = 0.5;
const W_TF: f64 = 0.2;
const W_TITLE: f64 = 0.15;
const W_PROVIDER: f64 = 0.15;

/// Hard cap on characters fed to the *scorer*.
const MAX_SCORING_CHARS: usize = 16_384;
/// Deep-mode consensus: boost per extra sub-query that surfaced a URL, and its cap.
const CONSENSUS_STEP: f64 = 0.04;
const CONSENSUS_CAP: f64 = 0.12;
/// Multi-query consensus: boost per extra input query that surfaced a URL, and cap.
const MULTI_CONSENSUS_STEP: f64 = 0.05;
const MULTI_CONSENSUS_CAP: f64 = 0.15;

/// Rank `hits` (each paired with its extracted page `content`) against `query`.
/// Returns at most `max_sources`, 1-indexed by final relevance, with content
/// trimmed to the best query-relevant passage of `max_chars_per_source`.
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
    let docs = build_doc_indexes(hits, contents);
    let idf = idf_map(&query_terms, &docs);
    let provider_max = provider_max_of(hits);

    let mut scored: Vec<Scored<'_>> = hits
        .iter()
        .enumerate()
        .map(|(index, hit)| {
            let (base, matched) = score_query(
                &query_terms,
                &idf,
                &docs[index],
                provider_norm(hit, provider_max),
            );
            let score = (base + authority_signal(&hit.url)).clamp(0.0, 1.0);
            Scored {
                score,
                matched,
                hit,
                content: contents.get(index).map_or("", String::as_str),
            }
        })
        .filter(|scored| !has_query || scored.matched > 0)
        .collect();

    sort_scored(&mut scored);
    scored.truncate(max_sources.max(1));

    scored
        .into_iter()
        .enumerate()
        .map(|(index, s)| {
            to_ranked(
                index + 1,
                s.hit,
                s.content,
                s.score,
                max_chars_per_source,
                &query_terms,
            )
        })
        .collect()
}

/// Deep-mode rerank: like [`rerank`] but folds in cross-query consensus and
/// domain diversity.
#[must_use]
pub fn rerank_deep(
    query: &str,
    hits: &[SearchHit],
    contents: &[String],
    max_sources: usize,
    max_chars_per_source: usize,
    per_host_cap: usize,
    frequency: &HashMap<String, usize>,
) -> Vec<RankedSource> {
    let query_terms = tokenize_unique(query);
    let has_query = !query_terms.is_empty();
    let docs = build_doc_indexes(hits, contents);
    let idf = idf_map(&query_terms, &docs);
    let provider_max = provider_max_of(hits);

    let mut scored: Vec<Scored<'_>> = hits
        .iter()
        .enumerate()
        .map(|(index, hit)| {
            let (base, matched) = score_query(
                &query_terms,
                &idf,
                &docs[index],
                provider_norm(hit, provider_max),
            );
            let seen = frequency
                .get(&canonical_url_key(&hit.url))
                .copied()
                .unwrap_or(1)
                .max(1);
            let consensus = consensus_boost(seen, CONSENSUS_STEP, CONSENSUS_CAP);
            let score = (base + consensus + authority_signal(&hit.url)).clamp(0.0, 1.0);
            Scored {
                score,
                matched,
                hit,
                content: contents.get(index).map_or("", String::as_str),
            }
        })
        .filter(|scored| !has_query || scored.matched > 0)
        .collect();

    sort_scored(&mut scored);
    retain_domain_diverse(&mut scored, per_host_cap, |s| &s.hit.url);
    scored.truncate(max_sources.max(1));

    scored
        .into_iter()
        .enumerate()
        .map(|(index, s)| {
            to_ranked(
                index + 1,
                s.hit,
                s.content,
                s.score,
                max_chars_per_source,
                &query_terms,
            )
        })
        .collect()
}

/// Multi-query rerank for `MultiWebResearch`: every doc is scored against EVERY
/// input query and ranked by its *best* one.
#[must_use]
pub fn rerank_multi(
    queries: &[String],
    hits: &[SearchHit],
    contents: &[String],
    surfaced_by: &[Vec<usize>],
    max_sources: usize,
    max_chars_per_source: usize,
    per_host_cap: usize,
) -> Vec<MultiRankedSource> {
    let term_sets: Vec<HashSet<String>> =
        queries.iter().map(|query| tokenize_unique(query)).collect();
    let has_query = term_sets.iter().any(|terms| !terms.is_empty());
    let docs = build_doc_indexes(hits, contents);
    let idfs: Vec<HashMap<String, f64>> = term_sets
        .iter()
        .map(|terms| idf_map(terms, &docs))
        .collect();
    let provider_max = provider_max_of(hits);
    let empty_terms: HashSet<String> = HashSet::new();
    let empty_idf: HashMap<String, f64> = HashMap::new();

    struct MultiScored<'a> {
        score: f64,
        matched: usize,
        best_query: usize,
        index: usize,
        hit: &'a SearchHit,
        content: &'a str,
    }

    let mut scored: Vec<MultiScored<'_>> = hits
        .iter()
        .enumerate()
        .map(|(index, hit)| {
            let provider = provider_norm(hit, provider_max);
            let mut best_score = 0.0_f64;
            let mut best_query = 0_usize;
            let mut matched_any = 0_usize;
            if has_query {
                for (query_index, terms) in term_sets.iter().enumerate() {
                    if terms.is_empty() {
                        continue;
                    }
                    let (score, matched) =
                        score_query(terms, &idfs[query_index], &docs[index], provider);
                    matched_any = matched_any.max(matched);
                    if score > best_score {
                        best_score = score;
                        best_query = query_index;
                    }
                }
            } else {
                let (score, _) = score_query(&empty_terms, &empty_idf, &docs[index], provider);
                best_score = score;
            }
            let seen = surfaced_by
                .get(index)
                .map_or(1, |sources| sources.len().max(1));
            let consensus = consensus_boost(seen, MULTI_CONSENSUS_STEP, MULTI_CONSENSUS_CAP);
            let score = (best_score + consensus + authority_signal(&hit.url)).clamp(0.0, 1.0);
            MultiScored {
                score,
                matched: matched_any,
                best_query,
                index,
                hit,
                content: contents.get(index).map_or("", String::as_str),
            }
        })
        .filter(|scored| !has_query || scored.matched > 0)
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    retain_domain_diverse(&mut scored, per_host_cap, |s| &s.hit.url);
    scored.truncate(max_sources.max(1));

    scored
        .into_iter()
        .enumerate()
        .map(|(rank_index, s)| MultiRankedSource {
            source: to_ranked(
                rank_index + 1,
                s.hit,
                s.content,
                s.score,
                max_chars_per_source,
                term_sets.get(s.best_query).unwrap_or(&empty_terms),
            ),
            matched_queries: surfaced_by
                .get(s.index)
                .map(|sources| {
                    sources
                        .iter()
                        .filter_map(|&query_index| queries.get(query_index).cloned())
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}

// ── scoring core ──

struct Scored<'a> {
    score: f64,
    matched: usize,
    hit: &'a SearchHit,
    content: &'a str,
}

fn sort_scored(scored: &mut [Scored<'_>]) {
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn provider_max_of(hits: &[SearchHit]) -> f64 {
    hits.iter()
        .map(|hit| hit.provider_score)
        .fold(0.0_f64, f64::max)
}

fn provider_norm(hit: &SearchHit, provider_max: f64) -> f64 {
    if provider_max > 0.0 {
        hit.provider_score / provider_max
    } else {
        0.0
    }
}

/// `+step` per extra surfacing query beyond the first, capped.
fn consensus_boost(seen: usize, step: f64, cap: f64) -> f64 {
    let extra = u32::try_from(seen.saturating_sub(1)).unwrap_or(u32::MAX);
    (f64::from(extra) * step).min(cap)
}

/// Per-candidate token index built once per rerank.
struct DocIndex {
    counts: HashMap<String, usize>,
    title_terms: HashSet<String>,
}

fn build_doc_indexes(hits: &[SearchHit], contents: &[String]) -> Vec<DocIndex> {
    hits.iter()
        .enumerate()
        .map(|(index, hit)| {
            let content = contents.get(index).map_or("", String::as_str);
            let body = util::take_chars(content, MAX_SCORING_CHARS);
            let haystack = format!("{} {} {}", hit.title, hit.snippet, body).to_lowercase();
            let counts = util::term_counts(&haystack)
                .into_iter()
                .map(|(term, count)| (term.to_string(), count))
                .collect();
            let title_lower = hit.title.to_lowercase();
            let title_terms = util::term_counts(&title_lower)
                .into_keys()
                .map(str::to_string)
                .collect();
            DocIndex {
                counts,
                title_terms,
            }
        })
        .collect()
}

/// Corpus IDF for each query term, BM25-shaped.
fn idf_map(query_terms: &HashSet<String>, docs: &[DocIndex]) -> HashMap<String, f64> {
    let total = docs.len() as f64;
    query_terms
        .iter()
        .map(|term| {
            let df = docs
                .iter()
                .filter(|doc| doc.counts.contains_key(term))
                .count() as f64;
            (term.clone(), (1.0 + (total - df + 0.5) / (df + 0.5)).ln())
        })
        .collect()
}

/// Score one indexed document against one query.
fn score_query(
    query_terms: &HashSet<String>,
    idf: &HashMap<String, f64>,
    doc: &DocIndex,
    provider: f64,
) -> (f64, usize) {
    if query_terms.is_empty() {
        let score = if provider > 0.0 { provider } else { 0.5 };
        return (score, 0);
    }
    let idf_total: f64 = idf.values().sum();
    if idf_total <= f64::EPSILON {
        return (W_PROVIDER * provider, 0);
    }

    let mut matched = 0_usize;
    let mut coverage = 0.0_f64;
    let mut tf_saturated = 0.0_f64;
    let mut title = 0.0_f64;
    for term in query_terms {
        let weight = idf.get(term).copied().unwrap_or(0.0);
        if let Some(&count) = doc.counts.get(term) {
            matched += 1;
            coverage += weight;
            let tf = count as f64;
            tf_saturated += weight * (tf / (tf + 2.0));
        }
        if doc.title_terms.contains(term) {
            title += weight;
        }
    }

    let score = W_COVERAGE * (coverage / idf_total)
        + W_TF * (tf_saturated / idf_total)
        + W_TITLE * (title / idf_total)
        + W_PROVIDER * provider;
    (score, matched)
}

/// Keep at most `per_host_cap` of the highest-scoring entries per registrable host.
fn retain_domain_diverse<T>(scored: &mut Vec<T>, per_host_cap: usize, url_of: impl Fn(&T) -> &str) {
    let cap = per_host_cap.max(1);
    let mut per_host: HashMap<String, usize> = HashMap::new();
    scored.retain(|entry| {
        let host = host_of(url_of(entry));
        let count = per_host.entry(host).or_insert(0);
        *count += 1;
        *count <= cap
    });
}

/// Build a `RankedSource` from a scored hit.
fn to_ranked(
    rank: usize,
    hit: &SearchHit,
    content: &str,
    score: f64,
    max_chars: usize,
    query_terms: &HashSet<String>,
) -> RankedSource {
    RankedSource {
        rank,
        url: hit.url.clone(),
        title: hit.title.clone(),
        snippet: hit.snippet.clone(),
        content: passage::best_passage(content, query_terms, max_chars),
        relevance: (score * 1000.0).round() / 1000.0,
        engine: hit.engine.clone(),
        domain: host_of(&hit.url),
    }
}
