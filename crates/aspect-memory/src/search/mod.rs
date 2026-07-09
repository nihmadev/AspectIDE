//! Pure scoring helpers for memory retrieval: FTS5 query construction, recency
//! decay, cosine similarity, reciprocal-rank fusion across lexical/vector/graph
//! streams, the Ebbinghaus-style retention model, and the token-set Jaccard
//! similarity used by supersession-on-create and the contradiction sweep.

use std::collections::{HashMap, HashSet};

use crate::model::{Memory, RelationKind, RetentionTier};


/// Relative weights of the salience (importance/recency/frequency) blend. They
/// need not sum to 1 on their own — [`salience_blend`] renormalizes them — but
/// the pinned boost is additive on top of everything so a pinned memory always
/// outranks an unpinned one of equal relevance.
const W_IMPORTANCE: f64 = 0.25;
const W_RECENCY: f64 = 0.12;
/// Frequency ("strengthening") weight — memories recalled often rank higher,
/// the agentmemory/Ebbinghaus behavior: access reinforces retention.
const W_FREQUENCY: f64 = 0.08;
const PINNED_BOOST: f64 = 0.5;
pub(crate) const MILLIS_PER_DAY: f64 = 86_400_000.0;
/// Access count at which the frequency signal saturates (log-scaled below it).
const FREQUENCY_SATURATION: f64 = 64.0;

/// Reciprocal-rank-fusion smoothing constant (agentmemory: `k = 60`) — keeps a
/// rank-1 candidate from completely dominating a fused score.
const RRF_K: f64 = 60.0;
/// Base per-stream weights for [`RrfWeights::renormalized`] (agentmemory RRF
/// fusion), renormalized per query over whichever streams actually produced a
/// candidate.
const RRF_BASE_BM25: f64 = 0.4;
const RRF_BASE_VECTOR: f64 = 0.6;
const RRF_BASE_GRAPH: f64 = 0.3;
/// Relative weight of the fused RRF score vs. the salience blend in the final
/// rank (agentmemory: `0.7` / `0.3`).
const FINAL_RRF_WEIGHT: f64 = 0.7;
const FINAL_SALIENCE_WEIGHT: f64 = 0.3;

/// Retention-tier thresholds (agentmemory Ebbinghaus model).
const RETENTION_HOT: f64 = 0.7;
const RETENTION_WARM: f64 = 0.4;
const RETENTION_COLD: f64 = 0.15;
/// Access count at which the reinforcement term's log-scaling saturates
/// (agentmemory: `ln(65)`).
const RETENTION_ACCESS_SATURATION: f64 = 65.0;
const RETENTION_TEMPORAL_DECAY_RATE: f64 = 0.01;
const RETENTION_ACCESS_SALIENCE_PER_HIT: f64 = 0.02;
const RETENTION_ACCESS_SALIENCE_CAP: f64 = 0.2;
const RETENTION_REINFORCEMENT_WEIGHT: f64 = 0.3;

/// Jaccard token-set similarity threshold above which [`crate::MemoryStore::create`]
/// treats a new memory as superseding an older one in the same category.
pub(crate) const NEAR_DUPLICATE_THRESHOLD: f64 = 0.7;
/// Jaccard token-set similarity threshold above which [`crate::MemoryStore::sweep_contradictions`]
/// treats two memories as the same fact restated (the older one is superseded).
pub(crate) const CONTRADICTION_THRESHOLD: f64 = 0.9;

/// Build a safe FTS5 `MATCH` expression from free user text: lowercase alnum
/// tokens (length ≥ 2), each quoted and prefix-matched, OR-joined. Returns
/// `None` when the query has no usable tokens (caller should fall back to a
/// plain listing).
pub fn fts_query(raw: &str) -> Option<String> {
    let tokens: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect();
    if tokens.is_empty() {
        return None;
    }
    // Quote each token (stripping any stray quotes) and prefix-match it so partial
    // words still hit; OR-join so any token can satisfy the match.
    let parts: Vec<String> = tokens
        .iter()
        .map(|token| format!("\"{}\"*", token.replace('"', "")))
        .collect();
    Some(parts.join(" OR "))
}

/// Recency weight in `(0, 1]`: 1.0 for "just now", halving every `half_life_days`.
#[must_use]
pub fn recency_decay(age_millis: i64, half_life_days: f64) -> f64 {
    if half_life_days <= 0.0 {
        return 1.0;
    }
    let age_days = (age_millis.max(0) as f64) / MILLIS_PER_DAY;
    0.5_f64.powf(age_days / half_life_days)
}

/// Min-max normalize a slice into `[0, 1]`. When all values are equal (or the
/// slice is degenerate) every entry maps to `1.0` so a uniform candidate set is
/// not zeroed out.
#[must_use]
pub fn min_max_normalize(values: &[f64]) -> Vec<f64> {
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = max - min;
    if !span.is_finite() || span <= f64::EPSILON {
        return vec![1.0; values.len()];
    }
    values.iter().map(|value| (value - min) / span).collect()
}

/// Frequency weight in `[0, 1]`: log-scaled recall count, saturating at
/// [`FREQUENCY_SATURATION`]. A never-recalled memory contributes 0.
#[must_use]
pub fn frequency_weight(access_count: i64) -> f64 {
    if access_count <= 0 {
        return 0.0;
    }
    ((access_count as f64) + 1.0).log2() / (FREQUENCY_SATURATION + 1.0).log2()
}

/// Importance/recency/frequency blend, renormalized to `[0, 1]` over its own
/// sub-weights. The lexical/semantic signal no longer lives here — it is fused
/// via [`RrfWeights`] instead — this is purely the "how salient is this memory
/// on its own merits" half of the final rank.
#[must_use]
pub fn salience_blend(importance: f64, recency: f64, access_count: i64) -> f64 {
    let sub_total = W_IMPORTANCE + W_RECENCY + W_FREQUENCY;
    (W_IMPORTANCE * importance.clamp(0.0, 1.0)
        + W_RECENCY * recency
        + W_FREQUENCY * frequency_weight(access_count).min(1.0))
        / sub_total
}

/// Final rank: `0.7 * rrf_normalized + 0.3 * salience`, plus the pinned boost
/// so a pinned memory always outranks an unpinned one of equal relevance.
#[must_use]
pub fn blend_rrf(
    rrf_normalized: f64,
    importance: f64,
    recency: f64,
    access_count: i64,
    pinned: bool,
) -> f64 {
    let base = FINAL_RRF_WEIGHT * rrf_normalized.clamp(0.0, 1.0)
        + FINAL_SALIENCE_WEIGHT * salience_blend(importance, recency, access_count);
    if pinned {
        base + PINNED_BOOST
    } else {
        base
    }
}

/// Reciprocal rank contribution for a 1-based rank (ties share a rank — see
/// [`dense_ranks`]).
#[must_use]
pub fn rrf_term(rank: usize) -> f64 {
    1.0 / (RRF_K + rank as f64)
}

/// Assign 1-based dense ranks (ties share a rank) to a best-first-ordered score
/// sequence, so fusing a stream never manufactures an order among truly-equal
/// candidates (e.g. two rows with byte-identical BM25 scores).
#[must_use]
pub fn dense_ranks(scores_best_first: &[f64]) -> Vec<usize> {
    let mut ranks = Vec::with_capacity(scores_best_first.len());
    let mut rank = 0usize;
    let mut prev: Option<f64> = None;
    for &score in scores_best_first {
        if prev.is_none_or(|previous| (previous - score).abs() > f64::EPSILON) {
            rank += 1;
        }
        ranks.push(rank);
        prev = Some(score);
    }
    ranks
}

/// Turn a set of `(id, score)` pairs (higher score = more relevant) into a map
/// of `id -> 1-based dense rank`, the shape every RRF stream needs before
/// fusion. Ids absent from the input simply have no entry (treated as "not in
/// this stream" by [`RrfWeights::fuse`]).
#[must_use]
pub fn rank_by_score(mut scored: Vec<(String, f64)>) -> HashMap<String, usize> {
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let scores: Vec<f64> = scored.iter().map(|(_, score)| *score).collect();
    dense_ranks(&scores)
        .into_iter()
        .zip(scored)
        .map(|(rank, (id, _))| (id, rank))
        .collect()
}

/// Per-stream weights for reciprocal-rank fusion, renormalized to the streams
/// that actually produced a candidate for this query (agentmemory RRF fusion:
/// base weights `bm25=0.4 vector=0.6 graph=0.3`; an absent stream's weight is
/// redistributed rather than silently deflating the fused score).
#[derive(Debug, Clone, Copy, Default)]
pub struct RrfWeights {
    bm25: f64,
    vector: f64,
    graph: f64,
}

impl RrfWeights {
    #[must_use]
    pub fn renormalized(bm25_active: bool, vector_active: bool, graph_active: bool) -> Self {
        let total = [
            (bm25_active, RRF_BASE_BM25),
            (vector_active, RRF_BASE_VECTOR),
            (graph_active, RRF_BASE_GRAPH),
        ]
        .into_iter()
        .filter_map(|(active, weight)| active.then_some(weight))
        .sum::<f64>();
        if total <= f64::EPSILON {
            return Self::default();
        }
        Self {
            bm25: if bm25_active {
                RRF_BASE_BM25 / total
            } else {
                0.0
            },
            vector: if vector_active {
                RRF_BASE_VECTOR / total
            } else {
                0.0
            },
            graph: if graph_active {
                RRF_BASE_GRAPH / total
            } else {
                0.0
            },
        }
    }

    /// Fuse one candidate's per-stream ranks (`None` = absent from that
    /// stream) into a single reciprocal-rank-fusion score.
    #[must_use]
    pub fn fuse(
        &self,
        bm25_rank: Option<usize>,
        vector_rank: Option<usize>,
        graph_rank: Option<usize>,
    ) -> f64 {
        bm25_rank.map_or(0.0, |rank| self.bm25 * rrf_term(rank))
            + vector_rank.map_or(0.0, |rank| self.vector * rrf_term(rank))
            + graph_rank.map_or(0.0, |rank| self.graph * rrf_term(rank))
    }
}

/// Cosine similarity in `[-1, 1]`; `0.0` when either vector is empty, the lengths
/// differ, or a vector has zero magnitude.
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

/// Encode an `f32` embedding as little-endian bytes for BLOB storage.
#[must_use]
pub fn encode_embedding(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Decode a little-endian BLOB back into an `f32` embedding; trailing partial
/// bytes (corrupt/legacy rows) are ignored.
#[must_use]
pub fn decode_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// Lowercased alnum tokens of length > 2, deduplicated into a set (agentmemory
/// near-duplicate/contradiction heuristic — short, mostly-stop-word tokens are
/// excluded so they don't inflate similarity between unrelated sentences).
#[must_use]
pub fn token_set(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.chars().count() > 2)
        .map(str::to_lowercase)
        .collect()
}

/// Jaccard similarity (`|A∩B| / |A∪B|`) between two token sets; `0.0` when both
/// are empty (nothing to compare, not "identical").
#[must_use]
pub fn jaccard_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Default confidence for an inferred relation edge when the caller doesn't
/// supply one (agentmemory heuristic): start at `0.5`, then nudge for temporal
/// co-occurrence and the relation kind itself, clamped to `[0, 1]`.
#[must_use]
pub fn infer_relation_confidence(
    source: &Memory,
    target: &Memory,
    relation: RelationKind,
    now_ms: i64,
) -> f64 {
    let mut confidence: f64 = 0.5;

    let source_days_since_update = ((now_ms - source.updated_at).max(0) as f64) / MILLIS_PER_DAY;
    let target_days_since_update = ((now_ms - target.updated_at).max(0) as f64) / MILLIS_PER_DAY;
    if source_days_since_update <= 7.0 && target_days_since_update <= 7.0 {
        confidence += 0.1;
    }

    let source_days_idle = ((now_ms - source.last_accessed_at).max(0) as f64) / MILLIS_PER_DAY;
    let target_days_idle = ((now_ms - target.last_accessed_at).max(0) as f64) / MILLIS_PER_DAY;
    if source_days_idle > 90.0 && target_days_idle > 90.0 {
        confidence -= 0.1;
    }

    match relation {
        RelationKind::Supersedes => confidence += 0.1,
        RelationKind::Contradicts => confidence -= 0.05,
        RelationKind::Extends | RelationKind::Derives | RelationKind::Related => {}
    }

    confidence.clamp(0.0, 1.0)
}

/// Per-category base salience for [`retention_score`]: how "sticky" a memory
/// is by default, before importance/access lift it.
fn category_base_salience(category: &str) -> f64 {
    match category {
        "core" => 0.9,
        "procedural" => 0.8,
        "semantic" => 0.7,
        "episodic" => 0.5,
        _ => 0.6,
    }
}

/// Ebbinghaus-style forgetting-curve retention score in `[0, 1]`: temporal
/// decay pulls a memory toward forgotten; category/importance/access salience
/// and recall-frequency reinforcement pull it back. Drives `prune`'s overflow
/// eviction order (weakest retention goes first) and [`crate::model::RetentionReport`].
#[must_use]
pub fn retention_score(memory: &Memory, now_ms: i64) -> f64 {
    let days_since_created = ((now_ms - memory.created_at).max(0) as f64) / MILLIS_PER_DAY;
    let days_since_accessed = ((now_ms - memory.last_accessed_at).max(0) as f64) / MILLIS_PER_DAY;
    let access_count = memory.access_count.max(0) as f64;

    let temporal_decay = (-RETENTION_TEMPORAL_DECAY_RATE * days_since_created).exp();
    let salience = category_base_salience(&memory.category).max(memory.importance.clamp(0.0, 1.0))
        + (access_count * RETENTION_ACCESS_SALIENCE_PER_HIT).min(RETENTION_ACCESS_SALIENCE_CAP);
    let access_reinforcement_ratio =
        ((1.0 + access_count).ln() / RETENTION_ACCESS_SATURATION.ln()).min(1.0);
    let reinforcement =
        RETENTION_REINFORCEMENT_WEIGHT * access_reinforcement_ratio / days_since_accessed.max(1.0);

    (salience * temporal_decay + reinforcement).min(1.0)
}

/// Retention tier for a score returned by [`retention_score`] (agentmemory:
/// hot ≥ 0.7, warm ≥ 0.4, cold ≥ 0.15, else evictable).
#[must_use]
pub fn retention_tier(score: f64) -> RetentionTier {
    if score >= RETENTION_HOT {
        RetentionTier::Hot
    } else if score >= RETENTION_WARM {
        RetentionTier::Warm
    } else if score >= RETENTION_COLD {
        RetentionTier::Cold
    } else {
        RetentionTier::Evictable
    }
}
