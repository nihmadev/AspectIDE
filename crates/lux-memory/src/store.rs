//! SQLite-backed memory store. One [`MemoryStore`] owns one connection to one
//! project's database file (FTS5 for lexical match, an `embedding` BLOB column
//! for optional hybrid scoring, and a `memory_relations` edge table for the
//! knowledge-graph-lite). All retrieval ranking lives in [`crate::search`].

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::str::FromStr;

use lux_core::{AppError, AppResult};
use rusqlite::{
    params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension, Row,
};
use uuid::Uuid;

use crate::model::{
    CategoryCount, ContradictionHit, CreateOutcome, Memory, MemoryPatch, MemoryRelation,
    MemoryStats, NewMemory, RelatedMemory, RelationKind, RetentionReport, RetentionTier,
    ScoredMemory, SearchOptions, SortOrder, DEFAULT_IMPORTANCE,
};
use crate::search::{
    blend_rrf, cosine_similarity, decode_embedding, encode_embedding, fts_query,
    infer_relation_confidence, jaccard_similarity, min_max_normalize, rank_by_score, recency_decay,
    retention_score, retention_tier, token_set, RrfWeights, CONTRADICTION_THRESHOLD,
    MILLIS_PER_DAY, NEAR_DUPLICATE_THRESHOLD,
};

/// Schema generation stamped into `PRAGMA user_version`. Bumped whenever a
/// migration adds columns/tables an older on-disk database won't have yet.
const SCHEMA_VERSION: i64 = 1;

// Split in two: `SCHEMA_TABLES` creates just the `memories` table (so `migrate`
// can run its `ALTER TABLE` backfill on a pre-existing v0 table before anything
// else touches it), then `SCHEMA_REST` adds the indexes/FTS/triggers/relations
// table that reference the now-guaranteed-present `superseded`/`forget_after`
// columns. A fresh database gets `superseded`/`forget_after` directly from
// `SCHEMA_TABLES`, so `migrate`'s `ALTER TABLE`s are a tolerated no-op on it.
const SCHEMA_TABLES: &str = "
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;

CREATE TABLE IF NOT EXISTS memories (
  rowid INTEGER PRIMARY KEY AUTOINCREMENT,
  id TEXT NOT NULL UNIQUE,
  category TEXT NOT NULL,
  content TEXT NOT NULL,
  metadata TEXT NOT NULL DEFAULT '{}',
  importance REAL NOT NULL DEFAULT 0.5,
  pinned INTEGER NOT NULL DEFAULT 0,
  source TEXT,
  embedding BLOB,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  last_accessed_at INTEGER NOT NULL,
  access_count INTEGER NOT NULL DEFAULT 0,
  superseded INTEGER NOT NULL DEFAULT 0,
  forget_after INTEGER
);
";

const SCHEMA_REST: &str = "
CREATE INDEX IF NOT EXISTS idx_memories_category ON memories(category);
CREATE INDEX IF NOT EXISTS idx_memories_pinned ON memories(pinned);
CREATE INDEX IF NOT EXISTS idx_memories_superseded ON memories(superseded);

CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
  content, category, content='memories', content_rowid='rowid'
);

CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
  INSERT INTO memories_fts(rowid, content, category) VALUES (new.rowid, new.content, new.category);
END;
CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
  INSERT INTO memories_fts(memories_fts, rowid, content, category) VALUES('delete', old.rowid, old.content, old.category);
END;
CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE OF content, category ON memories BEGIN
  INSERT INTO memories_fts(memories_fts, rowid, content, category) VALUES('delete', old.rowid, old.content, old.category);
  INSERT INTO memories_fts(rowid, content, category) VALUES (new.rowid, new.content, new.category);
END;

CREATE TABLE IF NOT EXISTS memory_relations (
  id TEXT PRIMARY KEY,
  source_id TEXT NOT NULL,
  target_id TEXT NOT NULL,
  relation TEXT NOT NULL,
  confidence REAL NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE(source_id, target_id, relation)
);
CREATE INDEX IF NOT EXISTS idx_memory_relations_source ON memory_relations(source_id);
CREATE INDEX IF NOT EXISTS idx_memory_relations_target ON memory_relations(target_id);
";

/// Hard ceiling for a single plain `list` page. A model/tool-driven request can
/// ask for a huge `limit`; without a cap one call could read and allocate the
/// entire memory table. Pagination via `offset` still walks the whole store.
const MAX_LIST_LIMIT: usize = 500;
/// Max embedded rows scanned when merging a semantic-recall pool, so hybrid
/// search stays bounded even on a large store with many embeddings.
const EMBEDDED_SCAN_CAP: usize = 512;
/// Max same-category rows scanned for near-duplicate supersession on a fresh
/// create (agentmemory mechanism 1), so a huge category can't make every
/// `create` call linear in its size.
const NEAR_DUP_SCAN_LIMIT: usize = 512;
/// How many of the strongest fused-so-far candidates seed the graph stream's
/// 1-hop relation expansion in [`MemoryStore::search`].
const GRAPH_SEED_COUNT: usize = 5;
/// Visited-node cap for [`MemoryStore::related`]'s BFS, so a densely-connected
/// graph can't make one call scan the whole store.
const RELATED_VISITED_CAP: usize = 500;

/// Unqualified column list for reads against `memories` alone.
const COLS: &str = "id, category, content, metadata, importance, pinned, source, created_at, updated_at, last_accessed_at, access_count, superseded, forget_after, (embedding IS NOT NULL) AS has_embedding";
/// `memories.`-qualified column list for joins against `memories_fts`
/// (both tables expose `content`/`category`, so they must be disambiguated).
const COLS_Q: &str = "memories.id, memories.category, memories.content, memories.metadata, memories.importance, memories.pinned, memories.source, memories.created_at, memories.updated_at, memories.last_accessed_at, memories.access_count, memories.superseded, memories.forget_after, (memories.embedding IS NOT NULL) AS has_embedding";

/// A SQLite-backed durable memory store for one project.
pub struct MemoryStore {
    conn: Connection,
}

/// A retrieval candidate mid-scoring: the memory plus whatever signal each
/// stream (lexical, vector, graph) resolved for it. `None` means "this
/// candidate never surfaced in that stream" — the RRF fusion in
/// [`MemoryStore::search`] treats an absent stream as a non-vote, not a zero.
struct Candidate {
    memory: Memory,
    /// Raw (negated) BM25 rank from FTS5, higher = stronger lexical match.
    raw_lex: Option<f64>,
    /// Normalized lexical sub-score reported on [`ScoredMemory::lexical`] —
    /// independent of the RRF fusion that drives the final `score`.
    lexical: f64,
    /// Cosine similarity to the query embedding.
    embed_sim: Option<f64>,
    /// Best (max) confidence of a relation edge reaching this candidate from
    /// one of the graph stream's seed nodes.
    graph_score: Option<f64>,
}

impl Candidate {
    fn from_memory(memory: Memory) -> Self {
        Self {
            memory,
            raw_lex: None,
            lexical: 0.0,
            embed_sim: None,
            graph_score: None,
        }
    }

    fn bm25_entry(&self) -> Option<(String, f64)> {
        self.raw_lex.map(|score| (self.memory.id.clone(), score))
    }

    fn vector_entry(&self) -> Option<(String, f64)> {
        self.embed_sim.map(|score| (self.memory.id.clone(), score))
    }

    fn graph_entry(&self) -> Option<(String, f64)> {
        self.graph_score
            .map(|score| (self.memory.id.clone(), score))
    }
}

impl MemoryStore {
    /// Open (creating + migrating) the store at `path`.
    pub fn open<P: AsRef<Path>>(path: P) -> AppResult<Self> {
        let conn = Connection::open(path).map_err(to_service)?;
        Self::initialize(conn)
    }

    /// Open an ephemeral in-memory store (used by tests).
    pub fn open_in_memory() -> AppResult<Self> {
        let conn = Connection::open_in_memory().map_err(to_service)?;
        Self::initialize(conn)
    }

    fn initialize(conn: Connection) -> AppResult<Self> {
        conn.execute_batch(SCHEMA_TABLES).map_err(to_service)?;
        migrate(&conn)?;
        conn.execute_batch(SCHEMA_REST).map_err(to_service)?;
        Ok(Self { conn })
    }

    /// Create a memory (or upsert when `input.id` names an existing one).
    ///
    /// Content dedup (agentmemory behavior): when no explicit id is supplied and
    /// an entry with the same category + exact content already exists, the
    /// existing memory is REINFORCED instead of duplicated — importance rises to
    /// the max of the two, pin/source/metadata merge, and the timestamps bump.
    /// Re-remembering the same fact must strengthen it, not clone it.
    ///
    /// See [`Self::create_with_outcome`] for the near-duplicate supersession
    /// that also runs on a fresh (non-upsert) insert.
    pub fn create(&self, input: NewMemory) -> AppResult<Memory> {
        Ok(self.create_with_outcome(input)?.memory)
    }

    /// Like [`Self::create`], but also returns the ids of any older,
    /// same-category memories the new one superseded (agentmemory mechanism 1:
    /// near-duplicate replacement). Empty when the write took the exact-content
    /// reinforce path, when an explicit id was supplied (an upsert isn't a
    /// "fresh" memory), or when nothing in the category was similar enough.
    pub fn create_with_outcome(&self, input: NewMemory) -> AppResult<CreateOutcome> {
        let now = now_millis();
        if input.id.is_none() {
            let duplicate: Option<Memory> = self
                .conn
                .query_row(
                    &format!(
                        "SELECT {COLS} FROM memories WHERE category = ?1 AND content = ?2 LIMIT 1"
                    ),
                    params![input.category, input.content],
                    row_to_memory,
                )
                .optional()
                .map_err(to_service)?;
            if let Some(existing) = duplicate {
                let importance = input
                    .importance
                    .unwrap_or(DEFAULT_IMPORTANCE)
                    .clamp(0.0, 1.0)
                    .max(existing.importance);
                let pinned = i64::from(existing.pinned || input.pinned.unwrap_or(false));
                let source = input.source.or(existing.source);
                let forget_after = input
                    .ttl_days
                    .map(|days| now + (days.max(0.0) * MILLIS_PER_DAY) as i64);
                // A fresh re-remember of the exact fact means the caller believes it is
                // current again — clear any earlier supersession so it stays recallable.
                self.conn
                    .execute(
                        "UPDATE memories SET importance=?2, pinned=?3, source=?4, updated_at=?5, last_accessed_at=?5, access_count=access_count+1,
                         forget_after = COALESCE(?6, forget_after), superseded=0 WHERE id=?1",
                        params![existing.id, importance, pinned, source, now, forget_after],
                    )
                    .map_err(to_service)?;
                if let Some(embedding) = input.embedding.as_deref() {
                    self.set_embedding(&existing.id, embedding)?;
                }
                return Ok(CreateOutcome {
                    memory: self.require(&existing.id)?,
                    superseded_ids: Vec::new(),
                });
            }
        }
        let is_fresh = input.id.is_none();
        let id = input
            .id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let metadata = normalize_metadata(input.metadata);
        let metadata_text = serde_json::to_string(&metadata).map_err(to_service)?;
        let importance = input
            .importance
            .unwrap_or(DEFAULT_IMPORTANCE)
            .clamp(0.0, 1.0);
        let pinned = i64::from(input.pinned.unwrap_or(false));
        let embedding = input.embedding.as_deref().map(encode_embedding);
        let forget_after = input
            .ttl_days
            .map(|days| now + (days.max(0.0) * MILLIS_PER_DAY) as i64);
        self.conn
            .execute(
                "INSERT INTO memories
                   (id, category, content, metadata, importance, pinned, source, embedding, forget_after, created_at, updated_at, last_accessed_at, access_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?10, 0)
                 ON CONFLICT(id) DO UPDATE SET
                   category = excluded.category,
                   content = excluded.content,
                   metadata = excluded.metadata,
                   importance = excluded.importance,
                   pinned = excluded.pinned,
                   source = excluded.source,
                   embedding = COALESCE(excluded.embedding, memories.embedding),
                   forget_after = COALESCE(excluded.forget_after, memories.forget_after),
                   updated_at = excluded.updated_at",
                params![
                    id,
                    input.category,
                    input.content,
                    metadata_text,
                    importance,
                    pinned,
                    input.source,
                    embedding,
                    forget_after,
                    now
                ],
            )
            .map_err(to_service)?;
        let memory = self.require(&id)?;

        let superseded_ids = if is_fresh {
            self.supersede_near_duplicates(&memory, now)?
        } else {
            Vec::new()
        };
        Ok(CreateOutcome {
            memory,
            superseded_ids,
        })
    }

    /// After a fresh insert, mark older same-category rows the new memory makes
    /// redundant (token-set Jaccard similarity over [`NEAR_DUPLICATE_THRESHOLD`])
    /// as `superseded` and link them with a `supersedes` relation, so they stop
    /// competing with the new memory in search. Bounded to the most recently
    /// updated [`NEAR_DUP_SCAN_LIMIT`] rows in the category.
    fn supersede_near_duplicates(&self, memory: &Memory, now: i64) -> AppResult<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {COLS} FROM memories WHERE category = ?1 AND id != ?2 AND superseded = 0
                 ORDER BY updated_at DESC LIMIT {NEAR_DUP_SCAN_LIMIT}"
            ))
            .map_err(to_service)?;
        let rows = stmt
            .query_map(params![memory.category, memory.id], row_to_memory)
            .map_err(to_service)?;
        let candidates = collect(rows)?;

        let new_tokens = token_set(&memory.content);
        let mut superseded_ids = Vec::new();
        for candidate in candidates {
            let similarity = jaccard_similarity(&new_tokens, &token_set(&candidate.content));
            if similarity > NEAR_DUPLICATE_THRESHOLD {
                self.conn
                    .execute(
                        "UPDATE memories SET superseded = 1 WHERE id = ?1",
                        params![candidate.id],
                    )
                    .map_err(to_service)?;
                let confidence =
                    infer_relation_confidence(memory, &candidate, RelationKind::Supersedes, now);
                self.upsert_relation(
                    &memory.id,
                    &candidate.id,
                    RelationKind::Supersedes,
                    confidence,
                    now,
                )?;
                superseded_ids.push(candidate.id);
            }
        }
        Ok(superseded_ids)
    }

    /// Fetch a single memory by id.
    pub fn get(&self, id: &str) -> AppResult<Option<Memory>> {
        self.conn
            .query_row(
                &format!("SELECT {COLS} FROM memories WHERE id = ?1"),
                params![id],
                row_to_memory,
            )
            .optional()
            .map_err(to_service)
    }

    /// Apply a partial update; errors with `NotFound` when the id is unknown.
    pub fn update(&self, id: &str, patch: MemoryPatch) -> AppResult<Memory> {
        let existing = self
            .get(id)?
            .ok_or_else(|| AppError::NotFound(format!("memory {id}")))?;
        let category = patch.category.unwrap_or(existing.category);
        let content = patch.content.unwrap_or(existing.content);
        let metadata = match patch.metadata {
            Some(value) => normalize_metadata(Some(value)),
            None => existing.metadata,
        };
        let importance = patch
            .importance
            .unwrap_or(existing.importance)
            .clamp(0.0, 1.0);
        let pinned = i64::from(patch.pinned.unwrap_or(existing.pinned));
        let source = if patch.source.is_some() {
            patch.source
        } else {
            existing.source
        };
        let metadata_text = serde_json::to_string(&metadata).map_err(to_service)?;
        let now = now_millis();
        self.conn
            .execute(
                "UPDATE memories SET category=?2, content=?3, metadata=?4, importance=?5, pinned=?6, source=?7, updated_at=?8 WHERE id=?1",
                params![id, category, content, metadata_text, importance, pinned, source, now],
            )
            .map_err(to_service)?;
        self.require(id)
    }

    /// Delete a memory (and any relation edges touching it); returns whether a
    /// row was removed.
    pub fn delete(&self, id: &str) -> AppResult<bool> {
        self.conn
            .execute(
                "DELETE FROM memory_relations WHERE source_id = ?1 OR target_id = ?1",
                params![id],
            )
            .map_err(to_service)?;
        let removed = self
            .conn
            .execute("DELETE FROM memories WHERE id=?1", params![id])
            .map_err(to_service)?;
        Ok(removed > 0)
    }

    /// Attach/replace an embedding vector for hybrid search.
    pub fn set_embedding(&self, id: &str, embedding: &[f32]) -> AppResult<()> {
        self.conn
            .execute(
                "UPDATE memories SET embedding=?2 WHERE id=?1",
                params![id, encode_embedding(embedding)],
            )
            .map_err(to_service)?;
        Ok(())
    }

    /// Insert (or refresh the confidence of, on a duplicate `(source, target,
    /// relation)` triple) a directed edge in the knowledge-graph-lite. Both
    /// memories must already exist and must differ. `confidence` defaults to
    /// [`crate::search::infer_relation_confidence`]'s heuristic when not
    /// supplied.
    pub fn relate(
        &self,
        source_id: &str,
        target_id: &str,
        kind: RelationKind,
        confidence: Option<f64>,
    ) -> AppResult<MemoryRelation> {
        if source_id == target_id {
            return Err(AppError::Service(
                "a memory cannot relate to itself".to_string(),
            ));
        }
        let source = self
            .get(source_id)?
            .ok_or_else(|| AppError::NotFound(format!("memory {source_id}")))?;
        let target = self
            .get(target_id)?
            .ok_or_else(|| AppError::NotFound(format!("memory {target_id}")))?;
        let now = now_millis();
        let confidence =
            confidence.unwrap_or_else(|| infer_relation_confidence(&source, &target, kind, now));
        self.upsert_relation(source_id, target_id, kind, confidence, now)
    }

    fn upsert_relation(
        &self,
        source_id: &str,
        target_id: &str,
        kind: RelationKind,
        confidence: f64,
        now: i64,
    ) -> AppResult<MemoryRelation> {
        let confidence = confidence.clamp(0.0, 1.0);
        let id = Uuid::new_v4().to_string();
        self.conn
            .execute(
                "INSERT INTO memory_relations (id, source_id, target_id, relation, confidence, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(source_id, target_id, relation) DO UPDATE SET confidence = excluded.confidence",
                params![id, source_id, target_id, kind.as_str(), confidence, now],
            )
            .map_err(to_service)?;
        self.conn
            .query_row(
                "SELECT id, source_id, target_id, relation, confidence, created_at
                 FROM memory_relations WHERE source_id = ?1 AND target_id = ?2 AND relation = ?3",
                params![source_id, target_id, kind.as_str()],
                row_to_relation,
            )
            .map_err(to_service)
    }

    /// Remove a relation edge by id; returns whether a row was removed.
    pub fn unrelate(&self, relation_id: &str) -> AppResult<bool> {
        let removed = self
            .conn
            .execute(
                "DELETE FROM memory_relations WHERE id = ?1",
                params![relation_id],
            )
            .map_err(to_service)?;
        Ok(removed > 0)
    }

    /// All relation edges touching `memory_id`, either as source or target.
    pub fn relations_of(&self, memory_id: &str) -> AppResult<Vec<MemoryRelation>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, source_id, target_id, relation, confidence, created_at
                 FROM memory_relations WHERE source_id = ?1 OR target_id = ?1
                 ORDER BY created_at DESC",
            )
            .map_err(to_service)?;
        let rows = stmt
            .query_map(params![memory_id], row_to_relation)
            .map_err(to_service)?;
        collect(rows)
    }

    /// BFS outward from `id` through `memory_relations` (either direction) up
    /// to `max_hops` (clamped to `1..=5`), returning memories reached with a
    /// combined path confidence (the product of the edge confidences along the
    /// first BFS path found) at or above `min_confidence`. Bounded to
    /// [`RELATED_VISITED_CAP`] visited nodes. The start memory itself is never
    /// included.
    pub fn related(
        &self,
        id: &str,
        max_hops: usize,
        min_confidence: f64,
    ) -> AppResult<Vec<RelatedMemory>> {
        let max_hops = max_hops.clamp(1, 5);
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(id.to_string());
        let mut frontier: Vec<(String, f64)> = vec![(id.to_string(), 1.0)];
        let mut results: Vec<RelatedMemory> = Vec::new();

        for hop in 1..=max_hops {
            if frontier.is_empty() || visited.len() >= RELATED_VISITED_CAP {
                break;
            }
            let mut next_frontier: Vec<(String, f64)> = Vec::new();
            for (node_id, path_confidence) in &frontier {
                for relation in self.relations_of(node_id)? {
                    let (neighbor_id, edge_confidence) = if &relation.source_id == node_id {
                        (relation.target_id, relation.confidence)
                    } else {
                        (relation.source_id, relation.confidence)
                    };
                    if visited.contains(&neighbor_id) {
                        continue;
                    }
                    if visited.len() >= RELATED_VISITED_CAP {
                        break;
                    }
                    visited.insert(neighbor_id.clone());
                    let combined = path_confidence * edge_confidence;
                    // Confidence only shrinks along a path (each factor is in
                    // [0, 1]), so once a node fails the floor no descendant
                    // reached only through it could pass either — prune here.
                    if combined < min_confidence {
                        continue;
                    }
                    if let Some(memory) = self.get(&neighbor_id)? {
                        results.push(RelatedMemory {
                            memory,
                            hops: hop,
                            path_confidence: combined,
                        });
                    }
                    next_frontier.push((neighbor_id, combined));
                }
            }
            frontier = next_frontier;
        }

        results.sort_by(|a, b| {
            b.path_confidence
                .partial_cmp(&a.path_confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.hops.cmp(&b.hops))
        });
        Ok(results)
    }

    /// Plain (non-ranked) listing ordered per `opts.sort`, with optional category
    /// filter and offset/limit paging. Excludes `superseded` rows unless
    /// `opts.include_superseded` is set.
    pub fn list(&self, opts: &SearchOptions) -> AppResult<Vec<Memory>> {
        let order = match opts.sort {
            SortOrder::Relevance => "pinned DESC, importance DESC, last_accessed_at DESC",
            SortOrder::Recent => "updated_at DESC",
            SortOrder::Importance => "importance DESC, updated_at DESC",
            SortOrder::Oldest => "created_at ASC",
        };
        let mut sql = format!("SELECT {COLS} FROM memories");
        let mut conditions: Vec<&str> = Vec::new();
        let mut values: Vec<SqlValue> = Vec::new();
        if let Some(category) = &opts.category {
            conditions.push("category = ?");
            values.push(SqlValue::Text(category.clone()));
        }
        if !opts.include_superseded {
            conditions.push("superseded = 0");
        }
        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }
        sql.push_str(&format!(" ORDER BY {order} LIMIT ? OFFSET ?"));
        // Clamp the caller-supplied limit to a sane page ceiling so one tool/model
        // request can't read and allocate the whole table. (The saturating cast
        // alone would still honor an absurd-but-< i64::MAX limit.)
        let limit = opts.limit.min(MAX_LIST_LIMIT);
        values.push(SqlValue::Integer(i64::try_from(limit).unwrap_or(i64::MAX)));
        values.push(SqlValue::Integer(
            i64::try_from(opts.offset).unwrap_or(i64::MAX),
        ));
        let mut stmt = self.conn.prepare(&sql).map_err(to_service)?;
        let rows = stmt
            .query_map(params_from_iter(values), row_to_memory)
            .map_err(to_service)?;
        collect(rows)
    }

    /// Ranked retrieval: reciprocal-rank fusion (agentmemory RRF) across three
    /// streams — lexical (FTS5 BM25), vector (query-embedding cosine), and graph
    /// (1-hop relation expansion from the top fused-so-far candidates) — blended
    /// 0.7/0.3 with importance/recency/frequency salience, plus a pinned boost.
    /// With an empty/queryless call it degrades to a relevance-ordered recall of
    /// the most salient memories. Excludes `superseded` rows unless
    /// `opts.include_superseded` is set.
    pub fn search(&self, query: &str, opts: &SearchOptions) -> AppResult<Vec<ScoredMemory>> {
        let now = now_millis();
        // Clamp the caller-supplied limit BEFORE multiplying so no input can overflow
        // the usize (the trailing clamp alone runs too late to prevent the multiply).
        let candidate_limit = opts.limit.clamp(1, 40).saturating_mul(5).clamp(10, 200);
        let fts = fts_query(query);

        let mut candidates: Vec<Candidate> = if let Some(match_expr) = &fts {
            self.lexical_candidates(match_expr, opts, candidate_limit)?
        } else {
            // No usable query → seed with the most salient memories.
            let seed_opts = SearchOptions {
                limit: candidate_limit,
                offset: 0,
                ..opts.clone()
            };
            self.list(&seed_opts)?
                .into_iter()
                .map(Candidate::from_memory)
                .collect()
        };

        if opts.include_pinned {
            self.augment_with_pinned(&mut candidates, opts)?;
        }

        // Hybrid recall: a memory can match strongly by embedding while sharing no
        // FTS token with the query. Such rows never reach the lexical/salience seed
        // above, so when an embedding is supplied we merge in a bounded, category-
        // filtered pool of embedded rows. Cosine scoring below then lets a real
        // semantic match surface even with zero lexical overlap.
        if opts.query_embedding.is_some() {
            self.augment_with_embedded(&mut candidates, opts)?;
        }
        if let Some(query_vec) = &opts.query_embedding {
            // Resolve cosine similarity for any candidate that carries a stored
            // embedding but wasn't already scored by the pool scan above
            // (lexical/pinned hits).
            for candidate in &mut candidates {
                if candidate.embed_sim.is_none() && candidate.memory.has_embedding {
                    candidate.embed_sim = self
                        .embedding_of(&candidate.memory.id)?
                        .map(|stored| f64::from(cosine_similarity(query_vec, &stored)));
                }
            }
        }

        // Normalize the lexical sub-score reported on `ScoredMemory::lexical`,
        // independent of the RRF fusion that drives the final rank below; an
        // embedding similarity can still lift a weak/absent lexical hit.
        let raws: Vec<f64> = candidates.iter().filter_map(|c| c.raw_lex).collect();
        let normalized = min_max_normalize(&raws);
        let mut norm_iter = normalized.into_iter();
        for candidate in &mut candidates {
            let base = if candidate.raw_lex.is_some() {
                norm_iter.next().unwrap_or(0.0)
            } else {
                0.0
            };
            candidate.lexical = match candidate.embed_sim {
                Some(sim) => base.max((sim + 1.0) / 2.0),
                None => base,
            };
        }

        // RRF stream 1+2 (bm25, vector), fused first so the strongest
        // candidates-so-far can seed the graph stream's 1-hop expansion.
        let bm25_ranks = rank_by_score(
            candidates
                .iter()
                .filter_map(Candidate::bm25_entry)
                .collect(),
        );
        let vector_ranks = rank_by_score(
            candidates
                .iter()
                .filter_map(Candidate::vector_entry)
                .collect(),
        );
        let prelim_weights =
            RrfWeights::renormalized(!bm25_ranks.is_empty(), !vector_ranks.is_empty(), false);
        let mut prelim: Vec<(usize, f64)> = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| {
                let score = prelim_weights.fuse(
                    bm25_ranks.get(&candidate.memory.id).copied(),
                    vector_ranks.get(&candidate.memory.id).copied(),
                    None,
                );
                (index, score)
            })
            .collect();
        prelim.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let seed_ids: Vec<String> = prelim
            .into_iter()
            .take(GRAPH_SEED_COUNT)
            .map(|(index, _)| candidates[index].memory.id.clone())
            .collect();

        // RRF stream 3: 1-hop relation expansion from those seeds. May append
        // new candidates (relation targets not already in the pool).
        self.expand_graph_neighbors(&mut candidates, &seed_ids, opts)?;
        let graph_ranks = rank_by_score(
            candidates
                .iter()
                .filter_map(Candidate::graph_entry)
                .collect(),
        );

        let weights = RrfWeights::renormalized(
            !bm25_ranks.is_empty(),
            !vector_ranks.is_empty(),
            !graph_ranks.is_empty(),
        );
        let raw_fused: Vec<f64> = candidates
            .iter()
            .map(|candidate| {
                weights.fuse(
                    bm25_ranks.get(&candidate.memory.id).copied(),
                    vector_ranks.get(&candidate.memory.id).copied(),
                    graph_ranks.get(&candidate.memory.id).copied(),
                )
            })
            .collect();
        let rrf_normalized = min_max_normalize(&raw_fused);

        let mut scored: Vec<ScoredMemory> = candidates
            .into_iter()
            .zip(rrf_normalized)
            .map(|(candidate, rrf)| {
                let recency = recency_decay(
                    now - candidate.memory.last_accessed_at,
                    opts.recency_half_life_days,
                );
                let score = blend_rrf(
                    rrf,
                    candidate.memory.importance,
                    recency,
                    candidate.memory.access_count,
                    candidate.memory.pinned,
                );
                ScoredMemory {
                    lexical: candidate.lexical,
                    memory: candidate.memory,
                    score,
                }
            })
            .filter(|scored| opts.min_score.is_none_or(|min| scored.score >= min))
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(opts.limit);

        if opts.touch {
            let ids: Vec<String> = scored.iter().map(|entry| entry.memory.id.clone()).collect();
            self.touch(&ids, now)?;
        }
        Ok(scored)
    }

    /// Count memories, optionally within one category.
    pub fn count(&self, category: Option<&str>) -> AppResult<usize> {
        let count: i64 = if let Some(category) = category {
            self.conn.query_row(
                "SELECT COUNT(*) FROM memories WHERE category=?1",
                params![category],
                |row| row.get(0),
            )
        } else {
            self.conn
                .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        }
        .map_err(to_service)?;
        Ok(count.max(0) as usize)
    }

    /// Aggregate counts for the stats card.
    pub fn stats(&self) -> AppResult<MemoryStats> {
        let total = self.count(None)?;
        let pinned: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM memories WHERE pinned=1", [], |row| {
                row.get(0)
            })
            .map_err(to_service)?;
        let last_updated_at: Option<i64> = self
            .conn
            .query_row("SELECT MAX(updated_at) FROM memories", [], |row| row.get(0))
            .optional()
            .map_err(to_service)?
            .flatten();
        let mut stmt = self
            .conn
            .prepare("SELECT category, COUNT(*) FROM memories GROUP BY category ORDER BY COUNT(*) DESC, category ASC")
            .map_err(to_service)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(CategoryCount {
                    category: row.get(0)?,
                    count: (row.get::<_, i64>(1)?).max(0) as usize,
                })
            })
            .map_err(to_service)?;
        let by_category = collect(rows)?;
        Ok(MemoryStats {
            total,
            pinned: pinned.max(0) as usize,
            by_category,
            last_updated_at,
        })
    }

    /// Scan the most recent `limit` non-superseded memories for restated facts:
    /// pairs whose token-set Jaccard similarity exceeds [`CONTRADICTION_THRESHOLD`]
    /// are treated as the same fact said twice — the older row is marked
    /// `superseded` and linked to the newer one with a `contradicts` relation.
    /// Uses an inverted token index so only candidates sharing at least one
    /// token are ever compared (not an O(n²) full scan).
    pub fn sweep_contradictions(&self, limit: usize) -> AppResult<Vec<ContradictionHit>> {
        let limit = limit.max(1);
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {COLS} FROM memories WHERE superseded = 0 ORDER BY created_at DESC LIMIT {limit}"
            ))
            .map_err(to_service)?;
        let rows = stmt.query_map([], row_to_memory).map_err(to_service)?;
        let candidates = collect(rows)?;
        let token_sets: Vec<HashSet<String>> = candidates
            .iter()
            .map(|memory: &Memory| token_set(&memory.content))
            .collect();

        let mut inverted: HashMap<&str, Vec<usize>> = HashMap::new();
        for (index, tokens) in token_sets.iter().enumerate() {
            for token in tokens {
                inverted.entry(token.as_str()).or_default().push(index);
            }
        }

        let now = now_millis();
        let mut checked: HashSet<(usize, usize)> = HashSet::new();
        let mut already_superseded: HashSet<usize> = HashSet::new();
        let mut hits = Vec::new();
        for bucket in inverted.values() {
            for a in 0..bucket.len() {
                for b in (a + 1)..bucket.len() {
                    let pair = if bucket[a] < bucket[b] {
                        (bucket[a], bucket[b])
                    } else {
                        (bucket[b], bucket[a])
                    };
                    if !checked.insert(pair) {
                        continue;
                    }
                    let (i, j) = pair;
                    if already_superseded.contains(&i) || already_superseded.contains(&j) {
                        continue;
                    }
                    let similarity = jaccard_similarity(&token_sets[i], &token_sets[j]);
                    if similarity <= CONTRADICTION_THRESHOLD {
                        continue;
                    }
                    let (newer, older) = if candidates[i].created_at >= candidates[j].created_at {
                        (i, j)
                    } else {
                        (j, i)
                    };
                    self.conn
                        .execute(
                            "UPDATE memories SET superseded = 1 WHERE id = ?1",
                            params![candidates[older].id],
                        )
                        .map_err(to_service)?;
                    let confidence = infer_relation_confidence(
                        &candidates[newer],
                        &candidates[older],
                        RelationKind::Contradicts,
                        now,
                    );
                    self.upsert_relation(
                        &candidates[newer].id,
                        &candidates[older].id,
                        RelationKind::Contradicts,
                        confidence,
                        now,
                    )?;
                    already_superseded.insert(older);
                    hits.push(ContradictionHit {
                        kept_id: candidates[newer].id.clone(),
                        superseded_id: candidates[older].id.clone(),
                        similarity,
                    });
                }
            }
        }
        Ok(hits)
    }

    /// Aggregate retention-tier counts (agentmemory Ebbinghaus model) over every
    /// non-superseded memory, for a UI health card.
    pub fn retention_report(&self) -> AppResult<RetentionReport> {
        let now = now_millis();
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {COLS} FROM memories WHERE superseded = 0"))
            .map_err(to_service)?;
        let rows = stmt.query_map([], row_to_memory).map_err(to_service)?;
        let mut report = RetentionReport {
            hot: 0,
            warm: 0,
            cold: 0,
            evictable: 0,
        };
        for memory in collect(rows)? {
            match retention_tier(retention_score(&memory, now)) {
                RetentionTier::Hot => report.hot += 1,
                RetentionTier::Warm => report.warm += 1,
                RetentionTier::Cold => report.cold += 1,
                RetentionTier::Evictable => report.evictable += 1,
            }
        }
        Ok(report)
    }

    /// Auto-forget (agentmemory lifecycle), pinned always immune:
    ///  0. hard-delete unpinned rows whose TTL (`forget_after`) has passed;
    ///  1. sweep the most recent 1000 rows for restated contradictions
    ///     ([`Self::sweep_contradictions`] — soft-deletes via `superseded`, not
    ///     counted in the return value);
    ///  2. hard-delete unpinned rows that are stale (idle beyond `max_idle_days`)
    ///     AND either weak (importance below `min_importance`) or already
    ///     superseded;
    ///  3. if the store still exceeds `max_entries`, evict the weakest unpinned
    ///     overflow ranked by [`retention_score`] (weakest first) rather than
    ///     plain importance/recency.
    ///
    /// Returns how many rows were hard-deleted.
    pub fn prune(
        &self,
        max_entries: usize,
        max_idle_days: f64,
        min_importance: f64,
    ) -> AppResult<usize> {
        let now = now_millis();
        let mut removed = self
            .conn
            .execute(
                "DELETE FROM memories WHERE pinned = 0 AND forget_after IS NOT NULL AND forget_after < ?1",
                params![now],
            )
            .map_err(to_service)?;

        self.sweep_contradictions(1000)?;

        let stale_cutoff = now - ((max_idle_days.max(0.0) * MILLIS_PER_DAY) as i64);
        removed += self
            .conn
            .execute(
                "DELETE FROM memories WHERE pinned = 0 AND last_accessed_at < ?1
                   AND (importance < ?2 OR superseded = 1)",
                params![stale_cutoff, min_importance.clamp(0.0, 1.0)],
            )
            .map_err(to_service)?;

        let total = self.count(None)?;
        if total > max_entries {
            let overflow = total - max_entries;
            let mut stmt = self
                .conn
                .prepare(&format!("SELECT {COLS} FROM memories WHERE pinned = 0"))
                .map_err(to_service)?;
            let rows = stmt.query_map([], row_to_memory).map_err(to_service)?;
            let mut unpinned = collect(rows)?;
            unpinned.sort_by(|a, b| {
                retention_score(a, now)
                    .partial_cmp(&retention_score(b, now))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for memory in unpinned.into_iter().take(overflow) {
                removed += self
                    .conn
                    .execute("DELETE FROM memories WHERE id = ?1", params![memory.id])
                    .map_err(to_service)?;
            }
        }

        // The bulk deletes above bypass `delete()`'s relation cleanup; sweep any
        // edges left dangling from a row removed by TTL/stale/overflow eviction.
        self.conn
            .execute(
                "DELETE FROM memory_relations WHERE source_id NOT IN (SELECT id FROM memories)
                                                  OR target_id NOT IN (SELECT id FROM memories)",
                [],
            )
            .map_err(to_service)?;

        Ok(removed)
    }

    /// Delete every memory in a category; returns how many were removed.
    pub fn wipe_category(&self, category: &str) -> AppResult<usize> {
        let removed = self
            .conn
            .execute("DELETE FROM memories WHERE category=?1", params![category])
            .map_err(to_service)?;
        Ok(removed)
    }

    /// Delete every memory; returns how many were removed.
    pub fn wipe_all(&self) -> AppResult<usize> {
        let removed = self
            .conn
            .execute("DELETE FROM memories", [])
            .map_err(to_service)?;
        Ok(removed)
    }

    // ── internals ──

    fn require(&self, id: &str) -> AppResult<Memory> {
        self.get(id)?
            .ok_or_else(|| AppError::Service(format!("memory {id} missing after write")))
    }

    fn lexical_candidates(
        &self,
        match_expr: &str,
        opts: &SearchOptions,
        candidate_limit: usize,
    ) -> AppResult<Vec<Candidate>> {
        let mut sql = format!(
            "SELECT {COLS_Q}, bm25(memories_fts) AS rank
             FROM memories_fts JOIN memories ON memories.rowid = memories_fts.rowid
             WHERE memories_fts MATCH ?1"
        );
        let mut values: Vec<SqlValue> = vec![SqlValue::Text(match_expr.to_string())];
        if let Some(category) = &opts.category {
            sql.push_str(" AND memories.category = ?2");
            values.push(SqlValue::Text(category.clone()));
        }
        if !opts.include_superseded {
            sql.push_str(" AND memories.superseded = 0");
        }
        sql.push_str(&format!(" ORDER BY rank LIMIT {candidate_limit}"));
        let mut stmt = self.conn.prepare(&sql).map_err(to_service)?;
        let rows = stmt
            .query_map(params_from_iter(values), |row| {
                let memory = row_to_memory(row)?;
                // bm25 returns lower = better; negate so larger = stronger match.
                let rank: f64 = row.get(14)?;
                Ok(Candidate {
                    memory,
                    raw_lex: Some(-rank),
                    lexical: 0.0,
                    embed_sim: None,
                    graph_score: None,
                })
            })
            .map_err(to_service)?;
        collect(rows)
    }

    fn augment_with_pinned(
        &self,
        candidates: &mut Vec<Candidate>,
        opts: &SearchOptions,
    ) -> AppResult<()> {
        let present: HashSet<String> = candidates.iter().map(|c| c.memory.id.clone()).collect();
        let mut sql = format!("SELECT {COLS} FROM memories WHERE pinned=1");
        let mut values: Vec<SqlValue> = Vec::new();
        if let Some(category) = &opts.category {
            sql.push_str(" AND category = ?");
            values.push(SqlValue::Text(category.clone()));
        }
        if !opts.include_superseded {
            sql.push_str(" AND superseded = 0");
        }
        sql.push_str(" ORDER BY importance DESC, updated_at DESC LIMIT 32");
        let mut stmt = self.conn.prepare(&sql).map_err(to_service)?;
        let rows = stmt
            .query_map(params_from_iter(values), row_to_memory)
            .map_err(to_service)?;
        for memory in collect(rows)? {
            if !present.contains(&memory.id) {
                candidates.push(Candidate::from_memory(memory));
            }
        }
        Ok(())
    }

    /// Merge a bounded pool of embedded rows into `candidates`, scoring each by
    /// cosine to the query embedding. This is what makes semantic-only recall work
    /// (a strong embedding match with no shared FTS token). Bounded by
    /// `EMBEDDED_SCAN_CAP` and ordered by salience so the most relevant embedded
    /// rows are considered first on large stores.
    fn augment_with_embedded(
        &self,
        candidates: &mut Vec<Candidate>,
        opts: &SearchOptions,
    ) -> AppResult<()> {
        let Some(query_vec) = opts.query_embedding.as_deref() else {
            return Ok(());
        };
        if query_vec.is_empty() {
            return Ok(());
        }
        let present: HashSet<String> = candidates.iter().map(|c| c.memory.id.clone()).collect();

        let mut sql = format!("SELECT {COLS}, embedding FROM memories WHERE embedding IS NOT NULL");
        let mut values: Vec<SqlValue> = Vec::new();
        if let Some(category) = &opts.category {
            sql.push_str(" AND category = ?");
            values.push(SqlValue::Text(category.clone()));
        }
        if !opts.include_superseded {
            sql.push_str(" AND superseded = 0");
        }
        sql.push_str(&format!(
            " ORDER BY pinned DESC, importance DESC, last_accessed_at DESC LIMIT {EMBEDDED_SCAN_CAP}"
        ));
        let mut stmt = self.conn.prepare(&sql).map_err(to_service)?;
        let rows = stmt
            .query_map(params_from_iter(values), |row| {
                let memory = row_to_memory(row)?;
                let blob: Vec<u8> = row.get(14)?;
                Ok((memory, blob))
            })
            .map_err(to_service)?;
        for row in rows {
            let (memory, blob) = row.map_err(to_service)?;
            if present.contains(&memory.id) {
                continue;
            }
            let stored = decode_embedding(&blob);
            let sim = f64::from(cosine_similarity(query_vec, &stored));
            candidates.push(Candidate {
                memory,
                raw_lex: None,
                lexical: 0.0,
                embed_sim: Some(sim),
                graph_score: None,
            });
        }
        Ok(())
    }

    /// RRF graph stream: 1-hop relation expansion from `seed_ids` (the top
    /// fused-so-far candidates). Neighbors not already in `candidates` are
    /// fetched and appended (respecting the category filter and
    /// `include_superseded`); every reached candidate — new or already
    /// present — gets its `graph_score` set to the strongest edge confidence
    /// that reached it.
    fn expand_graph_neighbors(
        &self,
        candidates: &mut Vec<Candidate>,
        seed_ids: &[String],
        opts: &SearchOptions,
    ) -> AppResult<()> {
        if seed_ids.is_empty() {
            return Ok(());
        }
        let present: HashSet<String> = candidates.iter().map(|c| c.memory.id.clone()).collect();
        let mut graph_scores: HashMap<String, f64> = HashMap::new();
        for seed_id in seed_ids {
            for relation in self.relations_of(seed_id)? {
                let neighbor_id = if &relation.source_id == seed_id {
                    relation.target_id
                } else {
                    relation.source_id
                };
                if &neighbor_id == seed_id {
                    continue;
                }
                let entry = graph_scores.entry(neighbor_id).or_insert(0.0);
                if relation.confidence > *entry {
                    *entry = relation.confidence;
                }
            }
        }

        for (neighbor_id, confidence) in &graph_scores {
            if present.contains(neighbor_id) {
                continue;
            }
            let Some(memory) = self.get(neighbor_id)? else {
                continue;
            };
            if !opts.include_superseded && memory.superseded {
                continue;
            }
            if opts
                .category
                .as_deref()
                .is_some_and(|category| category != memory.category)
            {
                continue;
            }
            candidates.push(Candidate {
                memory,
                raw_lex: None,
                lexical: 0.0,
                embed_sim: None,
                graph_score: Some(*confidence),
            });
        }
        for candidate in candidates.iter_mut() {
            if let Some(score) = graph_scores.get(&candidate.memory.id) {
                candidate.graph_score = Some(*score);
            }
        }
        Ok(())
    }

    fn embedding_of(&self, id: &str) -> AppResult<Option<Vec<f32>>> {
        let blob: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT embedding FROM memories WHERE id=?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(to_service)?
            .flatten();
        Ok(blob.map(|bytes| decode_embedding(&bytes)))
    }

    fn touch(&self, ids: &[String], now: i64) -> AppResult<()> {
        for id in ids {
            self.conn
                .execute(
                    "UPDATE memories SET last_accessed_at=?2, access_count=access_count+1 WHERE id=?1",
                    params![id, now],
                )
                .map_err(to_service)?;
        }
        Ok(())
    }
}

/// Idempotent v0 → v1 migration: backfill `superseded`/`forget_after` on the
/// `memories` table (which `SCHEMA_TABLES` has just created-if-absent, so on a
/// fresh database it already has both columns and the `ALTER TABLE`s below hit
/// "duplicate column" and are tolerated as a no-op). Runs before `SCHEMA_REST`
/// creates the `idx_memories_superseded` index and `memory_relations` table, so
/// those never see a v0 table mid-migration. Re-running is safe if
/// `user_version` was left stale by an interrupted previous run.
fn migrate(conn: &Connection) -> AppResult<()> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(to_service)?;
    if version >= SCHEMA_VERSION {
        return Ok(());
    }
    for stmt in [
        "ALTER TABLE memories ADD COLUMN superseded INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE memories ADD COLUMN forget_after INTEGER",
    ] {
        if let Err(error) = conn.execute(stmt, []) {
            if !is_duplicate_column(&error) {
                return Err(to_service(error));
            }
        }
    }
    conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))
        .map_err(to_service)?;
    Ok(())
}

fn is_duplicate_column(error: &rusqlite::Error) -> bool {
    error.to_string().contains("duplicate column name")
}

fn row_to_memory(row: &Row) -> rusqlite::Result<Memory> {
    let metadata_text: String = row.get(3)?;
    let metadata = serde_json::from_str(&metadata_text).unwrap_or_else(|_| serde_json::json!({}));
    Ok(Memory {
        id: row.get(0)?,
        category: row.get(1)?,
        content: row.get(2)?,
        metadata,
        importance: row.get(4)?,
        pinned: row.get::<_, i64>(5)? != 0,
        source: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        last_accessed_at: row.get(9)?,
        access_count: row.get(10)?,
        superseded: row.get::<_, i64>(11)? != 0,
        forget_after: row.get(12)?,
        has_embedding: row.get::<_, i64>(13)? != 0,
    })
}

fn row_to_relation(row: &Row) -> rusqlite::Result<MemoryRelation> {
    let relation_text: String = row.get(3)?;
    let relation = RelationKind::from_str(&relation_text).unwrap_or(RelationKind::Related);
    Ok(MemoryRelation {
        id: row.get(0)?,
        source_id: row.get(1)?,
        target_id: row.get(2)?,
        relation,
        confidence: row.get(4)?,
        created_at: row.get(5)?,
    })
}

fn collect<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&Row) -> rusqlite::Result<T>>,
) -> AppResult<Vec<T>> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(to_service)?);
    }
    Ok(out)
}

fn normalize_metadata(value: Option<serde_json::Value>) -> serde_json::Value {
    match value {
        Some(value) if value.is_object() => value,
        _ => serde_json::json!({}),
    }
}

fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn to_service<E: std::fmt::Display>(error: E) -> AppError {
    AppError::Service(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> MemoryStore {
        MemoryStore::open_in_memory().expect("open in-memory store")
    }

    fn new_memory(category: &str, content: &str) -> NewMemory {
        NewMemory {
            category: category.to_string(),
            content: content.to_string(),
            ..NewMemory::default()
        }
    }

    #[test]
    fn create_get_update_delete_roundtrip() {
        let store = store();
        let created = store
            .create(new_memory("core", "uses pnpm workspaces"))
            .unwrap();
        assert_eq!(created.category, "core");
        assert!((created.importance - DEFAULT_IMPORTANCE).abs() < 1e-9);

        let fetched = store.get(&created.id).unwrap().unwrap();
        assert_eq!(fetched.content, "uses pnpm workspaces");

        let updated = store
            .update(
                &created.id,
                MemoryPatch {
                    importance: Some(0.9),
                    pinned: Some(true),
                    ..MemoryPatch::default()
                },
            )
            .unwrap();
        assert!((updated.importance - 0.9).abs() < 1e-9);
        assert!(updated.pinned);

        assert!(store.delete(&created.id).unwrap());
        assert!(store.get(&created.id).unwrap().is_none());
    }

    #[test]
    fn search_matches_lexically_and_filters_category() {
        let store = store();
        store
            .create(new_memory("semantic", "the build uses vite and rolldown"))
            .unwrap();
        store
            .create(new_memory("semantic", "tests run under vitest"))
            .unwrap();
        store
            .create(new_memory("episodic", "fixed the vite chunking bug"))
            .unwrap();

        let hits = store
            .search(
                "vite",
                &SearchOptions {
                    touch: false,
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        assert!(hits.iter().any(|h| h.memory.content.contains("vite")));
        assert!(hits.iter().all(|h| h.lexical >= 0.0));

        let only_semantic = store
            .search(
                "vite",
                &SearchOptions {
                    category: Some("semantic".into()),
                    touch: false,
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        assert!(only_semantic
            .iter()
            .all(|h| h.memory.category == "semantic"));
    }

    #[test]
    fn pinned_memories_surface_without_a_lexical_hit() {
        let store = store();
        let pinned = store
            .create(NewMemory {
                pinned: Some(true),
                ..new_memory("core", "always prefer rust for core logic")
            })
            .unwrap();
        store
            .create(new_memory("episodic", "unrelated note about css"))
            .unwrap();

        // Query that does NOT match the pinned memory still returns it (pinned augmentation).
        let hits = store
            .search(
                "zzzznomatch",
                &SearchOptions {
                    touch: false,
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        assert!(hits.iter().any(|h| h.memory.id == pinned.id));
    }

    #[test]
    fn touch_bumps_access_count() {
        let store = store();
        let created = store
            .create(new_memory("core", "touch me on recall"))
            .unwrap();
        store.search("touch", &SearchOptions::default()).unwrap();
        let after = store.get(&created.id).unwrap().unwrap();
        assert!(after.access_count >= 1);
    }

    #[test]
    fn duplicate_content_reinforces_instead_of_cloning() {
        let store = store();
        let first = store
            .create(NewMemory {
                importance: Some(0.4),
                ..new_memory("core", "user prefers tabs over spaces")
            })
            .unwrap();
        let second = store
            .create(NewMemory {
                importance: Some(0.7),
                pinned: Some(true),
                ..new_memory("core", "user prefers tabs over spaces")
            })
            .unwrap();
        // Same row, strengthened: no duplicate, max importance, pin sticks.
        assert_eq!(first.id, second.id);
        assert_eq!(store.count(None).unwrap(), 1);
        assert!((second.importance - 0.7).abs() < 1e-9);
        assert!(second.pinned);
        assert!(second.access_count >= 1);
        // Different category with same content is NOT a duplicate.
        store
            .create(new_memory("episodic", "user prefers tabs over spaces"))
            .unwrap();
        assert_eq!(store.count(None).unwrap(), 2);
    }

    #[test]
    fn prune_evicts_stale_weak_and_overflow_but_never_pinned() {
        let store = store();
        // Stale + weak: last_accessed_at forced into the past via direct update.
        let stale = store
            .create(NewMemory {
                importance: Some(0.1),
                ..new_memory("core", "stale weak memory")
            })
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE memories SET last_accessed_at = 0 WHERE id = ?1",
                params![stale.id],
            )
            .unwrap();
        // Pinned but ancient — must survive any prune.
        let pinned = store
            .create(NewMemory {
                pinned: Some(true),
                importance: Some(0.05),
                ..new_memory("core", "pinned forever")
            })
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE memories SET last_accessed_at = 0 WHERE id = ?1",
                params![pinned.id],
            )
            .unwrap();
        let fresh = store
            .create(NewMemory {
                importance: Some(0.9),
                ..new_memory("core", "fresh important memory")
            })
            .unwrap();

        let removed = store.prune(10, 30.0, 0.3).unwrap();
        assert_eq!(removed, 1);
        assert!(store.get(&stale.id).unwrap().is_none());
        assert!(store.get(&pinned.id).unwrap().is_some());
        assert!(store.get(&fresh.id).unwrap().is_some());

        // Overflow eviction: cap 1 → weakest unpinned goes, pinned stays.
        let removed = store.prune(1, 3650.0, 0.0).unwrap();
        assert_eq!(removed, 1);
        assert!(store.get(&pinned.id).unwrap().is_some());
        assert!(store.get(&fresh.id).unwrap().is_none());
    }

    #[test]
    fn prune_overflow_evicts_by_retention_score_not_raw_importance() {
        let store = store();
        // Slightly higher raw importance, but stale and never recalled.
        let weak_but_stale = store
            .create(NewMemory {
                importance: Some(0.35),
                ..new_memory("episodic", "stale note, higher raw importance")
            })
            .unwrap();
        // Slightly lower raw importance, but fresh and heavily recalled.
        let strong_but_lower_importance = store
            .create(NewMemory {
                importance: Some(0.30),
                ..new_memory("episodic", "hot note, lower raw importance")
            })
            .unwrap();
        let day = 86_400_000;
        let now = now_millis();
        store
            .conn
            .execute(
                "UPDATE memories SET created_at=?2, updated_at=?2, last_accessed_at=?2, access_count=0 WHERE id=?1",
                params![weak_but_stale.id, now - 200 * day],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE memories SET created_at=?2, updated_at=?2, last_accessed_at=?2, access_count=200 WHERE id=?1",
                params![strong_but_lower_importance.id, now],
            )
            .unwrap();

        // Plain importance ordering would evict `strong_but_lower_importance`
        // (0.30 < 0.35) — but retention-ranked eviction, boosted by its heavy
        // recent recall, must evict the stale one instead.
        let removed = store.prune(1, 3650.0, 0.0).unwrap();
        assert_eq!(removed, 1);
        assert!(store.get(&weak_but_stale.id).unwrap().is_none());
        assert!(store
            .get(&strong_but_lower_importance.id)
            .unwrap()
            .is_some());
    }

    #[test]
    fn prune_hard_deletes_expired_ttl_rows_but_pinned_survive() {
        let store = store();
        let expiring = store
            .create(NewMemory {
                ttl_days: Some(1.0),
                ..new_memory("episodic", "temporary note")
            })
            .unwrap();
        let pinned_expiring = store
            .create(NewMemory {
                ttl_days: Some(1.0),
                pinned: Some(true),
                ..new_memory("episodic", "pinned temporary note")
            })
            .unwrap();
        let fresh = store.create(new_memory("episodic", "no ttl here")).unwrap();

        // Force both TTLs into the past.
        store
            .conn
            .execute(
                "UPDATE memories SET forget_after = 1 WHERE id IN (?1, ?2)",
                params![expiring.id, pinned_expiring.id],
            )
            .unwrap();

        let removed = store.prune(100, 3650.0, 0.0).unwrap();
        assert_eq!(removed, 1);
        assert!(store.get(&expiring.id).unwrap().is_none());
        assert!(
            store.get(&pinned_expiring.id).unwrap().is_some(),
            "pinned survives even an expired TTL"
        );
        assert!(store.get(&fresh.id).unwrap().is_some());
    }

    #[test]
    fn stats_and_wipe() {
        let store = store();
        store.create(new_memory("core", "a")).unwrap();
        store.create(new_memory("core", "b")).unwrap();
        store.create(new_memory("episodic", "c")).unwrap();

        let stats = store.stats().unwrap();
        assert_eq!(stats.total, 3);
        assert_eq!(
            stats
                .by_category
                .iter()
                .find(|c| c.category == "core")
                .unwrap()
                .count,
            2
        );

        assert_eq!(store.wipe_category("core").unwrap(), 2);
        assert_eq!(store.count(None).unwrap(), 1);
        assert_eq!(store.wipe_all().unwrap(), 1);
        assert_eq!(store.count(None).unwrap(), 0);
    }

    #[test]
    fn upsert_by_id_replaces_content() {
        let store = store();
        let first = store
            .create(NewMemory {
                id: Some("fixed".into()),
                ..new_memory("core", "v1")
            })
            .unwrap();
        let second = store
            .create(NewMemory {
                id: Some("fixed".into()),
                ..new_memory("core", "v2")
            })
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(store.count(None).unwrap(), 1);
        assert_eq!(store.get("fixed").unwrap().unwrap().content, "v2");
    }

    #[test]
    fn importance_breaks_lexical_ties() {
        let store = store();
        // Identical content => identical lexical match; importance must decide order.
        store
            .create(NewMemory {
                importance: Some(0.1),
                ..new_memory("core", "vite build pipeline")
            })
            .unwrap();
        let important = store
            .create(NewMemory {
                importance: Some(0.95),
                ..new_memory("core", "vite build pipeline")
            })
            .unwrap();
        let hits = store
            .search(
                "vite build pipeline",
                &SearchOptions {
                    touch: false,
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        assert_eq!(hits.first().unwrap().memory.id, important.id);
    }

    #[test]
    fn min_score_floor_drops_everything() {
        let store = store();
        store.create(new_memory("core", "alpha beta")).unwrap();
        // Max unpinned score is bounded at 1.0 (the RRF and salience weights sum
        // to 1); a 2.0 floor clears all.
        let hits = store
            .search(
                "alpha",
                &SearchOptions {
                    min_score: Some(2.0),
                    touch: false,
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn embedding_recalls_semantic_only_memory_without_shared_tokens() {
        // A memory whose TEXT shares no token with the query, but whose embedding
        // is close, must still be recalled via the embedded-pool scan (the
        // vector stream of RRF fusion).
        let store = store();
        let target = store
            .create(NewMemory {
                embedding: Some(vec![1.0, 0.0, 0.0]),
                ..new_memory("semantic", "the deployment pipeline runs nightly")
            })
            .unwrap();
        store
            .create(NewMemory {
                embedding: Some(vec![0.0, 1.0, 0.0]),
                ..new_memory("semantic", "unrelated note about colors")
            })
            .unwrap();

        let hits = store
            .search(
                // No lexical overlap with the target content at all.
                "zzz qqq",
                &SearchOptions {
                    query_embedding: Some(vec![0.99, 0.01, 0.0]),
                    touch: false,
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            hits.first().map(|h| h.memory.id.as_str()),
            Some(target.id.as_str()),
            "semantic-only memory must surface first via embedding recall"
        );
    }

    #[test]
    fn graph_stream_surfaces_a_related_row_with_no_lexical_or_vector_overlap() {
        let store = store();
        let anchor = store
            .create(new_memory(
                "semantic",
                "the ci pipeline runs on self-hosted runners",
            ))
            .unwrap();
        let related = store
            .create(new_memory(
                "semantic",
                "totally unrelated wording about nothing shared",
            ))
            .unwrap();
        store
            .relate(&anchor.id, &related.id, RelationKind::Related, Some(0.9))
            .unwrap();

        let hits = store
            .search(
                "ci pipeline self-hosted runners",
                &SearchOptions {
                    touch: false,
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        assert!(hits.iter().any(|h| h.memory.id == anchor.id));
        assert!(
            hits.iter().any(|h| h.memory.id == related.id),
            "a 1-hop relation from the top fused candidate should surface via the graph RRF stream"
        );
    }

    #[test]
    fn list_clamps_absurd_limit_to_cap() {
        let store = store();
        // Distinct enough content that near-duplicate supersession (Jaccard
        // similarity on tokens > 2 chars) doesn't collapse these into one row.
        for topic in ["alpha widgets", "beta gadgets", "gamma sprockets"] {
            store.create(new_memory("core", topic)).unwrap();
        }
        // A huge caller-supplied limit must be clamped, not honored verbatim.
        let listed = store
            .list(&SearchOptions {
                limit: usize::MAX,
                ..SearchOptions::default()
            })
            .unwrap();
        assert!(listed.len() <= super::MAX_LIST_LIMIT);
        assert_eq!(listed.len(), 3);
    }

    #[test]
    fn persists_to_disk_across_reopen() {
        // The exact path the desktop uses: a real .db file, reopened on the next
        // workspace open / app restart. Proves persistence + schema survive reopen.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("memory.db");
        {
            let store = MemoryStore::open(&path).expect("open store");
            store
                .create(new_memory("core", "the build uses pnpm and rolldown"))
                .unwrap();
        }
        let store = MemoryStore::open(&path).expect("reopen store");
        assert_eq!(store.count(None).unwrap(), 1);
        let hits = store
            .search(
                "rolldown build",
                &SearchOptions {
                    touch: false,
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        assert!(hits
            .iter()
            .any(|hit| hit.memory.content.contains("rolldown")));
    }

    #[test]
    fn migration_from_v0_backfills_columns_and_relations_table() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("v0.db");
        {
            // Build a raw pre-migration (v0) schema: no `superseded`/`forget_after`
            // columns, no `memory_relations` table — exactly what a database
            // created before this port would look like on disk.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE memories (
                  rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                  id TEXT NOT NULL UNIQUE,
                  category TEXT NOT NULL,
                  content TEXT NOT NULL,
                  metadata TEXT NOT NULL DEFAULT '{}',
                  importance REAL NOT NULL DEFAULT 0.5,
                  pinned INTEGER NOT NULL DEFAULT 0,
                  source TEXT,
                  embedding BLOB,
                  created_at INTEGER NOT NULL,
                  updated_at INTEGER NOT NULL,
                  last_accessed_at INTEGER NOT NULL,
                  access_count INTEGER NOT NULL DEFAULT 0
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO memories (id, category, content, created_at, updated_at, last_accessed_at)
                 VALUES ('legacy', 'core', 'pre-migration row', 1, 1, 1)",
                [],
            )
            .unwrap();
        }

        let store = MemoryStore::open(&path).expect("open + migrate v0 db");
        let migrated = store
            .get("legacy")
            .unwrap()
            .expect("row survives migration");
        assert_eq!(migrated.content, "pre-migration row");
        assert!(!migrated.superseded);
        assert_eq!(migrated.forget_after, None);

        // memory_relations must now exist and be usable.
        let other = store.create(new_memory("core", "a related row")).unwrap();
        let relation = store
            .relate(&other.id, "legacy", RelationKind::Related, Some(0.8))
            .unwrap();
        assert!((relation.confidence - 0.8).abs() < 1e-9);

        // Reopening an already-migrated db is a no-op, not an error.
        drop(store);
        let reopened = MemoryStore::open(&path).expect("reopen migrated db");
        assert_eq!(
            reopened.get("legacy").unwrap().unwrap().content,
            "pre-migration row"
        );
    }

    #[test]
    fn relate_unrelate_and_relations_of_roundtrip() {
        let store = store();
        // Distinct content: "fact a" / "fact b" would collapse to the same
        // >2-char token set ({"fact"}) and trigger near-duplicate supersession
        // on create, adding an unwanted extra relation edge to `a`.
        let a = store.create(new_memory("core", "the sky is blue")).unwrap();
        let b = store
            .create(new_memory("core", "grass tends to be green"))
            .unwrap();

        let relation = store
            .relate(&a.id, &b.id, RelationKind::Extends, None)
            .unwrap();
        // Default confidence: both rows freshly created (updated within 7 days) → 0.5 + 0.1.
        assert!((relation.confidence - 0.6).abs() < 1e-9);

        let relations = store.relations_of(&a.id).unwrap();
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].id, relation.id);

        // Same triple again refreshes confidence instead of duplicating the edge.
        let refreshed = store
            .relate(&a.id, &b.id, RelationKind::Extends, Some(0.2))
            .unwrap();
        assert_eq!(refreshed.id, relation.id);
        assert!((refreshed.confidence - 0.2).abs() < 1e-9);
        assert_eq!(store.relations_of(&a.id).unwrap().len(), 1);

        assert!(store.unrelate(&relation.id).unwrap());
        assert!(store.relations_of(&a.id).unwrap().is_empty());

        // Self-relation is rejected.
        assert!(store
            .relate(&a.id, &a.id, RelationKind::Related, None)
            .is_err());
        // Unknown id is rejected.
        assert!(store
            .relate(&a.id, "missing", RelationKind::Related, None)
            .is_err());
    }

    #[test]
    fn related_bfs_respects_hops_and_min_confidence() {
        let store = store();
        // Distinct content (see the note in `relate_unrelate_and_relations_of_roundtrip`
        // about "node a"/"node b"/"node c" colliding under near-dup supersession).
        let a = store
            .create(new_memory("core", "alpha node about widgets"))
            .unwrap();
        let b = store
            .create(new_memory("core", "beta node about gadgets"))
            .unwrap();
        let c = store
            .create(new_memory("core", "gamma node about sprockets"))
            .unwrap();
        store
            .relate(&a.id, &b.id, RelationKind::Related, Some(0.9))
            .unwrap();
        store
            .relate(&b.id, &c.id, RelationKind::Related, Some(0.9))
            .unwrap();

        // 1 hop only reaches b.
        let one_hop = store.related(&a.id, 1, 0.0).unwrap();
        assert_eq!(one_hop.len(), 1);
        assert_eq!(one_hop[0].memory.id, b.id);
        assert_eq!(one_hop[0].hops, 1);

        // 2 hops reaches both b and c; c's path confidence is the product 0.9*0.9.
        let two_hop = store.related(&a.id, 2, 0.0).unwrap();
        assert_eq!(two_hop.len(), 2);
        let c_hit = two_hop.iter().find(|r| r.memory.id == c.id).unwrap();
        assert_eq!(c_hit.hops, 2);
        assert!((c_hit.path_confidence - 0.81).abs() < 1e-9);
        assert!(two_hop.iter().all(|r| r.memory.id != a.id));

        // A high min_confidence floor prunes the second hop.
        let filtered = store.related(&a.id, 5, 0.85).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].memory.id, b.id);
    }

    #[test]
    fn supersession_on_create_marks_near_duplicates_within_category() {
        let store = store();
        let old = store
            .create(new_memory(
                "semantic",
                "the project uses pnpm workspaces for the monorepo",
            ))
            .unwrap();
        let outcome = store
            .create_with_outcome(new_memory(
                "semantic",
                "the project uses pnpm workspaces for the monorepo today",
            ))
            .unwrap();
        assert_eq!(outcome.superseded_ids, vec![old.id.clone()]);
        assert!(store.get(&old.id).unwrap().unwrap().superseded);

        // A supersedes relation was recorded, new → old.
        let relations = store.relations_of(&old.id).unwrap();
        assert!(relations
            .iter()
            .any(|r| r.relation == RelationKind::Supersedes && r.source_id == outcome.memory.id));

        // Superseded rows are excluded from search by default...
        let hits = store
            .search(
                "pnpm workspaces monorepo",
                &SearchOptions {
                    touch: false,
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        assert!(hits.iter().all(|h| h.memory.id != old.id));
        // ...but included when explicitly requested.
        let with_superseded = store
            .search(
                "pnpm workspaces monorepo",
                &SearchOptions {
                    touch: false,
                    include_superseded: true,
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        assert!(with_superseded.iter().any(|h| h.memory.id == old.id));

        // A near-duplicate in a DIFFERENT category is not touched.
        let other_category = store
            .create(new_memory(
                "episodic",
                "the project uses pnpm workspaces for the monorepo",
            ))
            .unwrap();
        assert!(!store.get(&other_category.id).unwrap().unwrap().superseded);
    }

    #[test]
    fn reinforcing_exact_content_clears_a_prior_supersession() {
        let store = store();
        let old = store
            .create(new_memory(
                "semantic",
                "the desktop bundle targets webview2 on windows",
            ))
            .unwrap();
        // Near-duplicate marks the original superseded.
        store
            .create(new_memory(
                "semantic",
                "the desktop bundle targets webview2 on windows now",
            ))
            .unwrap();
        assert!(store.get(&old.id).unwrap().unwrap().superseded);

        // Re-remembering the exact original content reinforces AND revives it.
        let outcome = store
            .create_with_outcome(new_memory(
                "semantic",
                "the desktop bundle targets webview2 on windows",
            ))
            .unwrap();
        assert_eq!(outcome.memory.id, old.id);
        assert!(!outcome.memory.superseded);
        let hits = store
            .search(
                "desktop bundle webview2 windows",
                &SearchOptions {
                    touch: false,
                    ..SearchOptions::default()
                },
            )
            .unwrap();
        assert!(hits.iter().any(|h| h.memory.id == old.id));
    }

    #[test]
    fn sweep_contradictions_marks_older_row_and_links_relation() {
        let store = store();
        // Explicit ids bypass both the exact-dedup reinforce path and the
        // fresh-insert near-dup supersession, so the sweep is what catches them.
        let older = store
            .create(NewMemory {
                id: Some("older".into()),
                ..new_memory("core", "the deploy window is tuesdays at noon")
            })
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE memories SET created_at = 1000 WHERE id = ?1",
                params![older.id],
            )
            .unwrap();
        let newer = store
            .create(NewMemory {
                id: Some("newer".into()),
                ..new_memory("core", "the deploy window is tuesdays at noon")
            })
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE memories SET created_at = 2000 WHERE id = ?1",
                params![newer.id],
            )
            .unwrap();

        let hits = store.sweep_contradictions(100).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kept_id, newer.id);
        assert_eq!(hits[0].superseded_id, older.id);
        assert!((hits[0].similarity - 1.0).abs() < 1e-9);

        assert!(store.get(&older.id).unwrap().unwrap().superseded);
        assert!(!store.get(&newer.id).unwrap().unwrap().superseded);

        let relations = store.relations_of(&older.id).unwrap();
        assert!(relations
            .iter()
            .any(|r| r.relation == RelationKind::Contradicts && r.source_id == newer.id));
    }

    #[test]
    fn retention_report_buckets_match_individual_retention_tiers() {
        let store = store();
        let now = now_millis();
        let day = 86_400_000;
        let rows: [(&str, &str, f64, i64, i64, i64); 4] = [
            ("fresh", "core", 0.9, now, now, 50),
            ("aging", "episodic", 0.5, now - 60 * day, now - 60 * day, 2),
            (
                "stale",
                "episodic",
                0.2,
                now - 300 * day,
                now - 300 * day,
                0,
            ),
            (
                "ancient",
                "episodic",
                0.1,
                now - 1000 * day,
                now - 1000 * day,
                0,
            ),
        ];
        for (id, category, importance, created_at, last_accessed_at, access_count) in rows {
            store
                .create(NewMemory {
                    id: Some(id.to_string()),
                    importance: Some(importance),
                    ..new_memory(category, &format!("memory {id}"))
                })
                .unwrap();
            store
                .conn
                .execute(
                    "UPDATE memories SET created_at=?2, updated_at=?2, last_accessed_at=?3, access_count=?4 WHERE id=?1",
                    params![id, created_at, last_accessed_at, access_count],
                )
                .unwrap();
        }

        let mut expected = RetentionReport {
            hot: 0,
            warm: 0,
            cold: 0,
            evictable: 0,
        };
        for (id, ..) in rows {
            let memory = store.get(id).unwrap().unwrap();
            match retention_tier(retention_score(&memory, now)) {
                RetentionTier::Hot => expected.hot += 1,
                RetentionTier::Warm => expected.warm += 1,
                RetentionTier::Cold => expected.cold += 1,
                RetentionTier::Evictable => expected.evictable += 1,
            }
        }

        let report = store.retention_report().unwrap();
        assert_eq!(report.hot, expected.hot);
        assert_eq!(report.warm, expected.warm);
        assert_eq!(report.cold, expected.cold);
        assert_eq!(report.evictable, expected.evictable);
        assert_eq!(report.hot + report.warm + report.cold + report.evictable, 4);
    }
}
