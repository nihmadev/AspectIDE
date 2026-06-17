//! Workspace indexing: walk a root, parse every supported file in parallel
//! (bounded so the UI keeps a core via [`lux_core::scan_threads`]), and assemble
//! the resolved [`CodeGraph`].
//!
//! The [`Index`] retains each file's [`ParsedFile`] so a single-file change can be
//! re-parsed and the graph rebuilt without touching the rest of the workspace —
//! the incremental path the file watcher drives. Rebuild is linear in total
//! symbols (cheap next to re-parsing), and only the changed file is re-parsed.

use std::path::{Component, Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::WalkBuilder;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::graph::{CodeGraph, Confidence, Edge, EdgeKind, Node, NodeId};
use crate::lang::Lang;
use crate::parse::{parse_source, ParsedFile, RefKind};
use crate::resolve::{enclosing_def, resolve_targets, Placed, Resolution};

/// Files larger than this are skipped.
///
/// Such files are almost always generated/minified blobs whose symbols add
/// noise, and parsing them would stall indexing. 2 MiB clears any hand-written
/// source.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Hard ceiling on graph nodes.
///
/// A pathological or generated tree can't blow up memory; indexing stops
/// admitting new symbols past this and the graph stays a bounded, useful
/// approximation.
pub const MAX_NODES: usize = 2_000_000;

/// Failures while indexing a workspace.
///
/// Per-file read/parse problems are *not* errors — they are skipped so one bad
/// file never aborts the index. Only a failure to set up the walk/parse
/// machinery surfaces here.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("failed to build the parse thread pool: {0}")]
    ThreadPool(#[from] rayon::ThreadPoolBuildError),
    #[error("workspace root is not a directory: {0}")]
    NotADirectory(PathBuf),
}

/// One parsed source file. The language is re-derived from the path when needed
/// (incremental update), so it isn't stored here.
#[derive(Debug, Clone)]
struct FileEntry {
    parsed: ParsedFile,
}

/// The same admission policy the build walk applies, distilled to a single-path
/// check so the **incremental** update path stays in lock-step with the **build**
/// path. [`collect_source_files`] uses [`WalkBuilder`], which honors `.gitignore`,
/// the global gitignore, and skips hidden entries; a raw watcher event for a
/// gitignored or hidden file would otherwise sneak into the graph on save while a
/// fresh build skipped it — a silent build/incremental divergence.
///
/// Fidelity note: this captures the workspace-root `.gitignore` plus the user's
/// global gitignore (the dominant cases) and the hidden-entry rule. Nested
/// `.gitignore` files below the root are not aggregated here; a full rebuild
/// (workspace open, or a collapsed watch batch) re-applies the complete walk.
#[derive(Debug)]
struct IgnorePolicy {
    root: Gitignore,
    global: Gitignore,
}

impl Default for IgnorePolicy {
    fn default() -> Self {
        Self {
            root: Gitignore::empty(),
            global: Gitignore::empty(),
        }
    }
}

impl IgnorePolicy {
    fn build(root: &Path) -> Self {
        let mut builder = GitignoreBuilder::new(root);
        // `add` returns `Some(err)` on a malformed/absent file — ignored: a missing
        // `.gitignore` simply means nothing is ignored at the root level.
        let _ = builder.add(root.join(".gitignore"));
        let root_gi = builder.build().unwrap_or_else(|_| Gitignore::empty());
        let (global, _) = Gitignore::global();
        Self {
            root: root_gi,
            global,
        }
    }

    /// True when the build walk would have skipped `path`: a hidden path component
    /// (`WalkBuilder`'s default `hidden(true)`) or a gitignore match on the path or
    /// any parent directory.
    fn is_ignored(&self, root: &Path, path: &Path) -> bool {
        if let Ok(relative) = path.strip_prefix(root) {
            let hidden = relative.components().any(|component| match component {
                Component::Normal(name) => name.to_str().is_some_and(|name| name.starts_with('.')),
                _ => false,
            });
            if hidden {
                return true;
            }
        }
        self.root
            .matched_path_or_any_parents(path, false)
            .is_ignore()
            || self
                .global
                .matched_path_or_any_parents(path, false)
                .is_ignore()
    }
}

/// A workspace code-graph index. Build it with [`Index::build`], query the
/// [`CodeGraph`] via [`Index::graph`], and keep it current with
/// [`Index::update_file`] / [`Index::remove_file`].
#[derive(Debug, Default)]
pub struct Index {
    root: PathBuf,
    files: FxHashMap<PathBuf, FileEntry>,
    graph: CodeGraph,
    /// Build-walk admission policy, reused by the incremental path for parity.
    ignore: IgnorePolicy,
}

impl Index {
    /// Index every supported file under `root`, parsing in parallel on a pool
    /// bounded by [`lux_core::concurrency::scan_threads`] so the UI keeps a core.
    pub fn build(root: impl AsRef<Path>) -> Result<Self, IndexError> {
        let root = root.as_ref();
        if !root.is_dir() {
            return Err(IndexError::NotADirectory(root.to_path_buf()));
        }
        let paths = collect_source_files(root);
        let parsed = parse_files_parallel(&paths)?;

        let mut files = FxHashMap::default();
        for (path, _lang, file) in parsed {
            files.insert(path, FileEntry { parsed: file });
        }
        let graph = build_graph(&files);
        Ok(Self {
            ignore: IgnorePolicy::build(root),
            root: root.to_path_buf(),
            files,
            graph,
        })
    }

    /// The resolved, finalized code graph.
    #[must_use]
    pub const fn graph(&self) -> &CodeGraph {
        &self.graph
    }

    /// The workspace root this index was built from.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Number of files currently represented in the index.
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Re-parse a single file and rebuild the graph. Adds the file if new,
    /// refreshes it if changed. A read failure or unsupported extension removes
    /// the file from the index (it may have been deleted or renamed). Returns
    /// `true` when the index changed.
    pub fn update_file(&mut self, path: impl AsRef<Path>) -> bool {
        let changed = self.stage_file(path.as_ref());
        if changed {
            self.graph = build_graph(&self.files);
        }
        changed
    }

    /// Re-parse a batch of changed/added/deleted paths and rebuild the graph
    /// **once**. This is what the file watcher drives: a "Save All" or branch
    /// switch touching N files costs one rebuild, not N. Returns `true` if any
    /// path changed the file set.
    pub fn update_files<P: AsRef<Path>>(&mut self, paths: &[P]) -> bool {
        let mut changed = false;
        for path in paths {
            changed |= self.stage_file(path.as_ref());
        }
        if changed {
            self.graph = build_graph(&self.files);
        }
        changed
    }

    /// Drop a file (e.g. deleted on disk) and rebuild. Returns `true` when the
    /// file was present and removed.
    pub fn remove_file(&mut self, path: impl AsRef<Path>) -> bool {
        if self.files.remove(path.as_ref()).is_some() {
            self.graph = build_graph(&self.files);
            true
        } else {
            false
        }
    }

    /// Re-parse one path into the file map **without** rebuilding the graph. A
    /// path that is unsupported, too large, unreadable, or unparsable is removed
    /// from the map (it may have been deleted or renamed). Returns `true` if the
    /// file set changed (so the caller knows whether a rebuild is warranted).
    fn stage_file(&mut self, path: &Path) -> bool {
        // A fresh build would skip gitignored/hidden files; the incremental path
        // must too, or a save to such a file would diverge the graph from a
        // rebuild. Drop it if it had somehow been admitted before.
        if self.ignore.is_ignored(&self.root, path) {
            return self.files.remove(path).is_some();
        }
        let parsed = Lang::from_path(path).and_then(|lang| {
            let metadata = std::fs::metadata(path).ok()?;
            if metadata.len() > MAX_FILE_BYTES {
                return None;
            }
            let source = std::fs::read_to_string(path).ok()?;
            parse_source(lang, &source).ok()
        });
        match parsed {
            Some(parsed) => {
                self.files.insert(path.to_path_buf(), FileEntry { parsed });
                true
            }
            None => self.files.remove(path).is_some(),
        }
    }
}

/// Walk `root` with standard ignore rules (.gitignore, hidden files, etc.),
/// returning the paths of files in a language the graph understands.
fn collect_source_files(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    // `WalkBuilder` honors .gitignore and skips hidden entries by default, which
    // matches the discovery policy elsewhere in the IDE (no node_modules/target).
    for entry in WalkBuilder::new(root).build().flatten() {
        let path = entry.path();
        if entry.file_type().is_some_and(|t| t.is_file()) && Lang::from_path(path).is_some() {
            paths.push(path.to_path_buf());
        }
    }
    paths
}

/// Parse every path in parallel on a pool capped at the scan-thread budget. Files
/// that are too large, unreadable, or unparsable are skipped, never fatal.
fn parse_files_parallel(paths: &[PathBuf]) -> Result<Vec<(PathBuf, Lang, ParsedFile)>, IndexError> {
    let threads = lux_core::scan_threads();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()?;
    let parsed = pool.install(|| {
        paths
            .par_iter()
            .filter_map(|path| {
                let lang = Lang::from_path(path)?;
                let metadata = std::fs::metadata(path).ok()?;
                if metadata.len() > MAX_FILE_BYTES {
                    return None;
                }
                let source = std::fs::read_to_string(path).ok()?;
                let parsed = parse_source(lang, &source).ok()?;
                Some((path.clone(), lang, parsed))
            })
            .collect()
    });
    Ok(parsed)
}

/// Assemble a resolved [`CodeGraph`] from the parsed files: one node per
/// definition, `Defines` edges for lexical nesting, and reference edges
/// (`Calls`/`Imports`/`Implements`/`References`) linked by name with the locality
/// rules in [`crate::resolve`].
fn build_graph(files: &FxHashMap<PathBuf, FileEntry>) -> CodeGraph {
    let mut graph = CodeGraph::new();

    // Pass 1 — definitions become nodes. Sort paths so node ids are deterministic
    // regardless of the hash-map iteration order.
    let mut paths: Vec<&PathBuf> = files.keys().collect();
    paths.sort_unstable();
    for path in &paths {
        let entry = &files[*path];
        let file_id = graph.add_file((*path).clone());
        for symbol in &entry.parsed.symbols {
            if graph.node_count() >= MAX_NODES {
                break;
            }
            let name = graph.intern(&symbol.name);
            graph.add_node(Node {
                name,
                kind: symbol.kind,
                file: file_id,
                span: symbol.span,
                name_span: symbol.name_span,
            });
        }
    }
    // Build name/file indexes so edge resolution can look definitions up by name.
    graph.finalize();

    // Pass 2 — edges. For each file, locate the enclosing definition of every
    // definition (nesting) and every reference (the edge source), then resolve
    // each reference's name to its target definition(s).
    for path in &paths {
        let entry = &files[*path];
        let Some(&file_id) = graph.file_id_of(path) else {
            continue;
        };
        let placed: Vec<Placed> = graph
            .nodes_in_file(file_id)
            .iter()
            .filter_map(|&node| graph.node(node).map(|n| Placed { node, span: n.span }))
            .collect();

        add_nesting_edges(&mut graph, &placed);
        add_reference_edges(&mut graph, &entry.parsed, file_id, &placed);
    }

    graph.finalize();
    graph
}

/// `Defines` edges from each definition to the definition that lexically encloses
/// it (struct → method, module → function, …). Always [`Confidence::Extracted`] —
/// nesting is a structural fact, not an inference.
fn add_nesting_edges(graph: &mut CodeGraph, placed: &[Placed]) {
    let mut edges = Vec::new();
    for child in placed {
        // Exclude the child itself so a def whose extent covers its own name is
        // not reported as its own parent.
        if let Some(parent) = enclosing_def(placed, child.span, Some(child.node)) {
            edges.push(Edge {
                from: parent,
                to: child.node,
                kind: EdgeKind::Defines,
                confidence: Confidence::Extracted,
            });
        }
    }
    for edge in edges {
        graph.add_edge(edge);
    }
}

/// Reference edges: each [`RawRef`] is attributed to its enclosing definition
/// (the source) and linked to the definition(s) its name resolves to. The edge's
/// confidence reflects *how* the name resolved (local / unique-global / ambiguous).
fn add_reference_edges(
    graph: &mut CodeGraph,
    parsed: &ParsedFile,
    file_id: crate::graph::FileId,
    placed: &[Placed],
) {
    let mut edges = Vec::new();
    for reference in &parsed.refs {
        // A reference is not a node, so nothing to exclude — find the tightest
        // definition whose extent contains it.
        let Some(source) = enclosing_def(placed, reference.span, None) else {
            continue; // file-scope reference: no owning definition to attribute it to
        };
        let global = graph.nodes_by_name(&reference.name);
        if global.is_empty() {
            continue; // external / stdlib name — no node to point at
        }
        let same_file: Vec<NodeId> = global
            .iter()
            .copied()
            .filter(|&n| graph.node(n).is_some_and(|node| node.file == file_id))
            .collect();
        let kind = edge_kind(reference.kind);
        let (targets, resolution) = resolve_targets(&same_file, global, Some(source));
        let confidence = confidence_of(resolution);
        for target in targets {
            edges.push(Edge {
                from: source,
                to: target,
                kind,
                confidence,
            });
        }
    }
    for edge in edges {
        graph.add_edge(edge);
    }
}

const fn confidence_of(resolution: Resolution) -> Confidence {
    match resolution {
        Resolution::Local => Confidence::Extracted,
        Resolution::GlobalUnique => Confidence::Inferred,
        Resolution::Ambiguous => Confidence::Ambiguous,
    }
}

const fn edge_kind(kind: RefKind) -> EdgeKind {
    match kind {
        RefKind::Call => EdgeKind::Calls,
        RefKind::Import => EdgeKind::Imports,
        RefKind::Implement => EdgeKind::Implements,
        RefKind::Reference => EdgeKind::References,
    }
}

#[cfg(test)]
mod tests {
    use super::Index;
    use crate::graph::EdgeKind;
    use std::io::Write;

    /// Materialize a throwaway workspace of `(relative_path, contents)` files and
    /// return its root dir (removed by the caller). Each call gets a unique dir so
    /// tests running in parallel never share a path.
    fn workspace(files: &[(&str, &str)]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = format!(
            "lux-codegraph-idx-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        );
        let root = std::env::temp_dir().join(unique);
        let _ = std::fs::remove_dir_all(&root);
        for (rel, contents) in files {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(contents.as_bytes()).unwrap();
        }
        root
    }

    fn callees(index: &Index, caller: &str) -> Vec<String> {
        let graph = index.graph();
        let mut out = Vec::new();
        for &node in graph.nodes_by_name(caller) {
            for adj in graph.out_neighbors(node) {
                if adj.kind == EdgeKind::Calls {
                    if let Some(name) = graph.name_of(adj.node) {
                        out.push(name.to_string());
                    }
                }
            }
        }
        out.sort();
        out
    }

    fn callers(index: &Index, callee: &str) -> Vec<String> {
        let graph = index.graph();
        let mut out = Vec::new();
        for &node in graph.nodes_by_name(callee) {
            for adj in graph.in_neighbors(node) {
                if adj.kind == EdgeKind::Calls {
                    if let Some(name) = graph.name_of(adj.node) {
                        out.push(name.to_string());
                    }
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn builds_call_edges_within_a_file() {
        let root = workspace(&[("lib.rs", "fn helper() {}\nfn caller() { helper(); }\n")]);
        let index = Index::build(&root).expect("build");

        assert_eq!(callees(&index, "caller"), vec!["helper"]);
        assert_eq!(callers(&index, "helper"), vec!["caller"]);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolves_calls_across_files() {
        let root = workspace(&[
            ("util.rs", "pub fn shared() {}\n"),
            ("main.rs", "fn run() { shared(); }\n"),
        ]);
        let index = Index::build(&root).expect("build");

        assert_eq!(callees(&index, "run"), vec!["shared"]);
        assert_eq!(callers(&index, "shared"), vec!["run"]);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn local_definition_wins_over_cross_file() {
        // Both files define `target`; the call in a.rs must bind to a.rs's local
        // `target`, not b.rs's — so `target` has exactly one caller, `use_local`.
        let root = workspace(&[
            ("a.rs", "fn target() {}\nfn use_local() { target(); }\n"),
            ("b.rs", "fn target() {}\n"),
        ]);
        let index = Index::build(&root).expect("build");

        // Two definitions named target exist; only the local one is called.
        assert_eq!(index.graph().nodes_by_name("target").len(), 2);
        assert_eq!(callers(&index, "target"), vec!["use_local"]);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn nesting_creates_defines_edges() {
        // A function defined inside another function is a real containment case
        // the queries capture: `inner`'s extent sits within `outer`'s extent.
        let root = workspace(&[("s.rs", "fn outer() {\n    fn inner() {}\n}\n")]);
        let index = Index::build(&root).expect("build");
        let graph = index.graph();

        let inner = graph.nodes_by_name("inner");
        assert_eq!(inner.len(), 1);
        let has_defines_parent = graph
            .in_neighbors(inner[0])
            .iter()
            .any(|a| a.kind == EdgeKind::Defines);
        assert!(has_defines_parent, "inner should have a Defines parent");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn update_file_refreshes_the_graph() {
        let root = workspace(&[("m.rs", "fn a() {}\n")]);
        let mut index = Index::build(&root).expect("build");
        assert_eq!(index.graph().nodes_by_name("b").len(), 0);

        // Add a new function and a call to it, then re-index just this file.
        let path = root.join("m.rs");
        std::fs::write(&path, "fn a() { b(); }\nfn b() {}\n").unwrap();
        assert!(index.update_file(&path));

        assert_eq!(index.graph().nodes_by_name("b").len(), 1);
        assert_eq!(callees(&index, "a"), vec!["b"]);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn update_files_batches_one_rebuild() {
        let root = workspace(&[("a.rs", "fn a() {}\n"), ("b.rs", "fn b() {}\n")]);
        let mut index = Index::build(&root).expect("build");

        // Add a symbol to each file, then update both in one batch.
        std::fs::write(root.join("a.rs"), "fn a() { a2(); }\nfn a2() {}\n").unwrap();
        std::fs::write(root.join("b.rs"), "fn b() { b2(); }\nfn b2() {}\n").unwrap();
        let changed = index.update_files(&[root.join("a.rs"), root.join("b.rs")]);
        assert!(changed);
        assert_eq!(index.graph().nodes_by_name("a2").len(), 1);
        assert_eq!(index.graph().nodes_by_name("b2").len(), 1);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn remove_file_drops_its_symbols() {
        let root = workspace(&[("x.rs", "fn gone() {}\n"), ("y.rs", "fn kept() {}\n")]);
        let mut index = Index::build(&root).expect("build");
        assert_eq!(index.graph().nodes_by_name("gone").len(), 1);

        assert!(index.remove_file(root.join("x.rs")));
        assert_eq!(index.graph().nodes_by_name("gone").len(), 0);
        assert_eq!(index.graph().nodes_by_name("kept").len(), 1);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn incremental_skips_gitignored_and_hidden_files() {
        // `.gitignore` excludes the `generated/` tree; a hidden dir is excluded by
        // the walk's default. Neither must enter the graph via the incremental path
        // — that would diverge from a fresh build, which skips both.
        let root = workspace(&[(".gitignore", "generated/\n"), ("src/a.rs", "fn a() {}\n")]);
        let mut index = Index::build(&root).expect("build");

        let gitignored = root.join("generated/g.rs");
        std::fs::create_dir_all(gitignored.parent().unwrap()).unwrap();
        std::fs::write(&gitignored, "fn g() {}\n").unwrap();
        assert!(
            !index.update_file(&gitignored),
            "a gitignored file must not enter the index incrementally"
        );
        assert_eq!(index.graph().nodes_by_name("g").len(), 0);

        let hidden = root.join(".secret/h.rs");
        std::fs::create_dir_all(hidden.parent().unwrap()).unwrap();
        std::fs::write(&hidden, "fn h() {}\n").unwrap();
        assert!(
            !index.update_file(&hidden),
            "a file under a hidden directory must not enter the index incrementally"
        );
        assert_eq!(index.graph().nodes_by_name("h").len(), 0);

        // A normal source file still updates.
        std::fs::write(root.join("src/a.rs"), "fn a() { a2(); }\nfn a2() {}\n").unwrap();
        assert!(index.update_file(root.join("src/a.rs")));
        assert_eq!(index.graph().nodes_by_name("a2").len(), 1);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn confidence_reflects_resolution() {
        use crate::graph::Confidence;
        // Same-file call → Extracted. Unique cross-file call → Inferred.
        let root = workspace(&[
            (
                "a.rs",
                "fn local_def() {}\nfn caller() { local_def(); far(); }\n",
            ),
            ("b.rs", "pub fn far() {}\n"),
        ]);
        let index = Index::build(&root).expect("build");
        let graph = index.graph();

        let caller = graph.nodes_by_name("caller")[0];
        let by_name = |target: &str| {
            let want = graph.nodes_by_name(target)[0];
            graph
                .out_neighbors(caller)
                .iter()
                .find(|a| a.node == want && a.kind == EdgeKind::Calls)
                .map(|a| a.confidence)
        };
        assert_eq!(by_name("local_def"), Some(Confidence::Extracted));
        assert_eq!(by_name("far"), Some(Confidence::Inferred));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn duplicate_calls_collapse_to_one_edge() {
        // `again()` is called twice in one body; the graph is not a multigraph, so
        // there must be exactly one Calls edge (not two parallel ones).
        let root = workspace(&[(
            "lib.rs",
            "fn again() {}\nfn caller() { again(); again(); }\n",
        )]);
        let index = Index::build(&root).expect("build");
        let graph = index.graph();
        let caller = graph.nodes_by_name("caller")[0];
        let again = graph.nodes_by_name("again")[0];
        let edges = graph
            .out_neighbors(caller)
            .iter()
            .filter(|a| a.node == again && a.kind == EdgeKind::Calls)
            .count();
        assert_eq!(
            edges, 1,
            "parallel duplicate calls must collapse to one edge"
        );

        std::fs::remove_dir_all(&root).ok();
    }
}
