//! The query API: the surface the IDE and AI tools call to ask structural
//! questions of a finalized [`CodeGraph`].
//!
//! Ports the query semantics of graphify's `analyze.py` / `serve.py` to the typed
//! graph: name resolution (exact → prefix → substring buckets), callers/callees,
//! neighbor listing, shortest path (unweighted BFS on the undirected view), and a
//! node "explain". Results are plain data records, ready to serialize for the
//! frontend or fold into an AI tool response.

use std::collections::VecDeque;

use crate::graph::{Adjacent, CodeGraph, Confidence, EdgeKind, NodeId};
use crate::metrics::degree;

/// How many connections [`explain`] includes before truncating, matching
/// graphify's explain limit.
pub const EXPLAIN_CONNECTION_LIMIT: usize = 20;
/// Default hop ceiling for [`shortest_path`], matching graphify's `max_hops=8`.
pub const DEFAULT_MAX_HOPS: usize = 8;

/// A resolved reference to a definition node, with enough context to display it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRef {
    pub node: NodeId,
    pub name: String,
    pub file: String,
    /// 1-based line of the definition's name (editor-friendly).
    pub line: u32,
}

impl NodeRef {
    fn of(graph: &CodeGraph, node: NodeId) -> Option<Self> {
        let data = graph.node(node)?;
        Some(Self {
            node,
            name: graph.name_of(node)?.to_string(),
            file: graph
                .file_path(data.file)
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            line: data.name_span.start_row + 1,
        })
    }
}

/// Resolve a user-supplied symbol string to definition nodes.
///
/// Ordered exact → prefix → substring (graphify's `_find_node` bucket order).
/// Matching is case-insensitive. Within a bucket, results keep node order
/// (stable). Returns every match so callers can disambiguate; most take the first.
#[must_use]
pub fn resolve(graph: &CodeGraph, query: &str) -> Vec<NodeRef> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let (mut exact, mut prefix, mut substring) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..graph.node_count() {
        let id = NodeId::from_raw(u32::try_from(i).unwrap_or(u32::MAX));
        let Some(name) = graph.name_of(id) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if lower == needle {
            exact.push(id);
        } else if lower.starts_with(&needle) {
            prefix.push(id);
        } else if lower.contains(&needle) {
            substring.push(id);
        }
    }
    exact
        .into_iter()
        .chain(prefix)
        .chain(substring)
        .filter_map(|id| NodeRef::of(graph, id))
        .collect()
}

/// The single best node for `query` (first bucket match), or `None`.
#[must_use]
pub fn resolve_one(graph: &CodeGraph, query: &str) -> Option<NodeRef> {
    resolve(graph, query).into_iter().next()
}

/// All call sites *into* a definition: who calls it. Direct callers only (the
/// predecessors on `Calls` edges). Sorted by caller name for stable output.
#[must_use]
pub fn callers(graph: &CodeGraph, node: NodeId) -> Vec<NodeRef> {
    neighbors_of_kind(graph, node, Direction::In, EdgeKind::Calls)
}

/// All definitions a node calls: its callees. Direct callees only (successors on
/// `Calls` edges).
#[must_use]
pub fn callees(graph: &CodeGraph, node: NodeId) -> Vec<NodeRef> {
    neighbors_of_kind(graph, node, Direction::Out, EdgeKind::Calls)
}

/// Direct neighbors of a node in a direction, with the edge that reached each.
/// `None` direction filter means both in and out.
#[must_use]
pub fn neighbors(graph: &CodeGraph, node: NodeId, direction: Option<Direction>) -> Vec<Neighbor> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<Neighbor>, adj: &Adjacent, dir: Direction| {
        if let Some(node_ref) = NodeRef::of(graph, adj.node) {
            out.push(Neighbor {
                node: node_ref,
                relation: adj.kind,
                confidence: adj.confidence,
                direction: dir,
            });
        }
    };
    if direction != Some(Direction::In) {
        for adj in graph.out_neighbors(node) {
            push(&mut out, adj, Direction::Out);
        }
    }
    if direction != Some(Direction::Out) {
        for adj in graph.in_neighbors(node) {
            push(&mut out, adj, Direction::In);
        }
    }
    out
}

/// Edge direction relative to a focus node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Edges leaving the node (its successors / callees / dependencies).
    Out,
    /// Edges entering the node (its predecessors / callers / dependents).
    In,
}

/// A neighbor plus how it connects to the focus node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Neighbor {
    pub node: NodeRef,
    pub relation: EdgeKind,
    pub confidence: Confidence,
    pub direction: Direction,
}

fn neighbors_of_kind(
    graph: &CodeGraph,
    node: NodeId,
    direction: Direction,
    kind: EdgeKind,
) -> Vec<NodeRef> {
    let adjacency = match direction {
        Direction::In => graph.in_neighbors(node),
        Direction::Out => graph.out_neighbors(node),
    };
    let mut refs: Vec<NodeRef> = adjacency
        .iter()
        .filter(|adj| adj.kind == kind)
        .filter_map(|adj| NodeRef::of(graph, adj.node))
        .collect();
    refs.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.node.cmp(&b.node)));
    refs.dedup_by(|a, b| a.node == b.node);
    refs
}

/// A step along a [`shortest_path`]: the edge taken and where it led.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathStep {
    pub to: NodeRef,
    pub relation: EdgeKind,
    /// True when the edge runs source→target in the stored direction (forward),
    /// false when the path traversed it backward (the underlying graph is walked
    /// undirected for connectivity, as graphify does).
    pub forward: bool,
}

/// The shortest connection between two nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    pub start: NodeRef,
    pub steps: Vec<PathStep>,
}

impl Path {
    /// Number of hops (edges) along the path.
    #[must_use]
    pub const fn hops(&self) -> usize {
        self.steps.len()
    }
}

/// Find a shortest path from `from` to `to`.
///
/// Walks the graph as **undirected** for connectivity (unweighted BFS), then
/// recovers each edge's stored direction for display. Returns `None` if
/// unreachable or beyond `max_hops`. Mirrors graphify's `shortest_path`:
/// undirected reachability, directional rendering.
#[must_use]
pub fn shortest_path(graph: &CodeGraph, from: NodeId, to: NodeId, max_hops: usize) -> Option<Path> {
    if from == to {
        return NodeRef::of(graph, from).map(|start| Path {
            start,
            steps: Vec::new(),
        });
    }

    // BFS over the undirected view, tracking each node's predecessor.
    let n = graph.node_count();
    let mut predecessor: Vec<Option<NodeId>> = vec![None; n];
    let mut visited = vec![false; n];
    visited[from.index()] = true;
    let mut queue = VecDeque::new();
    queue.push_back((from, 0usize));

    let mut found = false;
    while let Some((node, depth)) = queue.pop_front() {
        if depth >= max_hops {
            continue;
        }
        for neighbor in undirected_neighbors(graph, node) {
            if !visited[neighbor.index()] {
                visited[neighbor.index()] = true;
                predecessor[neighbor.index()] = Some(node);
                if neighbor == to {
                    found = true;
                    break;
                }
                queue.push_back((neighbor, depth + 1));
            }
        }
        if found {
            break;
        }
    }
    if !found {
        return None;
    }

    // Reconstruct the node sequence from `to` back to `from`.
    let mut chain = vec![to];
    let mut cursor = to;
    while let Some(prev) = predecessor[cursor.index()] {
        chain.push(prev);
        cursor = prev;
        if prev == from {
            break;
        }
    }
    chain.reverse();

    // Turn consecutive node pairs into directional steps.
    let start = NodeRef::of(graph, chain[0])?;
    let mut steps = Vec::with_capacity(chain.len() - 1);
    for pair in chain.windows(2) {
        let (u, v) = (pair[0], pair[1]);
        let (relation, forward) = edge_between(graph, u, v);
        steps.push(PathStep {
            to: NodeRef::of(graph, v)?,
            relation,
            forward,
        });
    }
    Some(Path { start, steps })
}

/// A node's structural summary: where it is, how connected, and its strongest
/// connections (both directions), degree-sorted and truncated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    pub node: NodeRef,
    pub kind: crate::parse::SymbolKind,
    pub degree: u32,
    /// Total connections before truncation (so the UI can show "+N more").
    pub total_connections: usize,
    /// Up to [`EXPLAIN_CONNECTION_LIMIT`] connections, highest-degree neighbor
    /// first, out-edges before in-edges on ties.
    pub connections: Vec<Neighbor>,
}

/// Describe a node: its metadata, degree, and top connections (graphify's
/// `explain`). Connections are sorted by neighbor degree descending and capped at
/// [`EXPLAIN_CONNECTION_LIMIT`].
#[must_use]
pub fn explain(graph: &CodeGraph, node: NodeId) -> Option<Explanation> {
    let node_ref = NodeRef::of(graph, node)?;
    let data = graph.node(node)?;
    let mut connections = neighbors(graph, node, None);
    let total = connections.len();

    // Out before in on a degree tie (graphify lists successors first), then by
    // neighbor degree descending.
    connections.sort_by(|a, b| {
        degree(graph, b.node.node)
            .cmp(&degree(graph, a.node.node))
            .then_with(|| direction_rank(a.direction).cmp(&direction_rank(b.direction)))
            .then_with(|| a.node.node.cmp(&b.node.node))
    });
    connections.truncate(EXPLAIN_CONNECTION_LIMIT);

    Some(Explanation {
        node: node_ref,
        kind: data.kind,
        degree: degree(graph, node),
        total_connections: total,
        connections,
    })
}

const fn direction_rank(direction: Direction) -> u8 {
    match direction {
        Direction::Out => 0,
        Direction::In => 1,
    }
}

/// Distinct undirected neighbors of a node (successors ∪ predecessors), in a
/// deterministic order.
fn undirected_neighbors(graph: &CodeGraph, node: NodeId) -> Vec<NodeId> {
    let mut neighbors: Vec<NodeId> = graph
        .out_neighbors(node)
        .iter()
        .chain(graph.in_neighbors(node).iter())
        .map(|adj| adj.node)
        .filter(|&n| n != node)
        .collect();
    neighbors.sort_unstable();
    neighbors.dedup();
    neighbors
}

/// The relation and stored direction of the edge between `u` and `v` (in either
/// orientation). Prefers the forward `u → v` edge; falls back to `v → u`.
fn edge_between(graph: &CodeGraph, u: NodeId, v: NodeId) -> (EdgeKind, bool) {
    if let Some(adj) = graph.out_neighbors(u).iter().find(|a| a.node == v) {
        return (adj.kind, true);
    }
    if let Some(adj) = graph.out_neighbors(v).iter().find(|a| a.node == u) {
        return (adj.kind, false);
    }
    // Should not happen for a reconstructed path, but stay total.
    (EdgeKind::References, true)
}

#[cfg(test)]
mod tests {
    use super::{
        callees, callers, explain, neighbors, resolve, resolve_one, shortest_path, Direction,
        DEFAULT_MAX_HOPS,
    };
    use crate::index::Index;
    use std::io::Write;

    fn workspace(files: &[(&str, &str)]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = format!(
            "lux-codegraph-query-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        );
        let root = std::env::temp_dir().join(unique);
        let _ = std::fs::remove_dir_all(&root);
        for (rel, contents) in files {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::File::create(&path)
                .unwrap()
                .write_all(contents.as_bytes())
                .unwrap();
        }
        root
    }

    #[test]
    fn resolve_orders_exact_then_prefix_then_substring() {
        let root = workspace(&[("lib.rs", "fn run() {}\nfn runner() {}\nfn rerun_all() {}\n")]);
        let index = Index::build(&root).expect("build");
        let names: Vec<String> = resolve(index.graph(), "run")
            .into_iter()
            .map(|r| r.name)
            .collect();
        // exact "run", prefix "runner", substring "rerun_all".
        assert_eq!(names, vec!["run", "runner", "rerun_all"]);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn callers_and_callees_are_inverse() {
        let root = workspace(&[(
            "lib.rs",
            "fn leaf() {}\nfn mid() { leaf(); }\nfn top() { mid(); }\n",
        )]);
        let index = Index::build(&root).expect("build");
        let graph = index.graph();

        let mid = resolve_one(graph, "mid").unwrap().node;
        let outgoing: Vec<String> = callees(graph, mid).into_iter().map(|r| r.name).collect();
        let incoming: Vec<String> = callers(graph, mid).into_iter().map(|r| r.name).collect();
        assert_eq!(outgoing, vec!["leaf"]);
        assert_eq!(incoming, vec!["top"]);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn shortest_path_connects_call_chain() {
        let root = workspace(&[(
            "lib.rs",
            "fn leaf() {}\nfn mid() { leaf(); }\nfn top() { mid(); }\n",
        )]);
        let index = Index::build(&root).expect("build");
        let graph = index.graph();
        let top = resolve_one(graph, "top").unwrap().node;
        let leaf = resolve_one(graph, "leaf").unwrap().node;

        let path = shortest_path(graph, top, leaf, DEFAULT_MAX_HOPS).expect("path");
        assert_eq!(path.hops(), 2);
        assert_eq!(path.start.name, "top");
        assert_eq!(path.steps.last().unwrap().to.name, "leaf");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn shortest_path_returns_none_when_unreachable() {
        let root = workspace(&[
            ("a.rs", "fn island_a() {}\n"),
            ("b.rs", "fn island_b() {}\n"),
        ]);
        let index = Index::build(&root).expect("build");
        let graph = index.graph();
        let first = resolve_one(graph, "island_a").unwrap().node;
        let second = resolve_one(graph, "island_b").unwrap().node;
        assert!(shortest_path(graph, first, second, DEFAULT_MAX_HOPS).is_none());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn neighbors_respects_direction_filter() {
        let root = workspace(&[(
            "lib.rs",
            "fn leaf() {}\nfn mid() { leaf(); }\nfn top() { mid(); }\n",
        )]);
        let index = Index::build(&root).expect("build");
        let graph = index.graph();
        let mid = resolve_one(graph, "mid").unwrap().node;

        let outs = neighbors(graph, mid, Some(Direction::Out));
        assert!(outs.iter().all(|n| n.direction == Direction::Out));
        assert!(outs.iter().any(|n| n.node.name == "leaf"));

        let ins = neighbors(graph, mid, Some(Direction::In));
        assert!(ins.iter().any(|n| n.node.name == "top"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn explain_reports_degree_and_connections() {
        let root = workspace(&[(
            "lib.rs",
            "fn hub() {}\nfn a() { hub(); }\nfn b() { hub(); }\n",
        )]);
        let index = Index::build(&root).expect("build");
        let graph = index.graph();
        let hub = resolve_one(graph, "hub").unwrap().node;

        let explanation = explain(graph, hub).expect("explain");
        assert_eq!(explanation.node.name, "hub");
        // hub is called by a and b → degree 2, two in-connections.
        assert_eq!(explanation.degree, 2);
        assert_eq!(explanation.total_connections, 2);
        assert!(explanation
            .connections
            .iter()
            .all(|c| c.direction == Direction::In));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn empty_query_resolves_to_nothing() {
        let root = workspace(&[("lib.rs", "fn x() {}\n")]);
        let index = Index::build(&root).expect("build");
        assert!(resolve(index.graph(), "   ").is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}
