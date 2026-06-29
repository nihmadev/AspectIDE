//! Community detection over the undirected projection of a [`CodeGraph`].
//!
//! Ports graphify's `cluster.py`: partition the graph into communities, split any
//! that grow past a fraction of the whole, re-split low-cohesion ones, then index
//! by size (community 0 = largest) with a deterministic tie-break.
//!
//! graphify prefers `graspologic`'s Leiden and falls back to `NetworkX` Louvain.
//! We implement Louvain directly — modularity-greedy, seed-free, fully
//! deterministic — which is the portable, dependency-light equivalent of that
//! fallback and produces equivalent community structure for code graphs.

use rustc_hash::FxHashMap;

use crate::graph::{CodeGraph, NodeId};

/// Communities larger than this fraction of all nodes are split further.
const MAX_COMMUNITY_FRACTION: f64 = 0.25;
/// A community is only eligible for the oversized split if it has at least this
/// many nodes (so tiny graphs aren't shredded).
const MIN_SPLIT_SIZE: usize = 10;
/// Communities with cohesion below this (and at least
/// [`COHESION_SPLIT_MIN_SIZE`] nodes) get a second splitting pass.
const COHESION_SPLIT_THRESHOLD: f64 = 0.05;
const COHESION_SPLIT_MIN_SIZE: usize = 50;

/// A detected community: its members and intra-community cohesion.
#[derive(Debug, Clone)]
pub struct Community {
    /// Member nodes, sorted ascending.
    pub members: Vec<NodeId>,
    /// Ratio of present intra-community edges to all possible (0–1).
    pub cohesion: f64,
}

/// Detect communities. Community ids are stable and size-ordered: id 0 is the
/// largest. Returns one [`Community`] per id, in id order.
///
/// An empty graph yields no communities; a graph with nodes but no edges yields
/// one singleton community per node.
#[must_use]
pub fn detect(graph: &CodeGraph) -> Vec<Community> {
    let n = graph.node_count();
    if n == 0 {
        return Vec::new();
    }
    let adjacency = undirected_adjacency(graph);
    let total_edges: usize = adjacency.iter().map(Vec::len).sum::<usize>() / 2;
    if total_edges == 0 {
        // Every node is its own community, size-ordered (all size 1 → by id).
        return (0..n)
            .map(|i| Community {
                members: vec![NodeId::from_raw(i as u32)],
                cohesion: 1.0,
            })
            .collect();
    }

    let partition = louvain(&adjacency, total_edges);

    // Group node indices by community label.
    let mut groups: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    for (node, &label) in partition.iter().enumerate() {
        groups.entry(label).or_default().push(node);
    }
    let mut raw: Vec<Vec<usize>> = groups.into_values().collect();

    // Split oversized communities, then low-cohesion ones (matching cluster.py).
    let max_size = MIN_SPLIT_SIZE.max((n as f64 * MAX_COMMUNITY_FRACTION) as usize);
    raw = raw
        .into_iter()
        .flat_map(|nodes| {
            if nodes.len() > max_size {
                split_community(&adjacency, &nodes)
            } else {
                vec![nodes]
            }
        })
        .collect();
    raw = raw
        .into_iter()
        .flat_map(|nodes| {
            if nodes.len() >= COHESION_SPLIT_MIN_SIZE
                && cohesion(&adjacency, &nodes) < COHESION_SPLIT_THRESHOLD
            {
                let splits = split_community(&adjacency, &nodes);
                if splits.len() > 1 {
                    splits
                } else {
                    vec![nodes]
                }
            } else {
                vec![nodes]
            }
        })
        .collect();

    // Size-descending, with a total-order tie-break on sorted members so identical
    // groupings always get identical ids (cluster.py's determinism fix).
    for nodes in &mut raw {
        nodes.sort_unstable();
    }
    raw.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

    raw.into_iter()
        .map(|nodes| {
            let cohesion = cohesion(&adjacency, &nodes);
            Community {
                members: nodes
                    .into_iter()
                    .map(|i| NodeId::from_raw(i as u32))
                    .collect(),
                cohesion,
            }
        })
        .collect()
}

/// Map each node to the id of the community it belongs to.
#[must_use]
pub fn node_community_map(communities: &[Community]) -> FxHashMap<NodeId, usize> {
    let mut map = FxHashMap::default();
    for (cid, community) in communities.iter().enumerate() {
        for &node in &community.members {
            map.insert(node, cid);
        }
    }
    map
}

/// Cohesion of a member set: present intra-set edges / all possible pairs.
fn cohesion(adjacency: &[Vec<usize>], members: &[usize]) -> f64 {
    let n = members.len();
    if n <= 1 {
        return 1.0;
    }
    let set: rustc_hash::FxHashSet<usize> = members.iter().copied().collect();
    let mut internal = 0_usize;
    for &u in members {
        for &v in &adjacency[u] {
            if u < v && set.contains(&v) {
                internal += 1;
            }
        }
    }
    let possible = n * (n - 1) / 2;
    if possible == 0 {
        0.0
    } else {
        internal as f64 / possible as f64
    }
}

/// One Louvain pass over a member subgraph to split it; returns ≥1 groups.
fn split_community(adjacency: &[Vec<usize>], members: &[usize]) -> Vec<Vec<usize>> {
    let set: rustc_hash::FxHashSet<usize> = members.iter().copied().collect();
    // Induced adjacency restricted to `members`, relabeled 0..members.len().
    let index: FxHashMap<usize, usize> = members.iter().enumerate().map(|(i, &m)| (m, i)).collect();
    let mut sub: Vec<Vec<usize>> = vec![Vec::new(); members.len()];
    let mut sub_edges = 0_usize;
    for (&original, &local) in &index {
        for &v in &adjacency[original] {
            if set.contains(&v) {
                sub[local].push(index[&v]);
                sub_edges += 1;
            }
        }
    }
    sub_edges /= 2;
    if sub_edges == 0 {
        // No internal edges → each member its own group.
        let mut singles: Vec<Vec<usize>> = members.iter().map(|&m| vec![m]).collect();
        singles.sort();
        return singles;
    }
    let partition = louvain(&sub, sub_edges);
    let mut groups: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    for (local, &label) in partition.iter().enumerate() {
        groups.entry(label).or_default().push(members[local]);
    }
    if groups.len() <= 1 {
        return vec![members.to_vec()];
    }
    groups.into_values().collect()
}

/// A weighted undirected graph for one Louvain level. `adjacency[u]` lists
/// `(neighbor, weight)`; a self-loop `(u, w)` carries internal weight accumulated
/// when communities are contracted. `loops[u]` is twice the self-loop weight on
/// `u` (the standard Louvain bookkeeping), `degree[u]` the weighted degree
/// (including loops).
struct WeightedGraph {
    adjacency: Vec<Vec<(usize, f64)>>,
    loops: Vec<f64>,
    degree: Vec<f64>,
    two_m: f64,
}

impl WeightedGraph {
    /// Lift an unweighted adjacency (each edge weight 1) into the level-0 weighted
    /// graph Louvain operates on.
    fn from_unweighted(adjacency: &[Vec<usize>], total_edges: usize) -> Self {
        let weighted: Vec<Vec<(usize, f64)>> = adjacency
            .iter()
            .map(|neighbors| neighbors.iter().map(|&v| (v, 1.0)).collect())
            .collect();
        let degree: Vec<f64> = adjacency.iter().map(|a| a.len() as f64).collect();
        Self {
            adjacency: weighted,
            loops: vec![0.0; adjacency.len()],
            degree,
            two_m: (2 * total_edges) as f64,
        }
    }

    const fn node_count(&self) -> usize {
        self.adjacency.len()
    }
}

/// Multilevel Louvain modularity maximization on an undirected, unweighted
/// adjacency.
///
/// Full Louvain, not just one local-moving pass: repeatedly (1) move nodes greedily
/// to maximize modularity, then (2) **contract** each community into a single
/// super-node — summing inter-community edge weights and folding intra-community
/// edges into self-loops — and recurse on that smaller weighted graph. Levels
/// continue until a pass produces no merge, then every level's labels are projected
/// back down so the result is a per-original-node community label.
///
/// Deterministic: nodes are visited in index order and ties are broken toward the
/// lowest community id at every level.
fn louvain(adjacency: &[Vec<usize>], total_edges: usize) -> Vec<usize> {
    let n = adjacency.len();
    if total_edges == 0 {
        return (0..n).collect();
    }

    let mut level = WeightedGraph::from_unweighted(adjacency, total_edges);
    // `mapping[i]` = the level-0 community label of original node `i`. Updated after
    // every level by composing the new level's partition through it.
    let mut mapping: Vec<usize> = (0..n).collect();

    loop {
        let partition = local_moving(&level);
        // Renumber the raw labels to a contiguous 0..k. `node_label[s]` is the new
        // community of this level's super-node `s` (indexed by super-node index).
        let (node_label, community_count) = renumber(&partition);

        // Compose onto the original nodes: each original node currently maps to a
        // super-node index of *this* level, which now folds into `node_label`.
        for label in &mut mapping {
            *label = node_label[*label];
        }

        // Converged when local moving placed every super-node in its own community:
        // contraction would reproduce the same graph, so stop (else we'd loop
        // forever on a fixed point).
        if community_count == level.node_count() {
            break;
        }

        level = contract(&level, &node_label, community_count);
    }

    mapping
}

/// One local-moving pass: greedily move each node to the neighboring community
/// that most increases modularity, biasing toward staying / the lowest id on ties.
/// Returns a per-node community label (not necessarily contiguous).
fn local_moving(graph: &WeightedGraph) -> Vec<usize> {
    let n = graph.node_count();
    let two_m = graph.two_m;
    if two_m == 0.0 {
        return (0..n).collect();
    }

    let mut community: Vec<usize> = (0..n).collect();
    // Sum of weighted degrees of nodes in each community (seeded as singletons).
    let mut comm_degree: Vec<f64> = graph.degree.clone();

    let mut improved = true;
    while improved {
        improved = false;
        for v in 0..n {
            let current = community[v];
            // Tally edge weight from v to each neighboring community (self-loops on
            // v don't move v relative to itself, so they're excluded here).
            let mut weight_to: FxHashMap<usize, f64> = FxHashMap::default();
            for &(u, w) in &graph.adjacency[v] {
                if u != v {
                    *weight_to.entry(community[u]).or_insert(0.0) += w;
                }
            }
            // Remove v from its community before evaluating gains.
            comm_degree[current] -= graph.degree[v];
            let weight_current = weight_to.get(&current).copied().unwrap_or(0.0);

            // Pick the community maximizing modularity gain; bias toward staying.
            let mut best = current;
            let mut best_gain = weight_current - graph.degree[v] * comm_degree[current] / two_m;
            for (&candidate, &weight) in &weight_to {
                let gain = weight - graph.degree[v] * comm_degree[candidate] / two_m;
                // Strict improvement, or a near-tie resolved toward the lower
                // community id (for determinism independent of iteration order).
                let near_tie = (gain - best_gain).abs() < f64::EPSILON;
                if gain > best_gain || (near_tie && candidate < best) {
                    best_gain = gain;
                    best = candidate;
                }
            }

            comm_degree[best] += graph.degree[v];
            if best != current {
                community[v] = best;
                improved = true;
            }
        }
    }
    community
}

/// Renumber a per-super-node partition to contiguous community ids `0..k`,
/// assigning ids in ascending order of first appearance for determinism. Returns a
/// vec **indexed by super-node** (same length as `partition`) giving each
/// super-node's new community id, plus `k`. Indexing by super-node (not by raw
/// label value) is what lets the caller compose levels directly.
fn renumber(partition: &[usize]) -> (Vec<usize>, usize) {
    let max_label = partition.iter().copied().max().unwrap_or(0);
    let mut remap = vec![usize::MAX; max_label + 1];
    let mut next = 0;
    let mut node_label = vec![0usize; partition.len()];
    for (node, &label) in partition.iter().enumerate() {
        if remap[label] == usize::MAX {
            remap[label] = next;
            next += 1;
        }
        node_label[node] = remap[label];
    }
    (node_label, next)
}

/// Contract `graph` by `partition` (already renumbered to `0..community_count`):
/// every community becomes one super-node, inter-community edge weights are summed,
/// and intra-community edges fold into the super-node's self-loop. `two_m` is
/// invariant under contraction (total edge weight is preserved), so modularity is
/// comparable across levels.
fn contract(graph: &WeightedGraph, partition: &[usize], community_count: usize) -> WeightedGraph {
    // Accumulate weights between super-nodes in a map, plus self-loop weight.
    let mut between: Vec<FxHashMap<usize, f64>> = vec![FxHashMap::default(); community_count];
    let mut loops = vec![0.0; community_count];

    for v in 0..graph.node_count() {
        let cv = partition[v];
        // Carry v's existing self-loop weight into its community's loop.
        loops[cv] += graph.loops[v];
        for &(u, w) in &graph.adjacency[v] {
            let cu = partition[u];
            if cu == cv {
                if u == v {
                    // Self-loop already folded via `graph.loops`; skip to avoid
                    // double counting (loops are stored separately, not in adjacency).
                    continue;
                }
                // Intra-community edge: each undirected edge is seen from both ends,
                // so add w/2 per endpoint visit → full weight once into the loop.
                loops[cv] += w / 2.0;
            } else {
                *between[cv].entry(cu).or_insert(0.0) += w;
            }
        }
    }

    // Materialize the contracted adjacency (deterministic neighbor order) and the
    // weighted degrees (sum of inter-community weights + 2× self-loop, the standard
    // convention that keeps `sum(degree) == two_m`).
    let mut adjacency: Vec<Vec<(usize, f64)>> = Vec::with_capacity(community_count);
    let mut degree = vec![0.0; community_count];
    for c in 0..community_count {
        let mut neighbors: Vec<(usize, f64)> = between[c].iter().map(|(&u, &w)| (u, w)).collect();
        neighbors.sort_unstable_by_key(|&(u, _)| u);
        let inter: f64 = neighbors.iter().map(|&(_, w)| w).sum();
        degree[c] = 2.0_f64.mul_add(loops[c], inter);
        adjacency.push(neighbors);
    }

    WeightedGraph {
        adjacency,
        loops,
        degree,
        two_m: graph.two_m,
    }
}

/// Deduplicated undirected adjacency list indexed by node index.
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
    use super::{detect, node_community_map};
    use crate::graph::{CodeGraph, Confidence, Edge, EdgeKind, Node, NodeId};
    use crate::parse::{Span, SymbolKind};

    const SPAN: Span = Span {
        start_byte: 0,
        end_byte: 0,
        start_row: 0,
        start_col: 0,
        end_row: 0,
        end_col: 0,
    };

    fn add(g: &mut CodeGraph, file: crate::graph::FileId, name: &str) -> NodeId {
        let s = g.intern(name);
        g.add_node(Node {
            name: s,
            kind: SymbolKind::Function,
            file,
            span: SPAN,
            name_span: SPAN,
        })
    }

    fn link(g: &mut CodeGraph, a: NodeId, b: NodeId) {
        g.add_edge(Edge {
            from: a,
            to: b,
            kind: EdgeKind::Calls,
            confidence: Confidence::Extracted,
        });
    }

    #[test]
    fn empty_graph_has_no_communities() {
        assert!(detect(&CodeGraph::new()).is_empty());
    }

    #[test]
    fn edgeless_graph_is_all_singletons() {
        let mut g = CodeGraph::new();
        let file = g.add_file(std::path::PathBuf::from("a.rs"));
        add(&mut g, file, "a");
        add(&mut g, file, "b");
        g.finalize();
        let communities = detect(&g);
        assert_eq!(communities.len(), 2);
        assert!(communities.iter().all(|c| c.members.len() == 1));
    }

    #[test]
    fn two_cliques_split_into_two_communities() {
        // Two 4-cliques joined by a single bridge edge → Louvain should find two
        // communities aligned with the cliques.
        let mut g = CodeGraph::new();
        let file = g.add_file(std::path::PathBuf::from("a.rs"));
        let clique = |g: &mut CodeGraph, names: &[&str]| -> Vec<NodeId> {
            let ids: Vec<NodeId> = names.iter().map(|nm| add(g, file, nm)).collect();
            for i in 0..ids.len() {
                for j in (i + 1)..ids.len() {
                    link(g, ids[i], ids[j]);
                }
            }
            ids
        };
        let left = clique(&mut g, &["a", "b", "c", "d"]);
        let right = clique(&mut g, &["e", "f", "g", "h"]);
        link(&mut g, left[0], right[0]); // single bridge
        g.finalize();

        let communities = detect(&g);
        assert_eq!(
            communities.len(),
            2,
            "expected two communities for two cliques"
        );
        let map = node_community_map(&communities);
        // All of the left clique shares one community; same for the right.
        let lc = map[&left[0]];
        assert!(
            left.iter().all(|n| map[n] == lc),
            "left clique must be one community"
        );
        let rc = map[&right[0]];
        assert!(
            right.iter().all(|n| map[n] == rc),
            "right clique must be one community"
        );
        assert_ne!(lc, rc, "the two cliques must be different communities");
    }

    #[test]
    fn community_zero_is_largest() {
        // A big clique (5) and a small one (3); community 0 must be the big one.
        let mut g = CodeGraph::new();
        let file = g.add_file(std::path::PathBuf::from("a.rs"));
        let clique = |g: &mut CodeGraph, names: &[&str]| -> Vec<NodeId> {
            let ids: Vec<NodeId> = names.iter().map(|nm| add(g, file, nm)).collect();
            for i in 0..ids.len() {
                for j in (i + 1)..ids.len() {
                    link(g, ids[i], ids[j]);
                }
            }
            ids
        };
        let big = clique(&mut g, &["a", "b", "c", "d", "e"]);
        let _small = clique(&mut g, &["x", "y", "z"]);
        g.finalize();

        let communities = detect(&g);
        assert!(communities[0].members.len() >= communities.last().unwrap().members.len());
        let map = node_community_map(&communities);
        assert_eq!(map[&big[0]], 0, "largest clique should be community 0");
    }

    #[test]
    fn contract_preserves_total_edge_weight() {
        // Contraction must keep `two_m` (total weighted degree) invariant — the
        // property that makes modularity comparable across Louvain levels. Build a
        // small unweighted graph, contract by an arbitrary partition, and check the
        // summed weighted degree of the contracted graph equals the original.
        use super::{contract, WeightedGraph};
        // Triangle 0-1-2 plus an edge 2-3 (4 nodes, 4 undirected edges).
        let adjacency = vec![vec![1, 2], vec![0, 2], vec![0, 1, 3], vec![2]];
        let level = WeightedGraph::from_unweighted(&adjacency, 4);
        let original_two_m: f64 = level.degree.iter().sum();

        // Merge {0,1} into community 0 and {2,3} into community 1.
        let node_label = vec![0, 0, 1, 1];
        let contracted = contract(&level, &node_label, 2);
        let contracted_two_m: f64 = contracted.degree.iter().sum();

        assert!(
            (original_two_m - contracted_two_m).abs() < 1e-9,
            "contraction must preserve total weighted degree: {original_two_m} vs {contracted_two_m}"
        );
        assert!(
            (contracted.two_m - level.two_m).abs() < 1e-9,
            "two_m is invariant under contraction"
        );
    }

    #[test]
    fn multilevel_collapses_a_chain_of_aggregations() {
        // Two dense 5-cliques joined by several cross edges form a single optimal
        // community. Full multilevel Louvain must contract each clique to a
        // super-node and then merge the two super-nodes on the next level, ending
        // with ONE community. This exercises the aggregate-and-rerun path the
        // single-pass implementation lacked.
        let mut g = CodeGraph::new();
        let file = g.add_file(std::path::PathBuf::from("a.rs"));
        let clique = |g: &mut CodeGraph, names: &[&str]| -> Vec<NodeId> {
            let ids: Vec<NodeId> = names.iter().map(|nm| add(g, file, nm)).collect();
            for i in 0..ids.len() {
                for j in (i + 1)..ids.len() {
                    link(g, ids[i], ids[j]);
                }
            }
            ids
        };
        let left = clique(&mut g, &["a0", "a1", "a2", "a3", "a4"]);
        let right = clique(&mut g, &["b0", "b1", "b2", "b3", "b4"]);
        // Complete bipartite bridging: the union is a (near-)complete graph, so the
        // unambiguous modularity optimum is a single merged community.
        for &l in &left {
            for &r in &right {
                link(&mut g, l, r);
            }
        }
        g.finalize();

        let communities = detect(&g);
        let map = node_community_map(&communities);
        let c = map[&left[0]];
        assert!(
            left.iter().chain(right.iter()).all(|n| map[n] == c),
            "a densely-bridged pair of cliques must collapse to one community"
        );
    }

    #[test]
    fn detection_is_deterministic() {
        let build = || {
            let mut g = CodeGraph::new();
            let file = g.add_file(std::path::PathBuf::from("a.rs"));
            let ids: Vec<NodeId> = ["a", "b", "c", "d", "e", "f"]
                .iter()
                .map(|nm| add(&mut g, file, nm))
                .collect();
            for i in 0..3 {
                for j in (i + 1)..3 {
                    link(&mut g, ids[i], ids[j]);
                }
            }
            for i in 3..6 {
                for j in (i + 1)..6 {
                    link(&mut g, ids[i], ids[j]);
                }
            }
            g.finalize();
            g
        };
        let a = detect(&build());
        let b = detect(&build());
        let am: Vec<Vec<u32>> = a
            .iter()
            .map(|c| c.members.iter().map(|n| n.index() as u32).collect())
            .collect();
        let bm: Vec<Vec<u32>> = b
            .iter()
            .map(|c| c.members.iter().map(|n| n.index() as u32).collect())
            .collect();
        assert_eq!(am, bm, "community detection must be reproducible");
    }
}
