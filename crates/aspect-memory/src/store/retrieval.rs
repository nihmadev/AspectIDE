//! Ranked and plain retrieval: [`list`] and [`search`] with RRF fusion.

use aspect_core::AppResult;
use rusqlite::{params_from_iter, types::Value as SqlValue};

use crate::model::{Memory, ScoredMemory, SearchOptions, SortOrder};
use crate::search::{
    blend_rrf, cosine_similarity, fts_query, min_max_normalize, rank_by_score, recency_decay,
    RrfWeights,
};
use crate::store::{
    now_millis, row_to_memory, to_service, Candidate, MemoryStore, COLS, GRAPH_SEED_COUNT,
    MAX_LIST_LIMIT,
};
use crate::store::schema::collect;

impl MemoryStore {
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

    pub fn search(&self, query: &str, opts: &SearchOptions) -> AppResult<Vec<ScoredMemory>> {
        let now = now_millis();
        let candidate_limit = opts.limit.clamp(1, 40).saturating_mul(5).clamp(10, 200);
        let fts = fts_query(query);

        let mut candidates: Vec<Candidate> = if let Some(match_expr) = &fts {
            self.lexical_candidates(match_expr, opts, candidate_limit)?
        } else {
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

        if opts.query_embedding.is_some() {
            self.augment_with_embedded(&mut candidates, opts)?;
        }
        if let Some(query_vec) = &opts.query_embedding {
            for candidate in &mut candidates {
                if candidate.embed_sim.is_none() && candidate.memory.has_embedding {
                    candidate.embed_sim = self
                        .embedding_of(&candidate.memory.id)?
                        .map(|stored| f64::from(cosine_similarity(query_vec, &stored)));
                }
            }
        }

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
}


