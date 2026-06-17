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

#[cfg(test)]
mod tests {
    use super::{betweenness, confidence_counts, degree, degrees, ConfidenceCounts};
    use crate::graph::{CodeGraph, Confidence, Edge, EdgeKind, Node};
    use crate::parse::{Span, SymbolKind};

    const SPAN: Span = Span {
        start_byte: 0,
        end_byte: 0,
        start_row: 0,
        start_col: 0,
        end_row: 0,
        end_col: 0,
    };

    /// Build a path graph a—b—c—d—e (undirected via directed edges) so `c` is the
    /// obvious betweenness bridge.
    fn path_graph(len: usize) -> CodeGraph {
        let mut g = CodeGraph::new();
        let file = g.add_file(std::path::PathBuf::from("p.rs"));
        let mut nodes = Vec::new();
        for i in 0..len {
            let sym = g.intern(&format!("n{i}"));
            nodes.push(g.add_node(Node {
                name: sym,
                kind: SymbolKind::Function,
                file,
                span: SPAN,
                name_span: SPAN,
            }));
        }
        for pair in nodes.windows(2) {
            g.add_edge(Edge {
                from: pair[0],
                to: pair[1],
                kind: EdgeKind::Calls,
                confidence: Confidence::Extracted,
            });
        }
        g.finalize();
        g
    }

    #[test]
    fn degree_counts_both_directions() {
        let g = path_graph(3); // n0 -> n1 -> n2
        let n1 = g.nodes_by_name("n1")[0];
        // n1 has one in (from n0) and one out (to n2) → degree 2.
        assert_eq!(degree(&g, n1), 2);
        let all = degrees(&g);
        assert_eq!(all[n1.index()], 2);
    }

    #[test]
    fn betweenness_peaks_at_the_bridge() {
        let g = path_graph(5); // n0-n1-n2-n3-n4, n2 is the center
        let bc = betweenness(&g);
        let center = g.nodes_by_name("n2")[0];
        let end = g.nodes_by_name("n0")[0];
        assert!(
            bc[&center] > bc[&end],
            "center {} should outrank endpoint {}",
            bc[&center],
            bc[&end]
        );
        // Endpoints lie on no shortest path between others → ~0.
        assert!(bc[&end].abs() < 1e-9);
    }

    #[test]
    fn betweenness_is_empty_below_three_nodes() {
        let g = path_graph(2);
        let bc = betweenness(&g);
        assert!(bc.values().all(|&v| v.abs() < f64::EPSILON));
    }

    #[test]
    fn confidence_counts_and_percentages() {
        let mut g = CodeGraph::new();
        let file = g.add_file(std::path::PathBuf::from("c.rs"));
        let mk = |g: &mut CodeGraph, name: &str| {
            let s = g.intern(name);
            g.add_node(Node {
                name: s,
                kind: SymbolKind::Function,
                file,
                span: SPAN,
                name_span: SPAN,
            })
        };
        let n_a = mk(&mut g, "a");
        let n_b = mk(&mut g, "b");
        let n_c = mk(&mut g, "c");
        let n_d = mk(&mut g, "d");
        g.add_edge(Edge {
            from: n_a,
            to: n_b,
            kind: EdgeKind::Calls,
            confidence: Confidence::Extracted,
        });
        g.add_edge(Edge {
            from: n_a,
            to: n_c,
            kind: EdgeKind::Calls,
            confidence: Confidence::Inferred,
        });
        g.add_edge(Edge {
            from: n_a,
            to: n_d,
            kind: EdgeKind::Calls,
            confidence: Confidence::Ambiguous,
        });
        g.add_edge(Edge {
            from: n_b,
            to: n_c,
            kind: EdgeKind::Calls,
            confidence: Confidence::Extracted,
        });
        g.finalize();

        let counts = confidence_counts(&g);
        assert_eq!(
            counts,
            ConfidenceCounts {
                extracted: 2,
                inferred: 1,
                ambiguous: 1
            }
        );
        assert_eq!(counts.total(), 4);
        assert_eq!(counts.percent(Confidence::Extracted), 50);
        assert_eq!(counts.percent(Confidence::Inferred), 25);
    }

    #[test]
    fn empty_graph_has_no_confidence() {
        let g = CodeGraph::new();
        assert_eq!(confidence_counts(&g).total(), 0);
        assert_eq!(confidence_counts(&g).percent(Confidence::Extracted), 0);
    }

    #[test]
    fn betweenness_handles_disconnected_node() {
        let mut g = path_graph(4);
        // Add an isolated node; betweenness must stay finite and zero for it.
        let lone = g.intern("lone");
        let file = g.add_file(std::path::PathBuf::from("p.rs"));
        let id = g.add_node(Node {
            name: lone,
            kind: SymbolKind::Function,
            file,
            span: SPAN,
            name_span: SPAN,
        });
        g.finalize();
        let bc = betweenness(&g);
        assert!(bc[&id].abs() < f64::EPSILON);
    }
}
