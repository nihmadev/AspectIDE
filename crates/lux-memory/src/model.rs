//! Data model for the memory store: the stored [`Memory`], the create/update
//! inputs, search options, the scored/aggregate result shapes, and the
//! knowledge-graph-lite relation types. All shapes are plain `serde` (camelCase)
//! so the desktop layer can return them straight across the Tauri bridge and the
//! frontend can type them by hand.

use std::str::FromStr;

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
    /// Set when a newer memory has superseded this one (supersession-on-create /
    /// contradiction sweep). Superseded rows are excluded from search/list unless
    /// `SearchOptions::include_superseded` is set.
    pub superseded: bool,
    /// TTL cutoff (epoch millis); the memory is hard-deleted by `prune` once now
    /// passes this, pinned status notwithstanding. `None` means no expiry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forget_after: Option<i64>,
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
    /// Optional time-to-live in days; `prune` hard-deletes the row once it
    /// expires. Pinning still wins over an expired TTL (pin beats TTL, exactly
    /// as pin beats staleness/overflow eviction).
    #[serde(default)]
    pub ttl_days: Option<f64>,
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
    /// Include rows marked `superseded` (by supersession-on-create or the
    /// contradiction sweep). Off by default: a superseded memory is stale by
    /// definition and should not compete with its replacement.
    #[serde(default)]
    pub include_superseded: bool,
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
            include_superseded: false,
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

/// Outcome of [`crate::MemoryStore::create`]: the written memory plus the ids of
/// any older memories it superseded (agentmemory mechanism 1 — near-duplicate
/// replacement within a category), so callers can surface "this replaced N
/// older memories" to the agent/user.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateOutcome {
    #[serde(flatten)]
    pub memory: Memory,
    pub superseded_ids: Vec<String>,
}

/// Kind of edge in the knowledge-graph-lite `memory_relations` table.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RelationKind {
    /// The source memory replaces/invalidates the target (newer fact wins).
    Supersedes,
    /// The source memory adds detail to the target without replacing it.
    Extends,
    /// The source memory was derived/inferred from the target.
    Derives,
    /// The source and target memories conflict.
    Contradicts,
    /// A generic, otherwise-unclassified association.
    Related,
}

impl RelationKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supersedes => "supersedes",
            Self::Extends => "extends",
            Self::Derives => "derives",
            Self::Contradicts => "contradicts",
            Self::Related => "related",
        }
    }
}

impl std::fmt::Display for RelationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RelationKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "supersedes" => Ok(Self::Supersedes),
            "extends" => Ok(Self::Extends),
            "derives" => Ok(Self::Derives),
            "contradicts" => Ok(Self::Contradicts),
            "related" => Ok(Self::Related),
            other => Err(format!("unknown relation kind: {other}")),
        }
    }
}

/// A directed edge between two memories in the knowledge-graph-lite.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRelation {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation: RelationKind,
    /// Confidence in `[0, 1]`.
    pub confidence: f64,
    pub created_at: i64,
}

/// One hop-reachable memory returned by [`crate::MemoryStore::related`]: the
/// memory itself plus how far it is from the query memory and the path
/// confidence (the product of the edge confidences traversed to reach it).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedMemory {
    #[serde(flatten)]
    pub memory: Memory,
    pub hops: usize,
    /// Product of edge confidences along the shortest/first BFS path found.
    pub path_confidence: f64,
}

/// One pair of near-duplicate memories found by [`crate::MemoryStore::sweep_contradictions`],
/// where the older row was marked superseded and a `contradicts` relation was recorded.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContradictionHit {
    /// The row kept (the more recently updated of the pair).
    pub kept_id: String,
    /// The row marked superseded (the older of the pair).
    pub superseded_id: String,
    /// Token-set Jaccard similarity that triggered the match.
    pub similarity: f64,
}

/// Retention tier for [`crate::search::retention_score`] (Ebbinghaus-style forgetting curve).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RetentionTier {
    Hot,
    Warm,
    Cold,
    Evictable,
}

/// Aggregate retention-tier counts for the whole store, for a UI health card.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionReport {
    pub hot: usize,
    pub warm: usize,
    pub cold: usize,
    pub evictable: usize,
}
