//! Web research engine — the network glue around [`lux_research`].
//!
//! A `Perplexica`-style pipeline exposed as an IDE/agent tool, deliberately
//! SEPARATE from `WebFetch` (single URL) and any built-in web search: it runs a
//! provider search (`SearxNG` JSON, or a keyless `DuckDuckGo` HTML fallback), fetches
//! the top result pages concurrently through the SSRF-guarded [`crate::web_fetch`]
//! path, reranks them by lexical relevance, and returns ranked, cited sources for
//! the model to synthesize an answer from. It never calls into `web_fetch::fetch`'s
//! command surface, so the existing `WebFetch` tool is untouched.

use std::collections::HashMap;

use lux_research::{
    brave_search_url, canonical_url_key, duckduckgo_lite_search_url, duckduckgo_search_url,
    expand_queries, extract_result_links, focus_biased_query, parse_brave_html,
    parse_duckduckgo_html, parse_searxng_json, rerank, rerank_deep, rerank_multi,
    searxng_search_url, FocusMode, MultiResearchResponse, ResearchDepth, ResearchOptions,
    ResearchResponse, SearchHit,
};
use tauri::State;

use crate::{web_fetch, SharedState};

/// Settings key (user scope) holding the optional `SearxNG` base URL.
pub const SEARXNG_URL_KEY: &str = "ai.research.searxngUrl";

const SEARCH_TIMEOUT_SECS: u64 = 18;
const SEARCH_MAX_BYTES: usize = 600_000;
const PAGE_TIMEOUT_SECS: u64 = 15;
const PAGE_MAX_BYTES: u64 = 200_000;
/// Raw-HTML byte cap when fetching a result page for the deep-mode link crawl.
const CRAWL_MAX_BYTES: usize = 300_000;
/// Deep-mode crawl reads outbound links from at most this many top result pages.
const CRAWL_SEED_PAGES: usize = 3;
/// Max result pages fetched at once (politeness + socket pressure bound); results
/// keep their input order regardless.
const FETCH_CONCURRENCY: usize = 8;
/// Hard cap on distinct queries per `MultiWebResearch` run.
pub const MULTI_MAX_QUERIES: usize = 6;
/// Total page-fetch budget across all queries of a multi run.
const MULTI_MAX_FETCHES: usize = 18;
/// Per-host diversity cap for the merged multi ranking.
const MULTI_PER_HOST_CAP: usize = 2;
/// Ceiling on returned merged sources for a multi run.
const MULTI_MAX_SOURCES_CEILING: usize = 20;

/// Read the configured `SearxNG` base URL from settings (trimmed; `None` if unset/blank).
fn configured_searxng_url(state: &State<'_, SharedState>) -> Option<String> {
    let settings = state.settings.lock().ok()?;
    let value = settings
        .as_ref()?
        .get(lux_core::SettingsScope::User, SEARXNG_URL_KEY)?;
    let url = value.value.as_str()?.trim().to_string();
    (!url.is_empty()).then_some(url)
}

/// Run a research query: search → fetch top pages → rerank → assemble.
#[tauri::command]
pub async fn web_research(
    state: State<'_, SharedState>,
    query: String,
    options: Option<ResearchOptions>,
) -> Result<ResearchResponse, String> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err("WebResearch requires a non-empty query.".to_string());
    }
    let options = options.unwrap_or_default();
    let profile = options.depth.profile();
    let searxng = configured_searxng_url(&state);
    let mut notes = Vec::new();
    if searxng.is_none() {
        notes.push(
            "Using the keyless search fallback (DuckDuckGo, then Brave). Configure a SearxNG instance in Settings → AI → Web Research for richer, focus-aware results.".to_string(),
        );
    }

    // ── 1. Search — deep mode expands the query into several variants, issues
    // them ALL concurrently, and merges ──
    let queries = if profile.max_queries > 1 {
        expand_queries(&query, options.focus, profile.max_queries)
    } else {
        vec![query.clone()]
    };
    let searches = queries
        .iter()
        .map(|sub_query| search_provider(searxng.as_deref(), sub_query, &options));
    let search_results = futures_util::future::join_all(searches).await;

    let mut merged: Vec<SearchHit> = Vec::new();
    // How many of the (expanded) sub-queries surfaced each URL (by canonical key,
    // so tracking-param variants merge) — a consensus signal the deep reranker
    // uses to float widely-cited sources up.
    let mut frequency: HashMap<String, usize> = HashMap::new();
    let mut provider = "duckduckgo";
    let mut backend_responded = false;
    let mut fallback_noted = false;
    let mut first_error: Option<String> = None;
    for search_result in search_results {
        match search_result {
            Ok(result) => {
                provider = result.provider;
                backend_responded |= result.backend_responded;
                if let (Some(msg), false) = (result.fallback_note, fallback_noted) {
                    notes.push(msg);
                    fallback_noted = true;
                }
                // Consensus counts DISTINCT sub-queries per canonical key. One
                // sub-query's result list can contain tracking-param variants of
                // the same page (multi-engine SearxNG merges do) — those must not
                // self-inflate the "how many sub-queries agree" signal.
                let mut seen_this_query: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for hit in result.hits {
                    let key = canonical_url_key(&hit.url);
                    if seen_this_query.insert(key.clone()) {
                        *frequency.entry(key).or_insert(0) += 1;
                    }
                    merged.push(hit);
                }
            }
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    if queries.len() > 1 {
        notes.push(format!(
            "Deep research: issued {} query variants and merged the results.",
            queries.len()
        ));
    }
    // Every attempt errored AND nothing ever responded → a hard, surfaced failure
    // (network down / all endpoints refused), not a disguised empty result set.
    if merged.is_empty() && !backend_responded {
        if let Some(error) = first_error {
            return Err(error);
        }
    }
    // Backend answered but parsed to nothing: a search-backend issue, not "no results".
    if merged.is_empty() && backend_responded {
        notes.push(
            "The keyless search engines (DuckDuckGo, then Brave) responded but no results could be parsed (a challenge/anomaly page, or markup drift). This is a search-backend issue, not an absence of results — retry shortly, rephrase the query, or configure a SearxNG instance in Settings → AI → Web Research.".to_string(),
        );
    }

    // `focus` maps to SearxNG categories only; the keyless engines have no
    // category knob, so it is approximated there by biasing the query text
    // (see [`focus_biased_query`]). Tell the model which mechanism applied.
    let effective_focus = options.focus;
    if provider != "searxng" && options.focus != FocusMode::Web {
        notes.push(format!(
            "focus '{}' has no native {provider} category; it was approximated by biasing the search query. Configure a SearxNG instance in Settings → AI → Web Research for true focus categories.",
            options.focus.searxng_category(),
        ));
    }

    // Dedupe by URL (preserve first/highest-placed occurrence) and cap fetches by depth.
    dedupe_hits(&mut merged);
    if merged.is_empty() {
        notes.push("No search results found.".to_string());
        return Ok(ResearchResponse {
            query,
            focus: effective_focus,
            provider: provider.to_string(),
            source_count: 0,
            sources: Vec::new(),
            notes,
        });
    }
    let fetch_count = profile.max_fetches.min(merged.len());
    let mut pool: Vec<SearchHit> = merged[..fetch_count].to_vec();

    // ── 2. Fetch result pages concurrently (SSRF-guarded), bounded by the profile ──
    let mut fetched = fetch_pages(&pool).await;

    // ── 2b. Deep mode: one hop of on-topic in-page link crawl for extra depth ──
    let mut crawl_added = 0usize;
    if profile.crawl_budget > 0 {
        let mut seen: std::collections::HashSet<String> =
            pool.iter().map(|hit| hit.url.clone()).collect();
        let crawl_hits = crawl_one_hop(&pool, &query, profile.crawl_budget, &mut seen).await;
        if !crawl_hits.is_empty() {
            let mut crawl_fetched = fetch_pages(&crawl_hits).await;
            crawl_added = crawl_hits.len();
            pool.extend(crawl_hits);
            fetched.append(&mut crawl_fetched);
        }
    }

    // Backfill a missing title from the fetched page when the provider gave none.
    let hits_for_rank: Vec<SearchHit> = pool
        .iter()
        .enumerate()
        .map(|(index, hit)| {
            let mut hit = hit.clone();
            if hit.title.trim().is_empty() {
                if let Some(title) = fetched.get(index).and_then(|(_, title)| title.clone()) {
                    hit.title = title;
                }
            }
            hit
        })
        .collect();
    let contents: Vec<String> = fetched.into_iter().map(|(text, _)| text).collect();

    // ── 3. Rerank — deep folds in cross-query consensus + domain diversity ──
    let source_ceiling = options.max_sources.min(profile.max_sources_ceiling).max(1);
    let sources = if matches!(options.depth, ResearchDepth::Deep) {
        rerank_deep(
            &query,
            &hits_for_rank,
            &contents,
            source_ceiling,
            options.max_chars_per_source,
            profile.per_host_cap,
            &frequency,
        )
    } else {
        rerank(
            &query,
            &hits_for_rank,
            &contents,
            source_ceiling,
            options.max_chars_per_source,
        )
    };

    if matches!(options.depth, ResearchDepth::Deep) {
        let domains: std::collections::HashSet<&str> = sources
            .iter()
            .map(|source| source.domain.as_str())
            .collect();
        notes.push(format!(
            "Deep summary: fetched {} page(s) ({crawl_added} via link-crawl); returning {} source(s) across {} domain(s).",
            pool.len(),
            sources.len(),
            domains.len()
        ));
    }

    // Signal when fewer sources came back than requested, distinguishing
    // "fetched pages were dropped as off-topic" (rerank filtered them) from
    // "there simply weren't that many candidates".
    let requested = source_ceiling;
    let returned = sources.len();
    if returned < requested {
        let dropped = pool.len().saturating_sub(returned);
        if dropped > 0 {
            notes.push(format!(
                "Requested up to {requested} sources; returned {returned}. {dropped} fetched page(s) shared no query terms and were dropped as off-topic — consider broadening or rephrasing the query."
            ));
        } else {
            notes.push(format!(
                "Requested up to {requested} sources; only {} candidate result(s) were available. Consider broadening the query.",
                pool.len()
            ));
        }
    }

    Ok(ResearchResponse {
        query,
        focus: effective_focus,
        provider: provider.to_string(),
        source_count: sources.len(),
        sources,
        notes,
    })
}

/// Run several research queries CONCURRENTLY and merge them into one globally
/// ranked, domain-diverse source list ("`MultiWebResearch`"): all searches fan out
/// in parallel, hits merge by canonical URL with per-query attribution, a fair
/// (round-robin across queries) slice of pages is fetched — bounded by
/// [`MULTI_MAX_FETCHES`] total and [`FETCH_CONCURRENCY`] in flight — and one
/// multi-query rerank scores every page against every query, boosting sources
/// several queries agree on.
#[tauri::command]
pub async fn multi_web_research(
    state: State<'_, SharedState>,
    queries: Vec<String>,
    options: Option<ResearchOptions>,
) -> Result<MultiResearchResponse, String> {
    // Normalize: trim, drop empties, dedupe case-insensitively, cap the count.
    let mut seen_queries = std::collections::HashSet::new();
    let mut cleaned: Vec<String> = Vec::new();
    for raw in queries {
        let trimmed = raw.trim().to_string();
        if !trimmed.is_empty() && seen_queries.insert(trimmed.to_lowercase()) {
            cleaned.push(trimmed);
        }
    }
    if cleaned.is_empty() {
        return Err(
            "MultiWebResearch requires at least one non-empty query (2-6 distinct queries recommended)."
                .to_string(),
        );
    }
    let mut notes = Vec::new();
    if cleaned.len() > MULTI_MAX_QUERIES {
        notes.push(format!(
            "Capped to the first {MULTI_MAX_QUERIES} distinct queries ({} were given).",
            cleaned.len()
        ));
        cleaned.truncate(MULTI_MAX_QUERIES);
    }
    if cleaned.len() == 1 {
        notes.push(
            "Only one distinct query was given — WebResearch is the better fit for single queries."
                .to_string(),
        );
    }
    let options = options.unwrap_or_default();
    let searxng = configured_searxng_url(&state);
    if searxng.is_none() {
        notes.push(
            "Using the keyless search fallback (DuckDuckGo, then Brave). Configure a SearxNG instance in Settings → AI → Web Research for richer, focus-aware results.".to_string(),
        );
    }

    // ── 1. All queries search CONCURRENTLY ──
    let searches = cleaned
        .iter()
        .map(|query| search_provider(searxng.as_deref(), query, &options));
    let search_results = futures_util::future::join_all(searches).await;

    let mut provider = "duckduckgo";
    let mut backend_responded = false;
    let mut fallback_noted = false;
    let mut first_error: Option<String> = None;
    let mut per_query_hits: Vec<Vec<SearchHit>> = Vec::with_capacity(cleaned.len());
    for (query_index, search_result) in search_results.into_iter().enumerate() {
        match search_result {
            Ok(result) => {
                provider = result.provider;
                backend_responded |= result.backend_responded;
                if let (Some(msg), false) = (result.fallback_note, fallback_noted) {
                    notes.push(msg);
                    fallback_noted = true;
                }
                if result.hits.is_empty() {
                    notes.push(format!(
                        "Query '{}' returned no results.",
                        cleaned[query_index]
                    ));
                }
                per_query_hits.push(result.hits);
            }
            Err(error) => {
                notes.push(format!("Query '{}' failed: {error}", cleaned[query_index]));
                if first_error.is_none() {
                    first_error = Some(error);
                }
                per_query_hits.push(Vec::new());
            }
        }
    }

    // ── 2. Merge round-robin (fair across queries) by canonical URL, keeping
    // which queries surfaced each hit ──
    let mut pool: Vec<SearchHit> = Vec::new();
    let mut surfaced_by: Vec<Vec<usize>> = Vec::new();
    let mut key_to_pool: HashMap<String, usize> = HashMap::new();
    let deepest = per_query_hits.iter().map(Vec::len).max().unwrap_or(0);
    for position in 0..deepest {
        for (query_index, hits) in per_query_hits.iter().enumerate() {
            let Some(hit) = hits.get(position) else {
                continue;
            };
            let key = canonical_url_key(&hit.url);
            if let Some(&pool_index) = key_to_pool.get(&key) {
                if !surfaced_by[pool_index].contains(&query_index) {
                    surfaced_by[pool_index].push(query_index);
                }
            } else {
                key_to_pool.insert(key, pool.len());
                surfaced_by.push(vec![query_index]);
                pool.push(hit.clone());
            }
        }
    }

    if pool.is_empty() {
        // Every attempt errored AND nothing responded → hard failure, not an
        // empty result set in disguise.
        if !backend_responded {
            if let Some(error) = first_error {
                return Err(error);
            }
        }
        notes.push("No search results found for any query.".to_string());
        return Ok(MultiResearchResponse {
            queries: cleaned,
            focus: options.focus,
            provider: provider.to_string(),
            source_count: 0,
            sources: Vec::new(),
            notes,
        });
    }
    let merged_unique = pool.len();
    pool.truncate(MULTI_MAX_FETCHES);
    surfaced_by.truncate(MULTI_MAX_FETCHES);

    if provider != "searxng" && options.focus != FocusMode::Web {
        notes.push(format!(
            "focus '{}' has no native {provider} category; it was approximated by biasing the search queries.",
            options.focus.searxng_category(),
        ));
    }

    // ── 3. Fetch the merged slice (bounded concurrency) + backfill titles ──
    let fetched = fetch_pages(&pool).await;
    let hits_for_rank: Vec<SearchHit> = pool
        .iter()
        .enumerate()
        .map(|(index, hit)| {
            let mut hit = hit.clone();
            if hit.title.trim().is_empty() {
                if let Some(title) = fetched.get(index).and_then(|(_, title)| title.clone()) {
                    hit.title = title;
                }
            }
            hit
        })
        .collect();
    let contents: Vec<String> = fetched.into_iter().map(|(text, _)| text).collect();

    // ── 4. One global multi-query rerank ──
    let max_sources = options.max_sources.clamp(1, MULTI_MAX_SOURCES_CEILING);
    let sources = rerank_multi(
        &cleaned,
        &hits_for_rank,
        &contents,
        &surfaced_by,
        max_sources,
        options.max_chars_per_source,
        MULTI_PER_HOST_CAP,
    );

    let domains: std::collections::HashSet<&str> = sources
        .iter()
        .map(|source| source.source.domain.as_str())
        .collect();
    notes.push(format!(
        "Parallel research: {} quer{} searched concurrently; {merged_unique} unique result(s) merged; fetched {} page(s); returning {} source(s) across {} domain(s).",
        cleaned.len(),
        if cleaned.len() == 1 { "y" } else { "ies" },
        pool.len(),
        sources.len(),
        domains.len()
    ));

    Ok(MultiResearchResponse {
        queries: cleaned,
        focus: options.focus,
        provider: provider.to_string(),
        source_count: sources.len(),
        sources,
        notes,
    })
}

/// Outcome of one provider search: the label, hits, whether a `DuckDuckGo` backend
/// answered (2xx) at all, and an optional note when it fell back `SearxNG` → DDG.
struct ProviderSearch {
    provider: &'static str,
    hits: Vec<SearchHit>,
    backend_responded: bool,
    fallback_note: Option<String>,
}

/// One provider search: `SearxNG` when configured (falling back to `DuckDuckGo` on an
/// empty/error result), else the keyless `DuckDuckGo` path.
async fn search_provider(
    searxng: Option<&str>,
    query: &str,
    options: &ResearchOptions,
) -> Result<ProviderSearch, String> {
    if let Some(base) = searxng {
        match search_searxng(base, query, options).await {
            Ok(hits) if !hits.is_empty() => Ok(ProviderSearch {
                provider: "searxng",
                hits,
                backend_responded: false,
                fallback_note: None,
            }),
            Ok(_) => {
                let outcome = search_keyless(query, options.focus).await?;
                Ok(ProviderSearch {
                    provider: keyless_provider_name(&outcome),
                    hits: outcome.hits,
                    backend_responded: outcome.backend_responded,
                    fallback_note: Some(
                        "SearxNG returned no results; fell back to the keyless engines."
                            .to_string(),
                    ),
                })
            }
            Err(error) => {
                let outcome = search_keyless(query, options.focus).await?;
                Ok(ProviderSearch {
                    provider: keyless_provider_name(&outcome),
                    hits: outcome.hits,
                    backend_responded: outcome.backend_responded,
                    fallback_note: Some(format!(
                        "SearxNG search failed ({error}); fell back to the keyless engines."
                    )),
                })
            }
        }
    } else {
        let outcome = search_keyless(query, options.focus).await?;
        Ok(ProviderSearch {
            provider: keyless_provider_name(&outcome),
            hits: outcome.hits,
            backend_responded: outcome.backend_responded,
            fallback_note: None,
        })
    }
}

/// Which keyless engine actually produced the hits ("brave" when the Brave
/// fallback rescued a DDG failure, else "duckduckgo").
fn keyless_provider_name(outcome: &DuckDuckGoOutcome) -> &'static str {
    if outcome
        .hits
        .first()
        .is_some_and(|hit| hit.engine == "brave")
    {
        "brave"
    } else {
        "duckduckgo"
    }
}

/// Fetch each hit's page concurrently through the SSRF-guarded [`web_fetch`] path,
/// returning `(extracted_text, page_title)` per hit (empty/None on any failure).
/// Concurrency is bounded to [`FETCH_CONCURRENCY`] in-flight requests; output
/// order matches input order.
async fn fetch_pages(hits: &[SearchHit]) -> Vec<(String, Option<String>)> {
    use futures_util::StreamExt;
    // Owned URLs first: a `|hit| async move` closure over `&SearchHit` ties the
    // future's opaque type to the borrow's lifetime, which `buffered` rejects
    // ("implementation of FnOnce is not general enough").
    let urls: Vec<String> = hits.iter().map(|hit| hit.url.clone()).collect();
    futures_util::stream::iter(urls)
        .map(|url| async move {
            web_fetch::fetch(url, Some(PAGE_MAX_BYTES), Some(PAGE_TIMEOUT_SECS))
                .await
                .map_or_else(
                    |_| (String::new(), None),
                    |page| {
                        (
                            page.text().to_string(),
                            page.title().map(ToString::to_string),
                        )
                    },
                )
        })
        .buffered(FETCH_CONCURRENCY)
        .collect()
        .await
}

/// Deep-mode 1-hop crawl: read the raw HTML of the top few result pages, extract
/// on-topic outbound links (SSRF-gated inside [`extract_result_links`]), and return
/// up to `budget` new [`SearchHit`]s not already in `seen`.
async fn crawl_one_hop(
    top: &[SearchHit],
    query: &str,
    budget: usize,
    seen: &mut std::collections::HashSet<String>,
) -> Vec<SearchHit> {
    let mut found: Vec<SearchHit> = Vec::new();
    for hit in top.iter().take(CRAWL_SEED_PAGES) {
        if found.len() >= budget {
            break;
        }
        let remaining = budget - found.len();
        let Ok(html) = web_fetch::fetch_text(
            &hit.url,
            "text/html",
            PAGE_TIMEOUT_SECS,
            CRAWL_MAX_BYTES,
            false,
        )
        .await
        else {
            continue;
        };
        for url in extract_result_links(&html, &hit.url, query, remaining) {
            if seen.insert(url.clone()) {
                found.push(SearchHit {
                    url,
                    title: String::new(),
                    snippet: String::new(),
                    engine: "crawl".to_string(),
                    provider_score: 0.0,
                });
                if found.len() >= budget {
                    break;
                }
            }
        }
    }
    found
}

async fn search_searxng(
    base: &str,
    query: &str,
    options: &ResearchOptions,
) -> Result<Vec<SearchHit>, String> {
    let url = searxng_search_url(base, query, options.focus);
    // A user-configured SearxNG instance is trusted and usually local, so the
    // private-host guard is relaxed for it (but not for fetched result pages).
    let body = web_fetch::fetch_text(
        &url,
        "application/json",
        SEARCH_TIMEOUT_SECS,
        SEARCH_MAX_BYTES,
        true,
    )
    .await?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| format!("SearxNG returned invalid JSON: {error}"))?;
    Ok(parse_searxng_json(&json))
}

/// Outcome of a `DuckDuckGo` fallback search. `backend_responded` records that at
/// least one endpoint returned an HTTP 2xx body (even if it parsed to zero hits),
/// so the caller can distinguish "no results exist" from "the backend served an
/// unparseable anomaly/challenge page".
struct DuckDuckGoOutcome {
    hits: Vec<SearchHit>,
    backend_responded: bool,
}

/// Keyless search: `DuckDuckGo` first (best markup for scraping when it answers),
/// then Brave Search when DDG errors on every endpoint or serves its
/// anomaly/challenge page (which currently 403s the html/lite endpoints for
/// non-browser TLS/UA fingerprints). Engine name in the hits records which one
/// actually produced results.
async fn search_keyless(query: &str, focus: FocusMode) -> Result<DuckDuckGoOutcome, String> {
    let ddg = search_duckduckgo(query, focus).await;
    match ddg {
        Ok(outcome) if !outcome.hits.is_empty() => Ok(outcome),
        // DDG responded-but-empty (challenge page) or hard-failed → try Brave.
        Ok(empty_outcome) => match search_brave(query, focus).await {
            Ok(outcome) if !outcome.hits.is_empty() => Ok(outcome),
            // Brave also empty/failed: keep DDG's outcome so the caller's
            // "backend responded but unparseable" note stays accurate.
            _ => Ok(empty_outcome),
        },
        Err(ddg_error) => match search_brave(query, focus).await {
            Ok(outcome) => Ok(outcome),
            Err(brave_error) => Err(format!(
                "keyless search failed — DuckDuckGo: {ddg_error}; Brave: {brave_error}"
            )),
        },
    }
}

/// Brave Search HTML fallback (keyless, GET). One stable endpoint; the parser
/// keys off Brave's stable semantic tokens rather than its hashed classes.
async fn search_brave(query: &str, focus: FocusMode) -> Result<DuckDuckGoOutcome, String> {
    let biased = focus_biased_query(query, focus);
    let query = biased.as_deref().unwrap_or(query);
    let url = brave_search_url(query);
    let body = web_fetch::fetch_text(
        &url,
        "text/html",
        SEARCH_TIMEOUT_SECS,
        SEARCH_MAX_BYTES,
        false,
    )
    .await?;
    Ok(DuckDuckGoOutcome {
        hits: parse_brave_html(&body),
        backend_responded: true,
    })
}

async fn search_duckduckgo(query: &str, focus: FocusMode) -> Result<DuckDuckGoOutcome, String> {
    // DDG has no category verticals: approximate a non-web focus by biasing the
    // query text (never stacks — see `focus_biased_query`).
    let biased = focus_biased_query(query, focus);
    let query = biased.as_deref().unwrap_or(query);
    let mut backend_responded = false;
    let mut last_error: Option<String> = None;

    // DuckDuckGo increasingly serves an anomaly/challenge page to a plain GET on
    // the html endpoint, and its lite markup shifts periodically, so try several
    // (endpoint, method) combinations and take the first that parses to hits. The
    // html endpoint via POST (query in the form body) is the shape maintained
    // scrapers rely on and is the most reliable; the others are progressive
    // fallbacks. We keep going on a hard error rather than aborting, so one dead
    // endpoint doesn't sink the whole search.
    let html_get_url = duckduckgo_search_url(query);
    let lite_get_url = duckduckgo_lite_search_url(query);
    let attempts: [(&str, DuckDuckGoMethod); 4] = [
        (DDG_HTML_ENDPOINT, DuckDuckGoMethod::PostForm),
        (DDG_LITE_ENDPOINT, DuckDuckGoMethod::PostForm),
        (html_get_url.as_str(), DuckDuckGoMethod::Get),
        (lite_get_url.as_str(), DuckDuckGoMethod::Get),
    ];

    for (url, method) in attempts {
        let fetched = match method {
            DuckDuckGoMethod::Get => {
                web_fetch::fetch_text(
                    url,
                    "text/html",
                    SEARCH_TIMEOUT_SECS,
                    SEARCH_MAX_BYTES,
                    false,
                )
                .await
            }
            DuckDuckGoMethod::PostForm => {
                web_fetch::fetch_text_form(
                    url,
                    &[("q", query)],
                    "text/html",
                    SEARCH_TIMEOUT_SECS,
                    SEARCH_MAX_BYTES,
                )
                .await
            }
        };
        match fetched {
            Ok(body) => {
                backend_responded = true;
                let hits = parse_duckduckgo_html(&body);
                if !hits.is_empty() {
                    return Ok(DuckDuckGoOutcome {
                        hits,
                        backend_responded,
                    });
                }
            }
            Err(error) => last_error = Some(error),
        }
    }

    // Every attempt either errored or parsed to zero. If NOTHING ever responded,
    // that is a hard, surfaced failure (network down / all endpoints refused) —
    // don't disguise it as an empty result set.
    if !backend_responded {
        return Err(last_error.unwrap_or_else(|| {
            "DuckDuckGo search failed: no endpoint returned a response.".to_string()
        }));
    }
    Ok(DuckDuckGoOutcome {
        hits: Vec::new(),
        backend_responded,
    })
}

/// The `DuckDuckGo` html endpoint (query goes in the POST form body, not the URL).
const DDG_HTML_ENDPOINT: &str = "https://html.duckduckgo.com/html/";
/// The `DuckDuckGo` lite endpoint (query goes in the POST form body, not the URL).
const DDG_LITE_ENDPOINT: &str = "https://lite.duckduckgo.com/lite/";

/// How to issue a `DuckDuckGo` request: a plain GET (query in the URL) or a POST with
/// the query in a `application/x-www-form-urlencoded` body.
#[derive(Clone, Copy)]
enum DuckDuckGoMethod {
    Get,
    PostForm,
}

/// Dedupe by canonical URL key, so hits differing only in tracking decoration
/// (utm_*, fbclid, …), `www.`, or a trailing slash collapse to one source.
fn dedupe_hits(hits: &mut Vec<SearchHit>) {
    let mut seen = std::collections::HashSet::new();
    hits.retain(|hit| seen.insert(canonical_url_key(&hit.url)));
}
