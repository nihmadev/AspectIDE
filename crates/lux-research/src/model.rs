//! Data model for the research pipeline. All shapes are plain `serde` (camelCase)
//! so the desktop layer returns them straight across the Tauri bridge and the
//! agent reads them as a tool result.

use serde::{Deserialize, Serialize};

/// What kind of sources to bias toward — maps to SearxNG categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum FocusMode {
    /// General web search.
    #[default]
    Web,
    /// Papers / scholarly sources (SearxNG `science`).
    Academic,
    /// Recent news.
    News,
    /// Forums / social discussion.
    Social,
    /// Video results.
    Video,
    /// Code / developer sources (SearxNG `it`).
    Code,
}

impl FocusMode {
    /// SearxNG `categories` value for this focus.
    #[must_use]
    pub const fn searxng_category(self) -> &'static str {
        match self {
            Self::Web => "general",
            Self::Academic => "science",
            Self::News => "news",
            Self::Social => "social media",
            Self::Video => "videos",
            Self::Code => "it",
        }
    }
}

/// One raw result from a search provider, before page-content extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub url: String,
    pub title: String,
    /// The provider's short snippet/description.
    pub snippet: String,
    /// Which underlying engine produced the hit (best-effort).
    pub engine: String,
    /// The provider's own relevance score, when reported (else 0).
    pub provider_score: f64,
}

/// A fully ranked source: a search hit enriched with extracted page content and a
/// final blended relevance score, ready to cite.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedSource {
    /// 1-based citation index (`[1]`, `[2]`, …).
    pub rank: usize,
    pub url: String,
    pub title: String,
    pub snippet: String,
    /// Extracted, relevance-trimmed page text (empty if the fetch yielded nothing).
    pub content: String,
    /// Final blended relevance in `[0, 1]`.
    pub relevance: f64,
    pub engine: String,
}

/// Knobs for a research run.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchOptions {
    #[serde(default)]
    pub focus: FocusMode,
    /// How many ranked sources to return.
    #[serde(default = "default_max_sources")]
    pub max_sources: usize,
    /// Max characters of extracted content kept per source.
    #[serde(default = "default_max_chars")]
    pub max_chars_per_source: usize,
}

impl Default for ResearchOptions {
    fn default() -> Self {
        Self {
            focus: FocusMode::default(),
            max_sources: default_max_sources(),
            max_chars_per_source: default_max_chars(),
        }
    }
}

fn default_max_sources() -> usize {
    6
}
fn default_max_chars() -> usize {
    2_400
}

/// The assembled research result handed back to the agent.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchResponse {
    pub query: String,
    pub focus: FocusMode,
    /// `"searxng"` or `"duckduckgo"`.
    pub provider: String,
    pub source_count: usize,
    pub sources: Vec<RankedSource>,
    /// Human-facing notes (e.g. "configure SearxNG for better results").
    pub notes: Vec<String>,
}
