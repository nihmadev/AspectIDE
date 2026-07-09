//! Maintenance operations: count, stats, contradiction sweep, retention, prune, wipe.

use std::collections::{HashMap, HashSet};

use aspect_core::AppResult;
use rusqlite::{params, OptionalExtension};

use crate::model::{
    CategoryCount, ContradictionHit, MemoryStats, RelationKind, RetentionReport, RetentionTier,
};
use crate::search::{
    infer_relation_confidence, jaccard_similarity, retention_score, retention_tier, token_set,
    CONTRADICTION_THRESHOLD, MILLIS_PER_DAY,
};
use crate::store::{
    now_millis, row_to_memory, to_service, MemoryStore, COLS,
};
use crate::store::schema::collect;

impl MemoryStore {
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
            .map(|memory: &crate::model::Memory| token_set(&memory.content))
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

        self.conn
            .execute(
                "DELETE FROM memory_relations WHERE source_id NOT IN (SELECT id FROM memories)
                                                  OR target_id NOT IN (SELECT id FROM memories)",
                [],
            )
            .map_err(to_service)?;

        Ok(removed)
    }

    pub fn wipe_category(&self, category: &str) -> AppResult<usize> {
        let removed = self
            .conn
            .execute("DELETE FROM memories WHERE category=?1", params![category])
            .map_err(to_service)?;
        Ok(removed)
    }

    pub fn wipe_all(&self) -> AppResult<usize> {
        let removed = self
            .conn
            .execute("DELETE FROM memories", [])
            .map_err(to_service)?;
        Ok(removed)
    }
}

