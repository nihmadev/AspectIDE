//! High-level structural analyses over a finalized [`CodeGraph`]: god nodes,
//! surprising connections, and import cycles.
//!
//! Ports graphify's `analyze.py` to the typed code-graph model. Because this is
//! an AST-only graph (no LLM-injected concept/doc nodes), the noise filtering is
//! simpler than graphify's: we exclude file-mechanical hubs by edge composition
//! rather than by matching Python label heuristics.

use rustc_hash::FxHashMap;

use crate::community::Community;
use crate::graph::{CodeGraph, Confidence, EdgeKind, NodeId};
use crate::metrics::degree;

/// A most-connected "core abstraction" — the symbols everything leans on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GodNode {
    pub node: NodeId,
    pub name: String,
    pub degree: u32,
}

/// Default number of god nodes to surface, matching graphify's `top_n=10`.
pub const DEFAULT_GOD_NODES: usize = 10;
/// Default number of surprising connections, matching graphify's `top_n=5`.
pub const DEFAULT_SURPRISES: usize = 5;

/// The `top_n` most-connected real definitions, highest degree first.
///
/// Nodes whose only edges are structural nesting (`Defines`) are skipped — they
/// are file/module scaffolding that accrues containment edges mechanically and
/// don't represent architectural hubs, mirroring graphify's file-node exclusion.
/// Ties on degree are broken by node id for determinism.
#[must_use]
pub fn god_nodes(graph: &CodeGraph, top_n: usize) -> Vec<GodNode> {
    let mut ranked: Vec<(NodeId, u32)> = (0..graph.node_count())
        .map(|i| {
            let id = NodeId::from_raw(u32::try_from(i).unwrap_or(u32::MAX));
            (id, degree(graph, id))
        })
        .filter(|&(id, deg)| deg > 0 && !is_structural_only(graph, id))
        .collect();

    // Degree descending, then node id ascending (stable, deterministic).
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(top_n);

    ranked
        .into_iter()
        .map(|(node, deg)| GodNode {
            node,
            name: graph.name_of(node).unwrap_or_default().to_string(),
            degree: deg,
        })
        .collect()
}

/// True when every edge touching `node` is a structural `Defines` edge — i.e. the
/// node only participates in nesting, never in calls/references/imports. Such
/// nodes are containers, not abstractions.
fn is_structural_only(graph: &CodeGraph, node: NodeId) -> bool {
    let non_structural = |adj: &crate::graph::Adjacent| adj.kind != EdgeKind::Defines;
    !graph.out_neighbors(node).iter().any(non_structural)
        && !graph.in_neighbors(node).iter().any(non_structural)
}

/// A connection that isn't obvious from file layout — a cross-file edge between
/// real symbols, ranked by a composite surprise score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Surprise {
    pub source: NodeId,
    pub target: NodeId,
    pub source_name: String,
    pub target_name: String,
    pub relation: EdgeKind,
    pub confidence: Confidence,
    /// What makes it non-obvious (joined reasons), for the report.
    pub why: String,
}

/// Find up to `top_n` surprising cross-file connections.
///
/// Ports `_cross_file_surprises`: skip structural (`Defines`/`Imports`) edges and
/// same-file edges, score the rest, and return the highest. The composite score
/// rewards low confidence, cross-community bridges, and peripheral→hub links —
/// exactly graphify's `_surprise_score`, adapted to the typed edge kinds.
#[must_use]
pub fn surprising_connections(
    graph: &CodeGraph,
    communities: &[Community],
    top_n: usize,
) -> Vec<Surprise> {
    let community_of = crate::community::node_community_map(communities);
    let mut candidates: Vec<(i32, Surprise)> = Vec::new();

    for i in 0..graph.node_count() {
        let from = NodeId::from_raw(u32::try_from(i).unwrap_or(u32::MAX));
        let Some(from_node) = graph.node(from) else {
            continue;
        };
        for adj in graph.out_neighbors(from) {
            // Structural edges are obvious by construction — never surprising.
            if matches!(adj.kind, EdgeKind::Defines | EdgeKind::Imports) {
                continue;
            }
            let to = adj.node;
            let Some(to_node) = graph.node(to) else {
                continue;
            };
            // Only cross-file edges are candidates.
            if from_node.file == to_node.file {
                continue;
            }

            let (score, why) = surprise_score(graph, from, to, adj.confidence, &community_of);
            candidates.push((
                score,
                Surprise {
                    source: from,
                    target: to,
                    source_name: graph.name_of(from).unwrap_or_default().to_string(),
                    target_name: graph.name_of(to).unwrap_or_default().to_string(),
                    relation: adj.kind,
                    confidence: adj.confidence,
                    why,
                },
            ));
        }
    }

    // Score descending; deterministic tie-break on the symbol names + node ids.
    candidates.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.source.cmp(&b.1.source))
            .then_with(|| a.1.target.cmp(&b.1.target))
    });
    candidates.truncate(top_n);
    candidates.into_iter().map(|(_, s)| s).collect()
}

/// Composite "surprise" score for a cross-file edge, plus human reasons.
/// Mirrors `_surprise_score`: confidence weight + cross-community + peripheral→hub.
fn surprise_score(
    graph: &CodeGraph,
    u: NodeId,
    v: NodeId,
    confidence: Confidence,
    community_of: &FxHashMap<NodeId, usize>,
) -> (i32, String) {
    let mut score;
    let mut reasons: Vec<String> = Vec::new();

    // 1. Confidence weight — uncertain connections are more noteworthy.
    score = match confidence {
        Confidence::Ambiguous => 3,
        Confidence::Inferred => 2,
        Confidence::Extracted => 1,
    };
    if confidence != Confidence::Extracted {
        reasons.push(format!(
            "{} connection — not a direct same-file reference",
            confidence.tag().to_lowercase()
        ));
    }

    // 2. Cross-community bridge — the partitioner placed these far apart.
    if let (Some(&cu), Some(&cv)) = (community_of.get(&u), community_of.get(&v)) {
        if cu != cv {
            score += 1;
            reasons.push("bridges separate communities".to_string());
        }
    }

    // 3. Peripheral → hub: a low-degree node reaching a high-degree one.
    let du = degree(graph, u);
    let dv = degree(graph, v);
    if du.min(dv) <= 2 && du.max(dv) >= 5 {
        score += 1;
        let (peripheral, hub) = if du <= dv { (u, v) } else { (v, u) };
        reasons.push(format!(
            "peripheral `{}` unexpectedly reaches hub `{}`",
            graph.name_of(peripheral).unwrap_or_default(),
            graph.name_of(hub).unwrap_or_default(),
        ));
    }

    let why = if reasons.is_empty() {
        "cross-file connection".to_string()
    } else {
        reasons.join("; ")
    };
    (score, why)
}

/// A dependency cycle among files: each file imports/calls into the next, and the
/// last closes back to the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportCycle {
    /// File paths forming the cycle, in order (the closing repeat is implied).
    pub files: Vec<String>,
}

/// Detect cycles in the file-level dependency graph.
///
/// Edges between definitions are projected to their files; a strongly-connected
/// component of size ≥ 2 in that file graph is a cycle. One representative cycle
/// per component is returned (its files in a stable order). Self-imports are
/// ignored. Ports the intent of `find_import_cycles` (file-granular cycles).
#[must_use]
pub fn import_cycles(graph: &CodeGraph) -> Vec<ImportCycle> {
    // Build the file→file adjacency from all non-structural cross-file edges.
    let mut file_adj: FxHashMap<usize, rustc_hash::FxHashSet<usize>> = FxHashMap::default();
    for i in 0..graph.node_count() {
        let from = NodeId::from_raw(u32::try_from(i).unwrap_or(u32::MAX));
        let Some(from_file) = graph.node(from).map(|n| n.file.index()) else {
            continue;
        };
        for adj in graph.out_neighbors(from) {
            if adj.kind == EdgeKind::Defines {
                continue;
            }
            if let Some(to_file) = graph.node(adj.node).map(|n| n.file.index()) {
                if to_file != from_file {
                    file_adj.entry(from_file).or_default().insert(to_file);
                }
            }
        }
    }

    let components = strongly_connected(&file_adj);
    let mut cycles: Vec<ImportCycle> = components
        .into_iter()
        .filter(|component| component.len() >= 2)
        .map(|mut component| {
            component.sort_unstable();
            ImportCycle {
                files: component
                    .into_iter()
                    .map(|f| {
                        graph
                            .file_path(crate::graph::FileId::from_raw(f as u32))
                            .map(|p| p.display().to_string())
                            .unwrap_or_default()
                    })
                    .collect(),
            }
        })
        .collect();
    cycles.sort_by(|a, b| a.files.cmp(&b.files));
    cycles
}

/// Tarjan's strongly-connected components over an integer-keyed adjacency.
fn strongly_connected(
    adjacency: &FxHashMap<usize, rustc_hash::FxHashSet<usize>>,
) -> Vec<Vec<usize>> {
    // Collect the node universe (any file that is a source or a target).
    let mut nodes: rustc_hash::FxHashSet<usize> = rustc_hash::FxHashSet::default();
    for (&u, targets) in adjacency {
        nodes.insert(u);
        for &v in targets {
            nodes.insert(v);
        }
    }

    let mut tarjan = Tarjan {
        adjacency,
        index: FxHashMap::default(),
        low: FxHashMap::default(),
        on_stack: rustc_hash::FxHashSet::default(),
        stack: Vec::new(),
        next: 0,
        components: Vec::new(),
    };
    let mut ordered: Vec<usize> = nodes.into_iter().collect();
    ordered.sort_unstable();
    for v in ordered {
        if !tarjan.index.contains_key(&v) {
            tarjan.run(v);
        }
    }
    tarjan.components
}

/// Mutable state for one [`strongly_connected`] run.
struct Tarjan<'a> {
    adjacency: &'a FxHashMap<usize, rustc_hash::FxHashSet<usize>>,
    index: FxHashMap<usize, u32>,
    low: FxHashMap<usize, u32>,
    on_stack: rustc_hash::FxHashSet<usize>,
    stack: Vec<usize>,
    next: u32,
    components: Vec<Vec<usize>>,
}

/// One frame of the explicit DFS work-stack: a node plus how many of its
/// (sorted) neighbors have already been processed.
struct Frame {
    node: usize,
    neighbors: Vec<usize>,
    next_neighbor: usize,
}

impl Tarjan<'_> {
    /// Iterative Tarjan DFS rooted at `root`. Uses an explicit work-stack instead
    /// of recursion so a deep file-cycle chain can't overflow the call stack.
    fn run(&mut self, root: usize) {
        let mut work: Vec<Frame> = vec![self.enter(root)];

        while let Some(frame) = work.last_mut() {
            let v = frame.node;
            if frame.next_neighbor < frame.neighbors.len() {
                let w = frame.neighbors[frame.next_neighbor];
                frame.next_neighbor += 1;
                if self.index.contains_key(&w) {
                    if self.on_stack.contains(&w) {
                        let idx_w = self.index[&w];
                        let entry = self.low.get_mut(&v).unwrap();
                        *entry = (*entry).min(idx_w);
                    }
                } else {
                    work.push(self.enter(w));
                }
                continue;
            }

            // All neighbors of v processed: close the frame. Propagate low to the
            // parent, then root an SCC if v is a root.
            if self.low[&v] == self.index[&v] {
                let mut component = Vec::new();
                while let Some(w) = self.stack.pop() {
                    self.on_stack.remove(&w);
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                self.components.push(component);
            }
            let low_v = self.low[&v];
            work.pop();
            if let Some(parent) = work.last() {
                let entry = self.low.get_mut(&parent.node).unwrap();
                *entry = (*entry).min(low_v);
            }
        }
    }

    /// Initialize a node's index/low and push it onto the SCC stack, returning its
    /// DFS frame with neighbors in deterministic (sorted) order.
    fn enter(&mut self, v: usize) -> Frame {
        self.index.insert(v, self.next);
        self.low.insert(v, self.next);
        self.next += 1;
        self.stack.push(v);
        self.on_stack.insert(v);
        let mut neighbors: Vec<usize> = self
            .adjacency
            .get(&v)
            .map(|t| t.iter().copied().collect())
            .unwrap_or_default();
        neighbors.sort_unstable();
        Frame {
            node: v,
            neighbors,
            next_neighbor: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{god_nodes, import_cycles, surprising_connections, DEFAULT_GOD_NODES};
    use crate::community;
    use crate::index::Index;
    use std::io::Write;

    fn workspace(files: &[(&str, &str)]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = format!(
            "lux-codegraph-detect-{}-{}",
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
    fn god_node_is_the_most_called_symbol() {
        // `core` is called by three different functions → highest degree.
        let root = workspace(&[(
            "lib.rs",
            "fn core() {}\n\
             fn a() { core(); }\n\
             fn b() { core(); }\n\
             fn c() { core(); }\n",
        )]);
        let index = Index::build(&root).expect("build");
        let gods = god_nodes(index.graph(), DEFAULT_GOD_NODES);

        assert!(!gods.is_empty());
        assert_eq!(gods[0].name, "core", "most-called symbol should rank first");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn surprising_connection_is_cross_file() {
        let root = workspace(&[
            ("a.rs", "fn caller() { helper(); }\n"),
            ("b.rs", "pub fn helper() {}\n"),
        ]);
        let index = Index::build(&root).expect("build");
        let communities = community::detect(index.graph());
        let surprises = surprising_connections(index.graph(), &communities, 5);

        assert!(
            surprises
                .iter()
                .any(|s| s.source_name == "caller" && s.target_name == "helper"),
            "cross-file call should be surprising, got {surprises:?}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn import_cycle_between_two_files_is_detected() {
        // a.rs calls into b.rs and b.rs calls back into a.rs → a 2-file cycle.
        let root = workspace(&[
            ("a.rs", "pub fn a_fn() { b_fn(); }\n"),
            ("b.rs", "pub fn b_fn() { a_fn(); }\n"),
        ]);
        let index = Index::build(&root).expect("build");
        let cycles = import_cycles(index.graph());

        assert_eq!(
            cycles.len(),
            1,
            "expected exactly one file cycle, got {cycles:?}"
        );
        assert_eq!(cycles[0].files.len(), 2);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_cycle_when_dependencies_are_acyclic() {
        let root = workspace(&[
            ("a.rs", "pub fn a_fn() { b_fn(); }\n"),
            ("b.rs", "pub fn b_fn() {}\n"),
        ]);
        let index = Index::build(&root).expect("build");
        assert!(import_cycles(index.graph()).is_empty());

        std::fs::remove_dir_all(&root).ok();
    }
}
