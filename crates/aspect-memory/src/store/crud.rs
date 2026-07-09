//! Core CRUD: open, create, get, update, delete, set_embedding.

use std::path::Path;

use aspect_core::{AppError, AppResult};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::model::{
    CreateOutcome, Memory, MemoryPatch, NewMemory, RelationKind, DEFAULT_IMPORTANCE,
};
use crate::search::{
    encode_embedding, infer_relation_confidence, jaccard_similarity, token_set,
    MILLIS_PER_DAY, NEAR_DUPLICATE_THRESHOLD,
};
use crate::store::{
    migrate, normalize_metadata, now_millis, row_to_memory, to_service, MemoryStore,
    COLS, NEAR_DUP_SCAN_LIMIT, SCHEMA_REST, SCHEMA_TABLES,
};

impl MemoryStore {
    pub fn open<P: AsRef<Path>>(path: P) -> AppResult<Self> {
        let conn = Connection::open(path).map_err(to_service)?;
        Self::initialize(conn)
    }

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

    pub fn create(&self, input: NewMemory) -> AppResult<Memory> {
        Ok(self.create_with_outcome(input)?.memory)
    }

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

    fn supersede_near_duplicates(&self, memory: &Memory, now: i64) -> AppResult<Vec<String>> {
        use crate::store::schema::collect;
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
                    crate::model::RelationKind::Supersedes,
                    confidence,
                    now,
                )?;
                superseded_ids.push(candidate.id);
            }
        }
        Ok(superseded_ids)
    }

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

    pub fn set_embedding(&self, id: &str, embedding: &[f32]) -> AppResult<()> {
        self.conn
            .execute(
                "UPDATE memories SET embedding=?2 WHERE id=?1",
                params![id, encode_embedding(embedding)],
            )
            .map_err(to_service)?;
        Ok(())
    }

    fn require(&self, id: &str) -> AppResult<Memory> {
        self.get(id)?
            .ok_or_else(|| AppError::Service(format!("memory {id} missing after write")))
    }

    pub(crate) fn embedding_of(&self, id: &str) -> AppResult<Option<Vec<f32>>> {
        use crate::search::decode_embedding;
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

    pub(crate) fn touch(&self, ids: &[String], now: i64) -> AppResult<()> {
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
