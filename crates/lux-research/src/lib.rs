//! `lux-research` — the pure core of a Perplexica-style web research engine.
//!
//! It turns a query into a ranked, extracted set of cited sources: build a search
//! URL for a [`FocusMode`], parse the provider's results
//! ([SearxNG](https://docs.searxng.org) JSON or a DuckDuckGo HTML fallback), then
//! [`rerank`] the fetched pages by lexical relevance to the query. All of it is
//! pure (no network, no I/O) so the desktop layer owns only the HTTP + the agent
//! tool; the model writes the final cited answer from the ranked sources.

mod model;
mod provider;
mod rerank;

pub use model::{
    DepthProfile, FocusMode, RankedSource, ResearchDepth, ResearchOptions, ResearchResponse,
    SearchHit,
};
pub use provider::{
    duckduckgo_lite_search_url, duckduckgo_search_url, expand_queries, extract_result_links,
    parse_duckduckgo_html, parse_searxng_json, searxng_search_url, validate_source_url,
};
pub use rerank::{rerank, rerank_deep};
