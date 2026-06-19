//! Data model for the memory store: the stored [`Memory`], the create/update
//! inputs, search options, and the scored/aggregate result shapes. All shapes are
//! plain `serde` (camelCase) so the desktop layer can return them straight across
//! the Tauri bridge and the frontend can type them by hand.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default importance assigned to a memory when the caller does not specify one.
pub const DEFAULT_IMPORTANCE: f64 = 0.5;

/// One durable memory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    /// Stable unique id (uuid v4 unless the caller supplied one).
    pub id: String,
    /// Namespace this memory lives in (e.g. `core`, `episodic`, `semantic`, `procedural`, or custom).
    pub category: String,
    /// The memory text itself.
    pub content: String,
    /// Arbitrary JSON metadata object (always an object; defaults to `{}`).
    pub metadata: Value,
    /// Relevance weight in `[0, 1]`; higher surfaces earlier in recall.
    pub importance: f64,
    /// Pinned memories always rank first and are exempt from importance decay.
    pub pinned: bool,
    /// Where this memory came from (e.g. `agent`, `user`, a tool name, a file path).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Epoch-millis creation / last-update / last-recall timestamps.
    pub created_at: i64,
    pub updated_at: i64,
    pub last_accessed_at: i64,
    /// How many times this memory has been recalled (bumped on search hits).
    pub access_count: i64,
    /// Whether an embedding vector is stored (the raw vector is never serialized).
    pub has_embedding: bool,
}

/// Input for creating a memory. Only `category` and `content` are required.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMemory {
    pub category: String,
    pub content: String,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub importance: Option<f64>,
    #[serde(default)]
    pub pinned: Option<bool>,
    #[serde(default)]
    pub source: Option<String>,
    /// Supply a fixed id to upsert a known memory; otherwise a uuid v4 is minted.
    #[serde(default)]
    pub id: Option<String>,
    /// Optional embedding vector for hybrid (lexical + cosine) retrieval.
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
}

/// Partial update; every field is optional and only present fields are written.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPatch {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub importance: Option<f64>,
    #[serde(default)]
    pub pinned: Option<bool>,
    #[serde(default)]
    pub source: Option<String>,
}

/// Ordering for plain (non-search) listing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SortOrder {
    /// Pinned first, then importance, then most-recently-accessed (the default recall order).
    #[default]
    Relevance,
    /// Most recently updated first.
    Recent,
    /// Highest importance first.
    Importance,
    /// Oldest first.
    Oldest,
}

/// Options controlling [`MemoryStore::search`] and [`MemoryStore::list`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    /// Restrict to a single category when set.
    #[serde(default)]
    pub category: Option<String>,
    /// Maximum number of results.
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Offset for plain listing (ignored by ranked search).
    #[serde(default)]
    pub offset: usize,
    /// Drop results scoring below this threshold (ranked search only).
    #[serde(default)]
    pub min_score: Option<f64>,
    /// Half-life (days) for recency decay in scoring.
    #[serde(default = "default_half_life")]
    pub recency_half_life_days: f64,
    /// Sort order for plain listing.
    #[serde(default)]
    pub sort: SortOrder,
    /// Always include pinned memories in ranked search, even without a lexical hit.
    #[serde(default = "default_true")]
    pub include_pinned: bool,
    /// Bump `last_accessed_at`/`access_count` for the returned memories.
    #[serde(default = "default_true")]
    pub touch: bool,
    /// Optional query embedding for hybrid cosine scoring.
    #[serde(default)]
    pub query_embedding: Option<Vec<f32>>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            category: None,
            limit: default_limit(),
            offset: 0,
            min_score: None,
            recency_half_life_days: default_half_life(),
            sort: SortOrder::default(),
            include_pinned: true,
            touch: true,
            query_embedding: None,
        }
    }
}

fn default_limit() -> usize {
    8
}
fn default_half_life() -> f64 {
    30.0
}
fn default_true() -> bool {
    true
}

/// A memory with its blended retrieval score and the lexical sub-score.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoredMemory {
    #[serde(flatten)]
    pub memory: Memory,
    /// Final blended score (lexical + importance + recency + pinned boost).
    pub score: f64,
    /// Normalized lexical relevance sub-score in `[0, 1]`.
    pub lexical: f64,
}

/// Aggregate counts for a project's memory, for the UI header / stats card.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStats {
    pub total: usize,
    pub pinned: usize,
    pub by_category: Vec<CategoryCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated_at: Option<i64>,
}

/// Per-category count for [`MemoryStats`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryCount {
    pub category: String,
    pub count: usize,
}
