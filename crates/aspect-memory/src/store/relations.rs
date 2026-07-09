//! Relation management: knowledge-graph-lite edges between memories.

use std::collections::HashSet;

use aspect_core::{AppError, AppResult};
use rusqlite::params;
use uuid::Uuid;

use crate::model::{MemoryRelation, RelationKind};
use crate::search::infer_relation_confidence;
use crate::store::{now_millis, row_to_relation, to_service, MemoryStore, RELATED_VISITED_CAP};
use crate::store::schema::collect;

impl MemoryStore {
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

    pub(crate) fn upsert_relation(
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

    pub fn related(
        &self,
        id: &str,
        max_hops: usize,
        min_confidence: f64,
    ) -> AppResult<Vec<crate::model::RelatedMemory>> {
        let max_hops = max_hops.clamp(1, 5);
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(id.to_string());
        let mut frontier: Vec<(String, f64)> = vec![(id.to_string(), 1.0)];
        let mut results: Vec<crate::model::RelatedMemory> = Vec::new();

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
                    if combined < min_confidence {
                        continue;
                    }
                    if let Some(memory) = self.get(&neighbor_id)? {
                        results.push(crate::model::RelatedMemory {
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
}
