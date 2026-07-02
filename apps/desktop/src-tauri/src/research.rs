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
    duckduckgo_lite_search_url, duckduckgo_search_url, expand_queries, extract_result_links,
    parse_duckduckgo_html, parse_searxng_json, rerank, rerank_deep, searxng_search_url, FocusMode,
    ResearchDepth, ResearchOptions, ResearchResponse, SearchHit,
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
            "Using the keyless DuckDuckGo fallback. Configure a SearxNG instance in Settings → AI → Web Research for richer, focus-aware results.".to_string(),
        );
    }

    // ── 1. Search — deep mode expands the query into several variants and merges ──
    let queries = if profile.max_queries > 1 {
        expand_queries(&query, options.focus, profile.max_queries)
    } else {
        vec![query.clone()]
    };
    let mut merged: Vec<SearchHit> = Vec::new();
    // How many of the (expanded) sub-queries surfaced each URL — a consensus signal
    // the deep reranker uses to float widely-cited sources up.
    let mut frequency: HashMap<String, usize> = HashMap::new();
    let mut provider = "duckduckgo";
    let mut backend_responded = false;
    let mut fallback_noted = false;
    let mut first_error: Option<String> = None;
    for sub_query in &queries {
        match search_provider(searxng.as_deref(), sub_query, &options).await {
            Ok(result) => {
                provider = result.provider;
                backend_responded |= result.backend_responded;
                if let (Some(msg), false) = (result.fallback_note, fallback_noted) {
                    notes.push(msg);
                    fallback_noted = true;
                }
                for hit in result.hits {
                    *frequency.entry(hit.url.clone()).or_insert(0) += 1;
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
            "The DuckDuckGo search backend responded but no results could be parsed (it likely served an anomaly/challenge page, or its markup changed). This is a search-backend issue, not an absence of results — retry shortly, rephrase the query, or configure a SearxNG instance in Settings → AI → Web Research.".to_string(),
        );
    }

    // `focus` maps to SearxNG categories only; the DuckDuckGo fallback has no
    // category knob, so a non-web focus is silently dropped there. Reflect the
    // focus actually applied and tell the model when its focus was ignored.
    let effective_focus = if provider == "duckduckgo" && options.focus != FocusMode::Web {
        notes.push(format!(
            "focus '{}' was ignored: the DuckDuckGo fallback does not support focus categories. Configure a SearxNG instance in Settings → AI → Web Research to use focus.",
            options.focus.searxng_category(),
        ));
        FocusMode::Web
    } else {
        options.focus
    };

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
                let outcome = search_duckduckgo(query).await?;
                Ok(ProviderSearch {
                    provider: "duckduckgo",
                    hits: outcome.hits,
                    backend_responded: outcome.backend_responded,
                    fallback_note: Some(
                        "SearxNG returned no results; fell back to DuckDuckGo.".to_string(),
                    ),
                })
            }
            Err(error) => {
                let outcome = search_duckduckgo(query).await?;
                Ok(ProviderSearch {
                    provider: "duckduckgo",
                    hits: outcome.hits,
                    backend_responded: outcome.backend_responded,
                    fallback_note: Some(format!(
                        "SearxNG search failed ({error}); fell back to DuckDuckGo."
                    )),
                })
            }
        }
    } else {
        let outcome = search_duckduckgo(query).await?;
        Ok(ProviderSearch {
            provider: "duckduckgo",
            hits: outcome.hits,
            backend_responded: outcome.backend_responded,
            fallback_note: None,
        })
    }
}

/// Fetch each hit's page concurrently through the SSRF-guarded [`web_fetch`] path,
/// returning `(extracted_text, page_title)` per hit (empty/None on any failure).
async fn fetch_pages(hits: &[SearchHit]) -> Vec<(String, Option<String>)> {
    let fetches = hits.iter().map(|hit| {
        let url = hit.url.clone();
        async move {
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
        }
    });
    futures_util::future::join_all(fetches).await
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

async fn search_duckduckgo(query: &str) -> Result<DuckDuckGoOutcome, String> {
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

fn dedupe_hits(hits: &mut Vec<SearchHit>) {
    let mut seen = std::collections::HashSet::new();
    hits.retain(|hit| seen.insert(hit.url.clone()));
}
