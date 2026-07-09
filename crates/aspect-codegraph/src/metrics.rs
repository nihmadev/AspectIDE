//! Graph metrics over a finalized [`CodeGraph`]: degree, betweenness centrality,
//! and edge-confidence statistics.
//!
//! These are the structural signals the higher-level analyses ([`crate::detect`])
//! build on. They read the CSR adjacency directly and never mutate the graph, so
//! they are cheap to recompute after an incremental rebuild.

use rustc_hash::FxHashMap;

use crate::graph::{CodeGraph, Confidence, NodeId};

/// Total degree (in + out) of every node, indexed by node. Parallel edges each
/// count, matching graphify's `dict(G.degree())` semantics.
#[must_use]
pub fn degrees(graph: &CodeGraph) -> Vec<u32> {
    (0..graph.node_count())
        .map(|i| {
            let id = NodeId::from_raw(u32::try_from(i).unwrap_or(u32::MAX));
            let out = graph.out_neighbors(id).len();
            let inn = graph.in_neighbors(id).len();
            u32::try_from(out + inn).unwrap_or(u32::MAX)
        })
        .collect()
}

/// The degree of a single node (in + out).
#[must_use]
pub fn degree(graph: &CodeGraph, node: NodeId) -> u32 {
    let out = graph.out_neighbors(node).len();
    let inn = graph.in_neighbors(node).len();
    u32::try_from(out + inn).unwrap_or(u32::MAX)
}

/// Counts of edges by confidence tag across the whole graph.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConfidenceCounts {
    pub extracted: usize,
    pub inferred: usize,
    pub ambiguous: usize,
}

impl ConfidenceCounts {
    #[must_use]
    pub const fn total(self) -> usize {
        self.extracted + self.inferred + self.ambiguous
    }

    /// Percentage (0–100, rounded) of edges with the given tag, matching the
    /// `report.py` Summary line. Total is floored at 1 to avoid division by zero.
    #[must_use]
    pub fn percent(self, of: Confidence) -> u32 {
        let total = self.total().max(1) as f64;
        let count = match of {
            Confidence::Extracted => self.extracted,
            Confidence::Inferred => self.inferred,
            Confidence::Ambiguous => self.ambiguous,
        } as f64;
        (count / total * 100.0).round() as u32
    }
}

/// Tally every edge by confidence. Each directed edge is counted once (we read
/// out-adjacency only, so an edge is not double-counted from both endpoints).
#[must_use]
pub fn confidence_counts(graph: &CodeGraph) -> ConfidenceCounts {
    let mut counts = ConfidenceCounts::default();
    for i in 0..graph.node_count() {
        let id = NodeId::from_raw(u32::try_from(i).unwrap_or(u32::MAX));
        for adj in graph.out_neighbors(id) {
            match adj.confidence {
                Confidence::Extracted => counts.extracted += 1,
                Confidence::Inferred => counts.inferred += 1,
                Confidence::Ambiguous => counts.ambiguous += 1,
            }
        }
    }
    counts
}

/// Betweenness centrality for every node, via Brandes' algorithm.
///
/// Runs on the **undirected** view of the graph (matching graphify, which
/// computes centrality on the undirected projection). Returns a map from node to
/// its centrality score, normalized to `[0, 1]` for graphs with ≥ 3 nodes.
///
/// Cost is O(V·E). For large graphs the caller should gate on node count (the
/// detect layer skips betweenness past a threshold, as graphify does).
#[must_use]
pub fn betweenness(graph: &CodeGraph) -> FxHashMap<NodeId, f64> {
    let n = graph.node_count();
    let mut centrality: FxHashMap<NodeId, f64> =
        (0..n).map(|i| (NodeId::from_raw(i as u32), 0.0)).collect();
    if n < 3 {
        return centrality;
    }

    // Undirected neighbor list per node (dedup parallel edges for traversal).
    let adjacency = undirected_adjacency(graph);

    for s in 0..n {
        // Single-source shortest paths (unweighted BFS) with predecessor tracking.
        let mut stack: Vec<usize> = Vec::new();
        let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut sigma = vec![0.0_f64; n];
        let mut distance = vec![-1_i64; n];
        sigma[s] = 1.0;
        distance[s] = 0;
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(s);

        while let Some(v) = queue.pop_front() {
            stack.push(v);
            for &w in &adjacency[v] {
                if distance[w] < 0 {
                    distance[w] = distance[v] + 1;
                    queue.push_back(w);
                }
                if distance[w] == distance[v] + 1 {
                    sigma[w] += sigma[v];
                    predecessors[w].push(v);
                }
            }
        }

        // Accumulation phase (back-propagate dependencies).
        let mut delta = vec![0.0_f64; n];
        while let Some(w) = stack.pop() {
            for &v in &predecessors[w] {
                if sigma[w] > f64::EPSILON {
                    let ratio = sigma[v] / sigma[w];
                    delta[v] = ratio.mul_add(1.0 + delta[w], delta[v]);
                }
            }
            if w != s {
                *centrality.get_mut(&NodeId::from_raw(w as u32)).unwrap() += delta[w];
            }
        }
    }

    // Undirected Brandes accumulates each unordered pair's dependency from both
    // endpoints (halve), then normalize by the possible-pairs count so scores are
    // comparable across graph sizes. The two factors combine to 1/((n-1)(n-2)),
    // matching NetworkX `betweenness_centrality(normalized=True)` on undirected.
    let factor = 1.0 / ((n - 1) as f64 * (n - 2) as f64);
    for value in centrality.values_mut() {
        *value *= factor;
    }
    centrality
}

/// Build a deduplicated undirected adjacency list (Vec indexed by node index).
fn undirected_adjacency(graph: &CodeGraph) -> Vec<Vec<usize>> {
    let n = graph.node_count();
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in 0..n {
        let id = NodeId::from_raw(u32::try_from(i).unwrap_or(u32::MAX));
        for adj in graph.out_neighbors(id) {
            let j = adj.node.index();
            if j != i {
                adjacency[i].push(j);
                adjacency[j].push(i);
            }
        }
    }
    for neighbors in &mut adjacency {
        neighbors.sort_unstable();
        neighbors.dedup();
    }
    adjacency
}

