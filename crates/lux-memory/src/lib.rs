//! `lux-memory` — per-project durable agent memory.
//!
//! An [`agentmemory`](https://github.com/rohitg00/agentmemory)-inspired store: a
//! category-namespaced collection of text memories with metadata, an importance
//! score, recency tracking, and similarity search — backed by a single SQLite
//! file per project (FTS5 for lexical match, an optional embedding column for
//! future vector search). Retrieval blends lexical relevance, importance, and
//! recency decay, with a boost for pinned memories.
//!
//! The crate is pure logic with no Tauri/IPC dependency; the desktop layer wraps
//! [`MemoryStore`] in a per-workspace handle and exposes commands + agent tools.

mod model;
mod search;
mod store;

pub use model::{
    Memory, MemoryPatch, MemoryStats, NewMemory, ScoredMemory, SearchOptions, SortOrder,
};
pub use search::cosine_similarity;
pub use store::MemoryStore;
