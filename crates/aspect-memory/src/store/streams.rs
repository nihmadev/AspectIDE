//! Internal search-stream helpers: lexical, pinned, embedded, and graph.

use std::collections::HashMap;
use std::collections::HashSet;

use aspect_core::AppResult;
use rusqlite::{params_from_iter, types::Value as SqlValue};

use crate::search::{
    cosine_similarity, decode_embedding,
};
use crate::store::{
    row_to_memory, to_service, Candidate, MemoryStore, COLS, COLS_Q, EMBEDDED_SCAN_CAP,
};
use crate::store::schema::collect;

impl MemoryStore {
    pub(crate) fn lexical_candidates(
        &self,
        match_expr: &str,
        opts: &crate::model::SearchOptions,
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

    pub(crate) fn augment_with_pinned(
        &self,
        candidates: &mut Vec<Candidate>,
        opts: &crate::model::SearchOptions,
    ) -> AppResult<()> {
        let present: HashSet<String> =
            candidates.iter().map(|c| c.memory.id.clone()).collect();
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

    pub(crate) fn augment_with_embedded(
        &self,
        candidates: &mut Vec<Candidate>,
        opts: &crate::model::SearchOptions,
    ) -> AppResult<()> {
        let Some(query_vec) = opts.query_embedding.as_deref() else {
            return Ok(());
        };
        if query_vec.is_empty() {
            return Ok(());
        }
        let present: HashSet<String> =
            candidates.iter().map(|c| c.memory.id.clone()).collect();

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

    pub(crate) fn expand_graph_neighbors(
        &self,
        candidates: &mut Vec<Candidate>,
        seed_ids: &[String],
        opts: &crate::model::SearchOptions,
    ) -> AppResult<()> {
        if seed_ids.is_empty() {
            return Ok(());
        }
        let present: HashSet<String> =
            candidates.iter().map(|c| c.memory.id.clone()).collect();
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
}
