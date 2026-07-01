//! Web research engine — the network glue around [`lux_research`].
//!
//! A `Perplexica`-style pipeline exposed as an IDE/agent tool, deliberately
//! SEPARATE from `WebFetch` (single URL) and any built-in web search: it runs a
//! provider search (`SearxNG` JSON, or a keyless `DuckDuckGo` HTML fallback), fetches
//! the top result pages concurrently through the SSRF-guarded [`crate::web_fetch`]
//! path, reranks them by lexical relevance, and returns ranked, cited sources for
//! the model to synthesize an answer from. It never calls into `web_fetch::fetch`'s
//! command surface, so the existing `WebFetch` tool is untouched.

use lux_research::{
    duckduckgo_lite_search_url, duckduckgo_search_url, parse_duckduckgo_html, parse_searxng_json,
    rerank, searxng_search_url, FocusMode, ResearchOptions, ResearchResponse, SearchHit,
};
use tauri::State;

use crate::{web_fetch, SharedState};

/// Settings key (user scope) holding the optional `SearxNG` base URL.
pub const SEARXNG_URL_KEY: &str = "ai.research.searxngUrl";

const SEARCH_TIMEOUT_SECS: u64 = 18;
const SEARCH_MAX_BYTES: usize = 600_000;
const PAGE_TIMEOUT_SECS: u64 = 15;
const PAGE_MAX_BYTES: u64 = 200_000;
/// Cap on pages fetched per run, regardless of requested `max_sources`.
const MAX_PAGE_FETCHES: usize = 8;

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
    let searxng = configured_searxng_url(&state);
    let mut notes = Vec::new();

    // ── 1. Provider search (SearxNG primary, DuckDuckGo fallback) ──
    let (provider, mut hits) = if let Some(base) = &searxng {
        match search_searxng(base, &query, &options).await {
            Ok(hits) if !hits.is_empty() => ("searxng", hits),
            Ok(_) => {
                notes.push("SearxNG returned no results; fell back to DuckDuckGo.".to_string());
                ("duckduckgo", search_duckduckgo(&query).await?)
            }
            Err(error) => {
                notes.push(format!(
                    "SearxNG search failed ({error}); fell back to DuckDuckGo."
                ));
                ("duckduckgo", search_duckduckgo(&query).await?)
            }
        }
    } else {
        notes.push(
            "Using the keyless DuckDuckGo fallback. Configure a SearxNG instance in Settings → AI → Web Research for richer, focus-aware results.".to_string(),
        );
        ("duckduckgo", search_duckduckgo(&query).await?)
    };

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

    if hits.is_empty() {
        return Ok(ResearchResponse {
            query,
            focus: effective_focus,
            provider: provider.to_string(),
            source_count: 0,
            sources: Vec::new(),
            notes: {
                notes.push("No search results found.".to_string());
                notes
            },
        });
    }

    // Dedupe by URL (preserve first/highest-placed occurrence) and cap fetches.
    dedupe_hits(&mut hits);
    let fetch_count = options
        .max_sources
        .clamp(1, MAX_PAGE_FETCHES)
        .min(hits.len());
    let to_fetch = &hits[..fetch_count];

    // ── 2. Fetch result pages concurrently (SSRF-guarded) ──
    let fetches = to_fetch.iter().map(|hit| {
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
    let fetched: Vec<(String, Option<String>)> = futures_util::future::join_all(fetches).await;

    // Backfill a missing snippet from the page title when the provider gave none.
    let hits_for_rank: Vec<SearchHit> = to_fetch
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

    // ── 3. Rerank by lexical relevance to the query ──
    let sources = rerank(
        &query,
        &hits_for_rank,
        &contents,
        options.max_sources,
        options.max_chars_per_source,
    );

    // Signal when fewer sources came back than requested, distinguishing
    // "fetched pages were dropped as off-topic" (rerank filtered them) from
    // "there simply weren't that many candidates" — the model needs this to
    // decide whether to broaden the query rather than assume results are exhausted.
    let requested = options.max_sources.max(1);
    let returned = sources.len();
    if returned < requested {
        let dropped = fetch_count.saturating_sub(returned);
        if dropped > 0 {
            notes.push(format!(
                "Requested up to {requested} sources; returned {returned}. {dropped} fetched page(s) shared no query terms and were dropped as off-topic — consider broadening or rephrasing the query."
            ));
        } else {
            notes.push(format!(
                "Requested up to {requested} sources; only {fetch_count} candidate result(s) were available to fetch. Consider broadening the query."
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

async fn search_duckduckgo(query: &str) -> Result<Vec<SearchHit>, String> {
    let body = web_fetch::fetch_text(
        &duckduckgo_search_url(query),
        "text/html",
        SEARCH_TIMEOUT_SECS,
        SEARCH_MAX_BYTES,
        false,
    )
    .await?;
    let hits = parse_duckduckgo_html(&body);
    if !hits.is_empty() {
        return Ok(hits);
    }
    // The full page returned nothing parseable (markup change / bot wall) — retry
    // the minimal, JS-free lite endpoint, which is far more stable to scrape.
    let lite = web_fetch::fetch_text(
        &duckduckgo_lite_search_url(query),
        "text/html",
        SEARCH_TIMEOUT_SECS,
        SEARCH_MAX_BYTES,
        false,
    )
    .await?;
    Ok(parse_duckduckgo_html(&lite))
}

fn dedupe_hits(hits: &mut Vec<SearchHit>) {
    let mut seen = std::collections::HashSet::new();
    hits.retain(|hit| seen.insert(hit.url.clone()));
}
