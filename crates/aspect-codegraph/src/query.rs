//! The query API: the surface the IDE and AI tools call to ask structural
//! questions of a finalized [`CodeGraph`].
//!
//! Ports the query semantics of graphify's `analyze.py` / `serve.py` to the typed
//! graph: name resolution (exact в†’ prefix в†’ substring buckets), callers/callees,
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
/// Ordered exact в†’ prefix в†’ substring (graphify's `_find_node` bucket order).
/// Matching is case-insensitive. Within a bucket, results keep node order
/// (stable). Returns every match so callers can disambiguate; most take the first.
///
/// The exact bucket is an O(1) hash lookup ([`CodeGraph::nodes_by_lowercase_name`]);
/// the prefix/substring buckets scan the per-node lowercase names cached at
/// [`CodeGraph::finalize`], so no lowercase `String` is allocated per node per
/// query вЂ” the difference that matters when AI tools resolve names repeatedly on
/// a multi-million-node graph during a turn.
#[must_use]
pub fn resolve(graph: &CodeGraph, query: &str) -> Vec<NodeRef> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    // Exact case-insensitive matches: direct bucket lookup, no scan.
    let exact = graph.nodes_by_lowercase_name(&needle);

    // Prefix/substring buckets: one pass over cached lowercase names, skipping any
    // node that already landed in the exact bucket (its lowercase == needle).
    let (mut prefix, mut substring) = (Vec::new(), Vec::new());
    for i in 0..graph.node_count() {
        let id = NodeId::from_raw(u32::try_from(i).unwrap_or(u32::MAX));
        let Some(lower) = graph.lowercase_name_of(id) else {
            continue;
        };
        if lower == needle {
            continue; // already in the exact bucket
        }
        if lower.starts_with(needle.as_str()) {
            prefix.push(id);
        } else if lower.contains(needle.as_str()) {
            substring.push(id);
        }
    }
    exact
        .iter()
        .copied()
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
    /// True when the edge runs sourceв†’target in the stored direction (forward),
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
/// unreachable, beyond `max_hops`, or if either endpoint is out of range (e.g.
/// stale [`NodeId`]s from a prior graph build). Mirrors graphify's
/// `shortest_path`: undirected reachability, directional rendering.
#[must_use]
pub fn shortest_path(graph: &CodeGraph, from: NodeId, to: NodeId, max_hops: usize) -> Option<Path> {
    let n = graph.node_count();
    // Guard against stale or out-of-range NodeIds (AI/tool callers can hold ids
    // across graph rebuilds; indexing with a bad id would panic).
    if from.index() >= n || to.index() >= n {
        return None;
    }

    if from == to {
        return NodeRef::of(graph, from).map(|start| Path {
            start,
            steps: Vec::new(),
        });
    }

    // BFS over the undirected view, tracking each node's predecessor.
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

/// Distinct undirected neighbors of a node (successors в€Є predecessors), in a
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
/// orientation). Prefers the forward `u в†’ v` edge; falls back to `v в†’ u`.
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

