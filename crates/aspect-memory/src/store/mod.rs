//! SQLite-backed memory store. One [`MemoryStore`] owns one connection to one
//! project's database file (FTS5 for lexical match, an `embedding` BLOB column
//! for optional hybrid scoring, and a `memory_relations` edge table for the
//! knowledge-graph-lite). All retrieval ranking lives in [`crate::search`].

pub mod crud;
pub mod maintenance;
pub mod relations;
pub mod retrieval;
pub mod schema;
pub mod streams;

use crate::model::Memory;

pub(crate) use schema::{
    migrate, normalize_metadata, now_millis, row_to_memory, row_to_relation, to_service,
};

/// Schema generation stamped into `PRAGMA user_version`.
pub(crate) const SCHEMA_VERSION: i64 = 1;

pub(crate) const SCHEMA_TABLES: &str = "
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

pub(crate) const SCHEMA_REST: &str = "
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

pub(crate) const MAX_LIST_LIMIT: usize = 500;
pub(crate) const EMBEDDED_SCAN_CAP: usize = 512;
pub(crate) const NEAR_DUP_SCAN_LIMIT: usize = 512;
pub(crate) const GRAPH_SEED_COUNT: usize = 5;
pub(crate) const RELATED_VISITED_CAP: usize = 500;

pub(crate) const COLS: &str = "id, category, content, metadata, importance, pinned, source, created_at, updated_at, last_accessed_at, access_count, superseded, forget_after, (embedding IS NOT NULL) AS has_embedding";
pub(crate) const COLS_Q: &str = "memories.id, memories.category, memories.content, memories.metadata, memories.importance, memories.pinned, memories.source, memories.created_at, memories.updated_at, memories.last_accessed_at, memories.access_count, memories.superseded, memories.forget_after, (memories.embedding IS NOT NULL) AS has_embedding";

pub struct MemoryStore {
    pub(crate) conn: rusqlite::Connection,
}

pub(crate) struct Candidate {
    pub(crate) memory: Memory,
    pub(crate) raw_lex: Option<f64>,
    pub(crate) lexical: f64,
    pub(crate) embed_sim: Option<f64>,
    pub(crate) graph_score: Option<f64>,
}

impl Candidate {
    pub(crate) fn from_memory(memory: Memory) -> Self {
        Self {
            memory,
            raw_lex: None,
            lexical: 0.0,
            embed_sim: None,
            graph_score: None,
        }
    }

    pub(crate) fn bm25_entry(&self) -> Option<(String, f64)> {
        self.raw_lex.map(|score| (self.memory.id.clone(), score))
    }

    pub(crate) fn vector_entry(&self) -> Option<(String, f64)> {
        self.embed_sim.map(|score| (self.memory.id.clone(), score))
    }

    pub(crate) fn graph_entry(&self) -> Option<(String, f64)> {
        self.graph_score
            .map(|score| (self.memory.id.clone(), score))
    }
}
