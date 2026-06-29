//! SQLite-backed memory store. One [`MemoryStore`] owns one connection to one
//! project's database file (FTS5 for lexical match, an `embedding` BLOB column
//! for optional hybrid scoring). All retrieval ranking lives in [`crate::search`].

use std::collections::HashSet;
use std::path::Path;

use lux_core::{AppError, AppResult};
use rusqlite::{
    params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension, Row,
};
use uuid::Uuid;

use crate::model::{
    CategoryCount, Memory, MemoryPatch, MemoryStats, NewMemory, ScoredMemory, SearchOptions,
    SortOrder, DEFAULT_IMPORTANCE,
};
use crate::search::{
    blend, cosine_similarity, decode_embedding, encode_embedding, fts_query, min_max_normalize,
    recency_decay,
};

const SCHEMA: &str = "
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
  access_count INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_memories_category ON memories(category);
CREATE INDEX IF NOT EXISTS idx_memories_pinned ON memories(pinned);

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
";

/// Hard ceiling for a single plain `list` page. A model/tool-driven request can
/// ask for a huge `limit`; without a cap one call could read and allocate the
/// entire memory table. Pagination via `offset` still walks the whole store.
const MAX_LIST_LIMIT: usize = 500;
/// Max embedded rows scanned when merging a semantic-recall pool, so hybrid
/// search stays bounded even on a large store with many embeddings.
const EMBEDDED_SCAN_CAP: usize = 512;

/// Unqualified column list for reads against `memories` alone.
const COLS: &str = "id, category, content, metadata, importance, pinned, source, created_at, updated_at, last_accessed_at, access_count, (embedding IS NOT NULL) AS has_embedding";
/// `memories.`-qualified column list for joins against `memories_fts`
/// (both tables expose `content`/`category`, so they must be disambiguated).
const COLS_Q: &str = "memories.id, memories.category, memories.content, memories.metadata, memories.importance, memories.pinned, memories.source, memories.created_at, memories.updated_at, memories.last_accessed_at, memories.access_count, (memories.embedding IS NOT NULL) AS has_embedding";

/// A SQLite-backed durable memory store for one project.
pub struct MemoryStore {
    conn: Connection,
}

struct Candidate {
    memory: Memory,
    raw_lex: Option<f64>,
    lexical: f64,
    /// Cosine similarity to the query embedding, precomputed when this candidate
    /// came from the embedded-pool scan (so the scoring loop need not re-fetch it).
    embed_sim: Option<f64>,
}

impl MemoryStore {
    /// Open (creating + migrating) the store at `path`.
    pub fn open<P: AsRef<Path>>(path: P) -> AppResult<Self> {
        let conn = Connection::open(path).map_err(to_service)?;
        conn.execute_batch(SCHEMA).map_err(to_service)?;
        Ok(Self { conn })
    }

    /// Open an ephemeral in-memory store (used by tests).
    pub fn open_in_memory() -> AppResult<Self> {
        let conn = Connection::open_in_memory().map_err(to_service)?;
        conn.execute_batch(SCHEMA).map_err(to_service)?;
        Ok(Self { conn })
    }

    /// Create a memory (or upsert when `input.id` names an existing one).
    pub fn create(&self, input: NewMemory) -> AppResult<Memory> {
        let now = now_millis();
        let id = input.id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let metadata = normalize_metadata(input.metadata);
        let metadata_text = serde_json::to_string(&metadata).map_err(to_service)?;
        let importance = input
            .importance
            .unwrap_or(DEFAULT_IMPORTANCE)
            .clamp(0.0, 1.0);
        let pinned = i64::from(input.pinned.unwrap_or(false));
        let embedding = input.embedding.as_deref().map(encode_embedding);
        self.conn
            .execute(
                "INSERT INTO memories
                   (id, category, content, metadata, importance, pinned, source, embedding, created_at, updated_at, last_accessed_at, access_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?9, 0)
                 ON CONFLICT(id) DO UPDATE SET
                   category = excluded.category,
                   content = excluded.content,
                   metadata = excluded.metadata,
                   importance = excluded.importance,
                   pinned = excluded.pinned,
                   source = excluded.source,
                   embedding = COALESCE(excluded.embedding, memories.embedding),
                   updated_at = excluded.updated_at",
                params![id, input.category, input.content, metadata_text, importance, pinned, input.source, embedding, now],
            )
            .map_err(to_service)?;
        self.require(&id)
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

    /// Delete a memory; returns whether a row was removed.
    pub fn delete(&self, id: &str) -> AppResult<bool> {
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

    /// Plain (non-ranked) listing ordered per `opts.sort`, with optional category
    /// filter and offset/limit paging.
    pub fn list(&self, opts: &SearchOptions) -> AppResult<Vec<Memory>> {
        let order = match opts.sort {
            SortOrder::Relevance => "pinned DESC, importance DESC, last_accessed_at DESC",
            SortOrder::Recent => "updated_at DESC",
            SortOrder::Importance => "importance DESC, updated_at DESC",
            SortOrder::Oldest => "created_at ASC",
        };
        let mut sql = format!("SELECT {COLS} FROM memories");
        let mut values: Vec<SqlValue> = Vec::new();
        if let Some(category) = &opts.category {
            sql.push_str(" WHERE category = ?");
            values.push(SqlValue::Text(category.clone()));
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

    /// Ranked retrieval: lexical relevance (FTS5) blended with importance, recency
    /// decay, and a pinned boost. With an empty/queryless call it degrades to a
    /// relevance-ordered recall of the most salient memories.
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
                .map(|memory| Candidate {
                    memory,
                    raw_lex: None,
                    lexical: 0.0,
                    embed_sim: None,
                })
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

        // Normalize the lexical sub-scores across the candidate set (order-preserving).
        let raws: Vec<f64> = candidates
            .iter()
            .filter_map(|candidate| candidate.raw_lex)
            .collect();
        let normalized = min_max_normalize(&raws);
        let mut norm_iter = normalized.into_iter();
        for candidate in &mut candidates {
            candidate.lexical = if candidate.raw_lex.is_some() {
                norm_iter.next().unwrap_or(0.0)
            } else {
                0.0
            };
        }

        let mut scored: Vec<ScoredMemory> = candidates
            .into_iter()
            .map(|candidate| {
                let recency = recency_decay(
                    now - candidate.memory.last_accessed_at,
                    opts.recency_half_life_days,
                );
                let mut lexical = candidate.lexical;
                if let Some(query_vec) = &opts.query_embedding {
                    // Reuse the similarity precomputed by the embedded-pool scan;
                    // otherwise fetch it for lexical/pinned candidates that carry
                    // an embedding but weren't scored during augmentation.
                    let sim = candidate.embed_sim.or_else(|| {
                        if candidate.memory.has_embedding {
                            self.embedding_of(&candidate.memory.id)
                                .ok()
                                .flatten()
                                .map(|stored| f64::from(cosine_similarity(query_vec, &stored)))
                        } else {
                            None
                        }
                    });
                    if let Some(sim) = sim {
                        // Map cosine [-1,1] → [0,1] and let it lift a weak lexical hit.
                        lexical = lexical.max((sim + 1.0) / 2.0);
                    }
                }
                let score = blend(
                    lexical,
                    candidate.memory.importance,
                    recency,
                    candidate.memory.pinned,
                );
                ScoredMemory {
                    memory: candidate.memory,
                    score,
                    lexical,
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
        sql.push_str(&format!(" ORDER BY rank LIMIT {candidate_limit}"));
        let mut stmt = self.conn.prepare(&sql).map_err(to_service)?;
        let rows = stmt
            .query_map(params_from_iter(values), |row| {
                let memory = row_to_memory(row)?;
                // bm25 returns lower = better; negate so larger = stronger match.
                let rank: f64 = row.get(12)?;
                Ok(Candidate {
                    memory,
                    raw_lex: Some(-rank),
                    lexical: 0.0,
                    embed_sim: None,
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
        sql.push_str(" ORDER BY importance DESC, updated_at DESC LIMIT 32");
        let mut stmt = self.conn.prepare(&sql).map_err(to_service)?;
        let rows = stmt
            .query_map(params_from_iter(values), row_to_memory)
            .map_err(to_service)?;
        for memory in collect(rows)? {
            if !present.contains(&memory.id) {
                candidates.push(Candidate {
                    memory,
                    raw_lex: None,
                    lexical: 0.0,
                    embed_sim: None,
                });
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
        sql.push_str(&format!(
            " ORDER BY pinned DESC, importance DESC, last_accessed_at DESC LIMIT {EMBEDDED_SCAN_CAP}"
        ));
        let mut stmt = self.conn.prepare(&sql).map_err(to_service)?;
        let rows = stmt
            .query_map(params_from_iter(values), |row| {
                let memory = row_to_memory(row)?;
                let blob: Vec<u8> = row.get(12)?;
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
            });
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
        has_embedding: row.get::<_, i64>(11)? != 0,
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
        // Max unpinned score is W_LEXICAL+W_IMPORTANCE+W_RECENCY = 1.0; a 2.0 floor clears all.
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
        // is close, must still be recalled via the embedded-pool scan.
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
    fn list_clamps_absurd_limit_to_cap() {
        let store = store();
        for index in 0..3 {
            store
                .create(new_memory("core", &format!("note {index}")))
                .unwrap();
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
}
