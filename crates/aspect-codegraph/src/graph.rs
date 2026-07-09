//! Compact in-memory code graph.
//!
//! Design goals (in priority order): cache-friendly traversal, low memory, fast
//! lookup by name and by file.
//!
//! * **String interning** — every symbol name becomes a 32-bit [`Symbol`], so
//!   nodes/edges/indexes store integers, not strings.
//! * **CSR adjacency** — after [`CodeGraph::finalize`], out- and in-edges live in
//!   flat, offset-indexed arrays (compressed sparse row). Neighbor iteration is a
//!   contiguous slice walk with no per-node allocation.
//! * **Builder → finalized** split — nodes and edges are appended cheaply during
//!   the build, then `finalize` computes the indexes and adjacency once. Query
//!   methods assume a finalized graph.

use std::path::{Path, PathBuf};

use rustc_hash::FxHashMap;

use crate::parse::{Span, SymbolKind};

/// An interned symbol name. Index into [`Interner`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbol(u32);

/// A graph node handle. Index into [`CodeGraph`]'s node array.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

impl NodeId {
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Wrap a raw index as a [`NodeId`]. For graph-internal use and tests; callers
    /// must ensure the index refers to a real node before querying with it.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

/// A file handle. Index into [`CodeGraph`]'s file array.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(u32);

impl FileId {
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Wrap a raw index as a [`FileId`]. For graph-internal use and tests.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

/// The relationship an edge encodes. `Defines` is structural (a file/scope
/// defines a symbol); the rest come from resolved references.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    Defines,
    Calls,
    Imports,
    References,
    Implements,
}

/// How much to trust an edge, mirroring graphify's tags.
///
/// In this AST-only port the tag is derived from *how* a reference resolved, not
/// from an LLM: a same-file name match is strong, a unique cross-file match is a
/// label-matching inference, and a multi-candidate match is ambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// Structural fact (nesting) or a unique same-file resolution.
    Extracted,
    /// A unique cross-file name match — likely right, but label-based.
    Inferred,
    /// One of several same-named candidates — recall kept, precision uncertain.
    Ambiguous,
}

impl Confidence {
    /// Numeric weight graphify writes as `confidence_score` (export parity).
    ///
    /// Returns `f64` (not `f32`) so the literal `0.2` serializes as `"0.2"` —
    /// widening `0.2_f32` to f64 would print `0.20000000298…` and break byte
    /// compatibility with Python's `json.dump`.
    #[must_use]
    pub const fn score(self) -> f64 {
        match self {
            Self::Extracted => 1.0,
            Self::Inferred => 0.5,
            Self::Ambiguous => 0.2,
        }
    }

    /// The uppercase tag graphify writes as the `confidence` field.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Extracted => "EXTRACTED",
            Self::Inferred => "INFERRED",
            Self::Ambiguous => "AMBIGUOUS",
        }
    }
}

/// A definition site in the graph. `span` is the definition's full lexical
/// extent (drives nesting/containment); `name_span` locates just the identifier
/// (for display and "go to definition").
#[derive(Debug, Clone, Copy)]
pub struct Node {
    pub name: Symbol,
    pub kind: SymbolKind,
    pub file: FileId,
    pub span: Span,
    pub name_span: Span,
}

/// A directed, kinded edge between two nodes.
#[derive(Debug, Clone, Copy)]
pub struct Edge {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    pub confidence: Confidence,
}

/// One adjacency entry: the neighbor plus the edge kind that reached it.
///
/// The edge direction (whether `node` is a successor or predecessor) is implied
/// by which adjacency list — out or in — the entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Adjacent {
    pub node: NodeId,
    pub kind: EdgeKind,
    pub confidence: Confidence,
}

/// String interner: deduplicates names into stable 32-bit ids.
#[derive(Debug, Default)]
pub struct Interner {
    ids: FxHashMap<Box<str>, Symbol>,
    names: Vec<Box<str>>,
}

impl Interner {
    /// Intern `name`, returning its id (stable for the interner's lifetime).
    pub fn intern(&mut self, name: &str) -> Symbol {
        if let Some(&symbol) = self.ids.get(name) {
            return symbol;
        }
        let symbol =
            Symbol(u32::try_from(self.names.len()).expect("interner exceeded u32 symbols"));
        let boxed: Box<str> = Box::from(name);
        self.names.push(boxed.clone());
        self.ids.insert(boxed, symbol);
        symbol
    }

    /// The id `name` already has, if any (no insertion).
    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<Symbol> {
        self.ids.get(name).copied()
    }

    /// The string behind a symbol id.
    #[must_use]
    pub fn resolve(&self, symbol: Symbol) -> Option<&str> {
        self.names.get(symbol.0 as usize).map(AsRef::as_ref)
    }
}

/// The code graph. Build it with `add_file`/`intern`/`add_node`/`add_edge`, then
/// call [`CodeGraph::finalize`] before querying.
#[derive(Debug, Default)]
pub struct CodeGraph {
    interner: Interner,
    files: Vec<PathBuf>,
    file_ids: FxHashMap<PathBuf, FileId>,
    nodes: Vec<Node>,
    edges: Vec<Edge>,

    // ── Built by `finalize` ──
    by_name: FxHashMap<Symbol, Vec<NodeId>>,
    by_file: FxHashMap<FileId, Vec<NodeId>>,
    /// Per-node lowercase name, parallel to `nodes`. Computed once at finalize so
    /// case-insensitive [`crate::query::resolve`] does not allocate a fresh
    /// lowercase `String` for every node on every lookup (the hot AI-navigation
    /// path). Index by [`NodeId::index`].
    lower_names: Vec<Box<str>>,
    /// Exact case-insensitive name buckets: lowercase name → all nodes whose name
    /// lowercases to it. Lets the dominant exact-match query skip the full scan.
    by_lower_name: FxHashMap<Box<str>, Vec<NodeId>>,
    out_offsets: Vec<u32>,
    out_adjacent: Vec<Adjacent>,
    in_offsets: Vec<u32>,
    in_adjacent: Vec<Adjacent>,
    finalized: bool,
}

impl CodeGraph {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // ── Build ──

    /// Intern a name into the graph's shared interner.
    pub fn intern(&mut self, name: &str) -> Symbol {
        self.interner.intern(name)
    }

    /// Register a file path, returning its id. Idempotent: the same path always
    /// maps to the same [`FileId`].
    pub fn add_file(&mut self, path: PathBuf) -> FileId {
        if let Some(&id) = self.file_ids.get(&path) {
            return id;
        }
        let id = FileId(u32::try_from(self.files.len()).expect("file count exceeded u32"));
        self.files.push(path.clone());
        self.file_ids.insert(path, id);
        id
    }

    /// Append a node, returning its handle. Invalidates any prior finalize.
    pub fn add_node(&mut self, node: Node) -> NodeId {
        let id = NodeId(u32::try_from(self.nodes.len()).expect("node count exceeded u32"));
        self.nodes.push(node);
        self.finalized = false;
        id
    }

    /// Append an edge. Invalidates any prior finalize.
    pub fn add_edge(&mut self, edge: Edge) {
        self.edges.push(edge);
        self.finalized = false;
    }

    /// Compute the name/file indexes and CSR adjacency. Cheap to call repeatedly
    /// (a no-op when already finalized). Must run before any query method.
    pub fn finalize(&mut self) {
        if self.finalized {
            return;
        }
        self.dedup_edges();
        self.build_name_and_file_indexes();
        self.build_lowercase_name_index();
        let node_count = self.nodes.len();
        (self.out_offsets, self.out_adjacent) =
            build_csr(node_count, &self.edges, |edge| (edge.from, edge.to));
        (self.in_offsets, self.in_adjacent) =
            build_csr(node_count, &self.edges, |edge| (edge.to, edge.from));
        self.finalized = true;
    }

    /// Collapse parallel edges sharing `(from, to, kind)` to a single edge,
    /// keeping the first (its confidence — same-name refs resolve identically, so
    /// duplicates always carry the same confidence anyway). The graph is not a
    /// multigraph: two `b()` calls in one function are one `Calls` relationship,
    /// so this keeps degree/edge counts and the exported `links` honest.
    fn dedup_edges(&mut self) {
        let mut seen: rustc_hash::FxHashSet<(NodeId, NodeId, u8)> =
            rustc_hash::FxHashSet::default();
        self.edges
            .retain(|edge| seen.insert((edge.from, edge.to, edge.kind as u8)));
    }

    fn build_name_and_file_indexes(&mut self) {
        self.by_name.clear();
        self.by_file.clear();
        for (index, node) in self.nodes.iter().enumerate() {
            let id = NodeId(u32::try_from(index).expect("node index exceeded u32"));
            self.by_name.entry(node.name).or_default().push(id);
            self.by_file.entry(node.file).or_default().push(id);
        }
    }

    /// Cache each node's lowercase name and an exact case-insensitive bucket map.
    /// Done once here so case-insensitive name resolution is allocation-free in the
    /// query path. Bucket node lists keep node-id order (the `nodes` iteration
    /// order), so callers get stable, deterministic results.
    fn build_lowercase_name_index(&mut self) {
        self.lower_names.clear();
        self.lower_names.reserve(self.nodes.len());
        self.by_lower_name.clear();
        for (index, node) in self.nodes.iter().enumerate() {
            let id = NodeId(u32::try_from(index).expect("node index exceeded u32"));
            let lower: Box<str> = self
                .interner
                .resolve(node.name)
                .unwrap_or("")
                .to_ascii_lowercase()
                .into_boxed_str();
            self.by_lower_name
                .entry(lower.clone())
                .or_default()
                .push(id);
            self.lower_names.push(lower);
        }
    }

    // ── Query (require `finalize`) ──

    #[must_use]
    pub const fn node_count(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub const fn edge_count(&self) -> usize {
        self.edges.len()
    }

    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index())
    }

    /// The interned name of a node, as a string.
    #[must_use]
    pub fn name_of(&self, id: NodeId) -> Option<&str> {
        let node = self.nodes.get(id.index())?;
        self.interner.resolve(node.name)
    }

    #[must_use]
    pub fn file_path(&self, id: FileId) -> Option<&Path> {
        self.files.get(id.index()).map(PathBuf::as_path)
    }

    /// Number of files registered in the graph.
    #[must_use]
    pub const fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Every file as `(id, path)`, in registration order.
    pub fn files(&self) -> impl Iterator<Item = (FileId, &Path)> {
        self.files
            .iter()
            .enumerate()
            .map(|(index, path)| (FileId::from_raw(index as u32), path.as_path()))
    }

    /// The id a path was registered under, if any.
    #[must_use]
    pub fn file_id_of(&self, path: &Path) -> Option<&FileId> {
        self.file_ids.get(path)
    }

    /// All nodes defining `name`. Empty slice when the name is unknown.
    #[must_use]
    pub fn nodes_by_name(&self, name: &str) -> &[NodeId] {
        self.interner
            .lookup(name)
            .and_then(|symbol| self.by_name.get(&symbol))
            .map_or(&[], Vec::as_slice)
    }

    /// All nodes whose name lowercases to `lower` (an exact case-insensitive
    /// match). `lower` must already be lowercase. Empty slice when none match.
    /// O(1) — backed by the bucket map built in [`CodeGraph::finalize`].
    #[must_use]
    pub fn nodes_by_lowercase_name(&self, lower: &str) -> &[NodeId] {
        self.by_lower_name.get(lower).map_or(&[], Vec::as_slice)
    }

    /// The cached lowercase name of a node, or `None` for an out-of-range id.
    /// Populated by [`CodeGraph::finalize`]; avoids re-lowercasing in hot query
    /// loops (prefix/substring matching).
    #[must_use]
    pub fn lowercase_name_of(&self, id: NodeId) -> Option<&str> {
        self.lower_names.get(id.index()).map(AsRef::as_ref)
    }

    /// All nodes defined in `file`. Empty slice when the file has none.
    #[must_use]
    pub fn nodes_in_file(&self, file: FileId) -> &[NodeId] {
        self.by_file.get(&file).map_or(&[], Vec::as_slice)
    }

    /// Successors of `id` (edges pointing *out* of it), as a contiguous slice.
    #[must_use]
    pub fn out_neighbors(&self, id: NodeId) -> &[Adjacent] {
        slice_at(&self.out_offsets, &self.out_adjacent, id)
    }

    /// Predecessors of `id` (edges pointing *into* it), as a contiguous slice.
    #[must_use]
    pub fn in_neighbors(&self, id: NodeId) -> &[Adjacent] {
        slice_at(&self.in_offsets, &self.in_adjacent, id)
    }

    /// Borrow the interner (e.g. to resolve symbols held elsewhere).
    #[must_use]
    pub const fn interner(&self) -> &Interner {
        &self.interner
    }
}

/// Slice the CSR `entries` to the adjacency window for `id` using `offsets`.
/// Returns an empty slice for an out-of-range or finalize-less id.
fn slice_at<'a>(offsets: &[u32], entries: &'a [Adjacent], id: NodeId) -> &'a [Adjacent] {
    let index = id.index();
    let (Some(&start), Some(&end)) = (offsets.get(index), offsets.get(index + 1)) else {
        return &[];
    };
    entries.get(start as usize..end as usize).unwrap_or(&[])
}

/// Build a CSR adjacency (`offsets`, `entries`) from `edges`. `key` extracts
/// `(source, target)` for the desired direction, so the same routine serves both
/// out- and in-adjacency. Counting-sort: O(nodes + edges), no per-node Vec.
fn build_csr(
    node_count: usize,
    edges: &[Edge],
    key: impl Fn(&Edge) -> (NodeId, NodeId),
) -> (Vec<u32>, Vec<Adjacent>) {
    let mut offsets = vec![0_u32; node_count + 1];
    for edge in edges {
        let (source, _) = key(edge);
        offsets[source.index() + 1] += 1;
    }
    for i in 1..=node_count {
        offsets[i] += offsets[i - 1];
    }
    let mut entries = vec![
        Adjacent {
            node: NodeId(0),
            kind: EdgeKind::Defines,
            confidence: Confidence::Extracted
        };
        edges.len()
    ];
    let mut cursor = offsets.clone();
    for edge in edges {
        let (source, target) = key(edge);
        let slot = cursor[source.index()] as usize;
        entries[slot] = Adjacent {
            node: target,
            kind: edge.kind,
            confidence: edge.confidence,
        };
        cursor[source.index()] += 1;
    }
    (offsets, entries)
}

