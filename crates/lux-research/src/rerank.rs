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

const W_COVERAGE: f64 = 0.5;
const W_TF: f64 = 0.2;
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
/// Cap on characters scanned when selecting the best citation passage.
const MAX_PASSAGE_SCAN_CHARS: usize = 120_000;
/// Deep-mode consensus: boost per extra sub-query that surfaced a URL, and its cap.
const CONSENSUS_STEP: f64 = 0.04;
const CONSENSUS_CAP: f64 = 0.12;
/// Multi-query consensus: boost per extra input query that surfaced a URL, and cap.
/// Larger than deep's because independent user-facet queries agreeing is a much
/// stronger signal than mechanical expansions of one query agreeing.
const MULTI_CONSENSUS_STEP: f64 = 0.05;
const MULTI_CONSENSUS_CAP: f64 = 0.15;

/// Rank `hits` (each paired with its extracted page `content`) against `query`.
/// Returns at most `max_sources`, 1-indexed by final relevance, with content
/// trimmed to the best query-relevant passage of `max_chars_per_source`. Pairing
/// is positional: `contents[i]` belongs to `hits[i]` (empty string when the page
/// fetch yielded nothing).
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
        // For a real query, a source that matches NONE of its terms is noise:
        // citing it actively misleads the model and user, so drop it before
        // truncation. Token-free queries keep provider-order fallback.
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

/// Deep-mode rerank: like [`rerank`] but folds in a cross-query *consensus* signal
/// (a source surfaced by several expanded sub-queries is more trustworthy), then
/// enforces **domain diversity** — at most `per_host_cap` sources per registrable
/// host — so the result spans sites instead of clustering on one. `frequency` maps
/// a hit's canonical URL key ([`canonical_url_key`]) to how many sub-queries
/// surfaced it (absent/0 → 1). Zero-coverage sources are still dropped.
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
/// input query and ranked by its *best* one (a page needn't cover all facets),
/// with a cross-query consensus boost when several independent queries surfaced
/// the same URL, the authority signal, and domain diversity. `surfaced_by[i]`
/// lists the indices (into `queries`) of the queries whose search surfaced
/// `hits[i]`; it also becomes the per-source `matchedQueries` attribution. A doc
/// matching no query at all is dropped (unless every query tokenized to nothing,
/// which falls back to provider order).
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
    // For the all-queries-tokenized-to-nothing fallback: `score_query` with empty
    // terms never reads the IDF map, so a shared empty one avoids indexing
    // `idfs[0]` (which panics when `queries` itself is empty).
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
                    // An empty term set's fallback score (0.5) must never outrank a
                    // real lexical match, so skip it when any query has terms.
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

/// Per-candidate token index built once per rerank: term → count over
/// title+snippet+capped body, plus the title's own term set. Owned so corpus-level
/// document frequency (IDF) can be computed across all candidates and every query
/// (in the multi path) scores against the same index without re-tokenizing.
struct DocIndex {
    counts: HashMap<String, usize>,
    title_terms: HashSet<String>,
}

fn build_doc_indexes(hits: &[SearchHit], contents: &[String]) -> Vec<DocIndex> {
    hits.iter()
        .enumerate()
        .map(|(index, hit)| {
            let content = contents.get(index).map_or("", String::as_str);
            // Cap BEFORE tokenizing: never lowercase/scan more than
            // MAX_SCORING_CHARS of body text, regardless of page size.
            let body = take_chars(content, MAX_SCORING_CHARS);
            let haystack = format!("{} {} {}", hit.title, hit.snippet, body).to_lowercase();
            let counts = term_counts(&haystack)
                .into_iter()
                .map(|(term, count)| (term.to_string(), count))
                .collect();
            let title_lower = hit.title.to_lowercase();
            let title_terms = term_counts(&title_lower)
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

/// Corpus IDF for each query term, BM25-shaped: `ln(1 + (N - df + 0.5)/(df + 0.5))`.
/// A term present in every candidate barely discriminates; a term only one source
/// contains is what actually separates on-topic from off-topic.
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

/// Score one indexed document against one query. Returns `(lexical_score,
/// matched_term_count)` so callers can both rank and drop zero-coverage sources.
fn score_query(
    query_terms: &HashSet<String>,
    idf: &HashMap<String, f64>,
    doc: &DocIndex,
    provider: f64,
) -> (f64, usize) {
    if query_terms.is_empty() {
        // No usable query terms — fall back to the provider's own ordering.
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
            // Saturating tf so a keyword-stuffed page can't dominate.
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

/// Keep at most `per_host_cap` of the highest-scoring entries per registrable
/// host, preserving sorted order.
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

/// Build a `RankedSource` from a scored hit (shared by all rerank paths). Cites the
/// best query-relevant passage of the content, not its head.
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
        content: best_passage(content, query_terms, max_chars),
        relevance: (score * 1000.0).round() / 1000.0,
        engine: hit.engine.clone(),
        domain: host_of(&hit.url),
    }
}

/// Registrable-ish host of a URL: the authority between `scheme://` and the first
/// `/`, `?`, or `#`, with any `user@` and `:port` stripped. Empty when unparseable.
#[must_use]
fn host_of(url: &str) -> String {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    let host = authority.rsplit('@').next().unwrap_or(authority);
    // Strip a :port (but keep IPv6 brackets intact).
    let host = if host.starts_with('[') {
        host.split(']').next().map_or(host, |h| &h[1..])
    } else {
        host.split(':').next().unwrap_or(host)
    };
    host.to_ascii_lowercase()
}

/// A small, capped authority/junk signal so official docs and known-good hosts
/// edge out content farms at equal lexical relevance — and known low-signal
/// aggregators sink. Deliberately bounded (`-0.08..=0.06`) so the lexical signal
/// always dominates.
fn authority_signal(url: &str) -> f64 {
    const TRUSTED: &[&str] = &[
        "docs.rs",
        "doc.rust-lang.org",
        "rust-lang.org",
        "developer.mozilla.org",
        "wikipedia.org",
        "github.com",
        "stackoverflow.com",
        "arxiv.org",
        "docs.python.org",
        "python.org",
        "nodejs.org",
        "go.dev",
        "learn.microsoft.com",
        "developer.apple.com",
        "kubernetes.io",
        "postgresql.org",
        "ietf.org",
        "rfc-editor.org",
        "w3.org",
        "whatwg.org",
        "crates.io",
        "npmjs.com",
        "pypi.org",
    ];
    /// Low-signal aggregators/content farms that outrank primary sources on SEO
    /// alone; a small penalty, not a ban — strong lexical relevance still wins.
    const JUNK: &[&str] = &[
        "pinterest.com",
        "scribd.com",
        "slideshare.net",
        "coursehero.com",
        "chegg.com",
        "quizlet.com",
        "brainly.com",
        "studocu.com",
    ];
    let host = host_of(url);
    let in_list = |list: &[&str]| {
        list.iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
    };

    let mut signal: f64 = 0.0;
    if url.starts_with("https://") {
        signal += 0.01;
    }
    if in_list(TRUSTED) {
        signal += 0.04;
    }
    if in_list(JUNK) {
        signal -= 0.08;
    }
    // Primary-source-shaped path: official docs/reference/RFC URLs edge out mirrors.
    let path = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.find('/').map(|slash| &rest[slash..]))
        .unwrap_or("");
    let path_lower = path.to_ascii_lowercase();
    if ["/docs", "/documentation", "/reference", "/manual", "/rfc"]
        .iter()
        .any(|prefix| path_lower.starts_with(prefix))
    {
        signal += 0.02;
    }
    signal.clamp(-0.08, 0.06)
}

// ── passage selection ──

/// Pick the most query-relevant passage of `content` (up to `max_chars`) instead
/// of blindly taking the head of the page — page heads are usually navigation and
/// boilerplate, while the sentence that actually answers the query sits mid-page.
/// Splits into paragraph/sentence chunks, scores each by query-term hits, and
/// grows a window around the best chunk. Falls back to the head when nothing
/// matches (or the query has no terms).
fn best_passage(content: &str, query_terms: &HashSet<String>, max_chars: usize) -> String {
    let trimmed = content.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    if query_terms.is_empty() {
        return trim_to_chars(trimmed, max_chars);
    }
    let scan = take_chars(trimmed, MAX_PASSAGE_SCAN_CHARS);
    let chunks = split_chunks(scan);
    if chunks.is_empty() {
        return trim_to_chars(trimmed, max_chars);
    }
    let scores: Vec<usize> = chunks
        .iter()
        .map(|chunk| chunk_score(chunk, query_terms))
        .collect();
    // Earliest best-scoring chunk anchors the window.
    let best = scores
        .iter()
        .enumerate()
        .max_by_key(|(index, score)| (**score, std::cmp::Reverse(*index)))
        .map_or(0, |(index, _)| index);
    if scores[best] == 0 {
        return trim_to_chars(trimmed, max_chars);
    }

    let mut start = best;
    let mut end = best;
    let mut total = chunks[best].chars().count();
    loop {
        let can_next =
            end + 1 < chunks.len() && total + 1 + chunks[end + 1].chars().count() <= max_chars;
        let can_prev = start > 0 && total + 1 + chunks[start - 1].chars().count() <= max_chars;
        match (can_next, can_prev) {
            (true, true) => {
                // Grow toward the higher-scoring neighbor; ties extend forward.
                if scores[end + 1] >= scores[start - 1] {
                    end += 1;
                    total += 1 + chunks[end].chars().count();
                } else {
                    start -= 1;
                    total += 1 + chunks[start].chars().count();
                }
            }
            (true, false) => {
                end += 1;
                total += 1 + chunks[end].chars().count();
            }
            (false, true) => {
                start -= 1;
                total += 1 + chunks[start].chars().count();
            }
            (false, false) => break,
        }
    }

    let mut passage = chunks[start..=end].join(" ");
    if passage.chars().count() > max_chars {
        // The anchor chunk alone exceeds the budget (huge ". "-free paragraphs,
        // CJK text, minified/table-heavy extractions). A head trim here would
        // discard the very query term that made this chunk score best, so trim
        // AROUND the first term occurrence instead.
        passage = trim_around_terms(&passage, query_terms, max_chars);
    } else if end + 1 < chunks.len() || scan.len() < trimmed.len() {
        passage.push('…');
    }
    if start > 0 && !passage.starts_with('…') {
        passage.insert(0, '…');
    }
    passage
}

/// Trim `text` to `max_chars` centered on the first query-term occurrence: the
/// window starts ~¼ of the budget before the term (snapped to a word boundary)
/// so the citation shows the term in context. Falls back to a head trim when no
/// term occurs in `text`.
fn trim_around_terms(text: &str, query_terms: &HashSet<String>, max_chars: usize) -> String {
    let lower = text.to_lowercase();
    // Earliest token-boundary occurrence of any query term (byte offset in `lower`;
    // offsets can differ from `text` only when lowercasing changes byte lengths,
    // so the window is re-derived over chars, not carried over as bytes).
    let mut earliest: Option<usize> = None;
    for term in query_terms {
        let mut from = 0_usize;
        while let Some(relative) = lower[from..].find(term.as_str()) {
            let at = from + relative;
            let before_ok = at == 0
                || !lower[..at]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric);
            let after = at + term.len();
            let after_ok = after >= lower.len()
                || !lower[after..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric);
            if before_ok && after_ok {
                earliest = Some(earliest.map_or(at, |current| current.min(at)));
                break;
            }
            from = after;
        }
    }
    let Some(term_byte) = earliest else {
        return trim_to_chars(text, max_chars);
    };
    // Convert the byte offset to a char offset (in `lower`, same char count as
    // `text` for this purpose) and open the window ~¼ budget before the term.
    let term_char = lower[..term_byte].chars().count();
    let window_start_char = term_char.saturating_sub(max_chars / 4);
    if window_start_char == 0 {
        return trim_to_chars(text, max_chars);
    }
    let start_byte = text
        .char_indices()
        .nth(window_start_char)
        .map_or(0, |(byte, _)| byte);
    // Snap forward to the next word boundary so the window never opens mid-word.
    let snapped = text[start_byte..]
        .find(char::is_whitespace)
        .map_or(start_byte, |relative| start_byte + relative);
    let tail = text[snapped..].trim_start();
    format!("…{}", trim_to_chars(tail, max_chars.saturating_sub(1)))
}

/// Split into paragraph chunks; paragraphs longer than ~400 chars are further
/// split on sentence boundaries so the passage window has fine enough granularity.
fn split_chunks(text: &str) -> Vec<&str> {
    const TARGET_CHARS: usize = 400;
    let mut chunks = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            continue;
        }
        if paragraph.chars().count() <= TARGET_CHARS {
            chunks.push(paragraph);
            continue;
        }
        // Accumulate sentences (". "-delimited) into ≤TARGET_CHARS chunks. Byte
        // offsets stay aligned because split_inclusive pieces are contiguous.
        let mut start_byte = 0_usize;
        let mut cursor = 0_usize;
        let mut length_chars = 0_usize;
        for piece in paragraph.split_inclusive(". ") {
            let piece_chars = piece.chars().count();
            if length_chars > 0 && length_chars + piece_chars > TARGET_CHARS {
                let chunk = paragraph[start_byte..cursor].trim();
                if !chunk.is_empty() {
                    chunks.push(chunk);
                }
                start_byte = cursor;
                length_chars = 0;
            }
            cursor += piece.len();
            length_chars += piece_chars;
        }
        if start_byte < paragraph.len() {
            let tail = paragraph[start_byte..].trim();
            if !tail.is_empty() {
                chunks.push(tail);
            }
        }
    }
    chunks
}

/// Query-term density of one chunk: unique matched terms weighted over raw
/// occurrences so a chunk touching several query terms beats one repeating a
/// single term.
fn chunk_score(chunk: &str, query_terms: &HashSet<String>) -> usize {
    let lower = chunk.to_lowercase();
    let counts = term_counts(&lower);
    let mut unique = 0_usize;
    let mut occurrences = 0_usize;
    for term in query_terms {
        if let Some(count) = counts.get(term.as_str()) {
            unique += 1;
            occurrences += *count;
        }
    }
    unique * 3 + occurrences
}

// ── text utilities ──

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

    #[test]
    fn idf_lifts_source_matching_the_rare_discriminative_term() {
        // Every candidate matches the common term; only one also matches the rare
        // one. IDF weighting must rank the rare-term source first even though the
        // common-only source repeats the common term more often.
        let hits = vec![
            hit("https://common.com", "python tips", "python python python"),
            hit(
                "https://rare.com",
                "python asyncio",
                "python asyncio internals",
            ),
            hit("https://also.com", "python intro", "python for beginners"),
        ];
        let contents = vec![
            "python python python python".to_string(),
            "python asyncio event loop".to_string(),
            "python basics".to_string(),
        ];
        let ranked = rerank("python asyncio", &hits, &contents, 3, 300);
        assert_eq!(ranked[0].url, "https://rare.com");
    }

    #[test]
    fn junk_host_sinks_below_equal_relevance_source() {
        let hits = vec![
            hit(
                "https://pinterest.com/pin/1",
                "rust async",
                "rust async tokio",
            ),
            hit("https://neutral.dev/post", "rust async", "rust async tokio"),
        ];
        let contents = vec![
            "rust async tokio".to_string(),
            "rust async tokio".to_string(),
        ];
        let ranked = rerank("rust async tokio", &hits, &contents, 5, 200);
        assert_eq!(ranked[0].url, "https://neutral.dev/post");
    }

    #[test]
    fn best_passage_selects_the_relevant_middle_of_the_page() {
        let boilerplate = "Home About Products Pricing Blog Contact newsletter signup footer. ";
        let content = format!(
            "{}\nThe tokio runtime schedules asynchronous rust tasks across worker threads.\n{}",
            boilerplate.repeat(10),
            boilerplate.repeat(10),
        );
        let hits = vec![hit("https://x.com", "t", "s")];
        let ranked = rerank("tokio rust runtime", &hits, &[content], 1, 120);
        let cited = &ranked[0].content;
        assert!(
            cited.contains("tokio runtime"),
            "must cite the on-topic passage, got: {cited}"
        );
        assert!(
            cited.starts_with('…'),
            "a mid-page passage is marked as elided from the start"
        );
    }

    #[test]
    fn best_passage_centers_on_term_inside_one_oversized_chunk() {
        // A single ". "-free run longer than the budget, with the only query-term
        // occurrence deep past the head: the citation must still contain the term
        // (review finding: head-trim silently dropped it).
        let filler = "lorem ipsum dolor sit amet ".repeat(40); // ~1080 chars, no ". "
        let content = format!("{filler}tokio runtime details here {filler}");
        let hits = vec![hit("https://x.com", "t", "s")];
        let ranked = rerank("tokio", &hits, &[content], 1, 200);
        assert!(
            ranked[0].content.contains("tokio"),
            "oversized-chunk citation must center on the query term, got: {}",
            ranked[0].content
        );
        assert!(ranked[0].content.starts_with('…'));
    }

    #[test]
    fn rerank_multi_empty_queries_do_not_panic() {
        // Degenerate public-API inputs: no queries at all, and one token-free
        // query. Both must degrade to provider-order results, never panic.
        let hits = vec![hit("https://a.com/x", "a", "a")];
        let contents = vec![String::new()];
        let ranked = rerank_multi(&[], &hits, &contents, &[vec![0]], 5, 100, 2);
        assert_eq!(ranked.len(), 1);
        let ranked = rerank_multi(&["??".to_string()], &hits, &contents, &[vec![0]], 5, 100, 2);
        assert_eq!(ranked.len(), 1);
    }

    #[test]
    fn rerank_deep_enforces_domain_diversity() {
        // Three strong results from ONE host + one from another; per_host_cap=2 must
        // drop the third same-host result so the output spans sites.
        let hits = vec![
            hit("https://same.com/1", "rust async", "rust async tokio"),
            hit("https://same.com/2", "rust async", "rust async tokio"),
            hit("https://same.com/3", "rust async", "rust async tokio"),
            hit("https://other.com/1", "rust async", "rust async tokio"),
        ];
        let contents = vec![
            "rust async tokio".to_string(),
            "rust async tokio".to_string(),
            "rust async tokio".to_string(),
            "rust async tokio".to_string(),
        ];
        let freq = HashMap::new();
        let ranked = rerank_deep("rust async tokio", &hits, &contents, 10, 200, 2, &freq);
        let same = ranked.iter().filter(|s| s.domain == "same.com").count();
        assert_eq!(same, 2, "at most per_host_cap sources per host");
        assert!(ranked.iter().any(|s| s.domain == "other.com"));
    }

    #[test]
    fn rerank_deep_consensus_lifts_multi_query_hit() {
        let hits = vec![
            hit("https://a.com/x", "rust", "rust guide"),
            hit("https://b.com/y", "rust", "rust guide"),
        ];
        let contents = vec!["rust guide".to_string(), "rust guide".to_string()];
        // b.com surfaced from 3 sub-queries → consensus boost floats it above a.com.
        let mut freq = HashMap::new();
        freq.insert("https://b.com/y".to_string(), 3);
        let ranked = rerank_deep("rust", &hits, &contents, 5, 200, 3, &freq);
        assert_eq!(ranked[0].url, "https://b.com/y");
    }

    #[test]
    fn rerank_multi_ranks_by_best_query_and_attributes() {
        let queries = vec![
            "rust tokio runtime".to_string(),
            "python asyncio".to_string(),
        ];
        let hits = vec![
            hit(
                "https://rust.dev/a",
                "tokio guide",
                "the tokio runtime for rust",
            ),
            hit(
                "https://py.dev/b",
                "asyncio guide",
                "python asyncio event loop",
            ),
            hit("https://off.dev/c", "cooking", "pasta recipes"),
        ];
        let contents = vec![
            "tokio runtime rust".to_string(),
            "python asyncio".to_string(),
            String::new(),
        ];
        let surfaced = vec![vec![0], vec![1], vec![0]];
        let ranked = rerank_multi(&queries, &hits, &contents, &surfaced, 10, 300, 3);
        // Both on-topic sources kept (each matched its own facet); off-topic dropped.
        assert_eq!(ranked.len(), 2);
        assert!(ranked.iter().any(|s| s.source.url == "https://rust.dev/a"));
        assert!(ranked.iter().any(|s| s.source.url == "https://py.dev/b"));
        let rust_source = ranked
            .iter()
            .find(|s| s.source.url == "https://rust.dev/a")
            .expect("rust source present");
        assert_eq!(rust_source.matched_queries, vec!["rust tokio runtime"]);
    }

    #[test]
    fn rerank_multi_consensus_lifts_source_surfaced_by_several_queries() {
        let queries = vec!["rust async".to_string(), "tokio tutorial".to_string()];
        let hits = vec![
            hit(
                "https://single.dev/a",
                "rust async tokio",
                "rust async tokio",
            ),
            hit("https://both.dev/b", "rust async tokio", "rust async tokio"),
        ];
        let contents = vec![
            "rust async tokio tutorial".to_string(),
            "rust async tokio tutorial".to_string(),
        ];
        let surfaced = vec![vec![0], vec![0, 1]];
        let ranked = rerank_multi(&queries, &hits, &contents, &surfaced, 5, 200, 3);
        assert_eq!(
            ranked[0].source.url, "https://both.dev/b",
            "cross-query consensus must lift the doubly-surfaced source"
        );
        assert_eq!(ranked[0].matched_queries.len(), 2);
    }

    #[test]
    fn rerank_multi_enforces_domain_diversity() {
        let queries = vec!["rust async".to_string()];
        let hits = vec![
            hit("https://same.com/1", "rust async", "rust async"),
            hit("https://same.com/2", "rust async", "rust async"),
            hit("https://same.com/3", "rust async", "rust async"),
            hit("https://other.com/1", "rust async", "rust async"),
        ];
        let contents = vec![
            "rust async".to_string(),
            "rust async".to_string(),
            "rust async".to_string(),
            "rust async".to_string(),
        ];
        let surfaced = vec![vec![0], vec![0], vec![0], vec![0]];
        let ranked = rerank_multi(&queries, &hits, &contents, &surfaced, 10, 200, 2);
        let same = ranked
            .iter()
            .filter(|s| s.source.domain == "same.com")
            .count();
        assert_eq!(same, 2);
        assert!(ranked.iter().any(|s| s.source.domain == "other.com"));
    }

    #[test]
    fn host_of_strips_scheme_userinfo_and_port() {
        assert_eq!(host_of("https://user@Docs.RS:443/x"), "docs.rs");
        assert_eq!(host_of("http://example.com/path?q=1"), "example.com");
        assert_eq!(host_of("https://[2001:db8::1]:8080/x"), "2001:db8::1");
    }
}
