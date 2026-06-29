//! Workspace indexing: walk a root, parse every supported file in parallel
//! (bounded so the UI keeps a core via [`lux_core::scan_threads`]), and assemble
//! the resolved [`CodeGraph`].
//!
//! The [`Index`] retains each file's [`ParsedFile`] so a single-file change can be
//! re-parsed and the graph rebuilt without touching the rest of the workspace —
//! the incremental path the file watcher drives. Rebuild is linear in total
//! symbols (cheap next to re-parsing), and only the changed file is re-parsed.

use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::{Match, WalkBuilder, WalkState};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::cache::{self, FileMeta, PriorCache};
use crate::graph::{CodeGraph, Confidence, Edge, EdgeKind, FileId, Node, NodeId};
use crate::lang::Lang;
use crate::parse::{parse_source, ParsedFile, RefKind, Span, SymbolKind};
use crate::resolve::{enclosing_def, resolve_targets, Placed, Resolution};

/// A zero-width span used for synthetic nodes that have no source location.
const SYNTHETIC_SPAN: Span = Span {
    start_byte: 0,
    end_byte: 0,
    start_row: 0,
    start_col: 0,
    end_row: 0,
    end_col: 0,
};

/// Normalize a path to an absolute, lexically-cleaned form without touching the
/// filesystem (so it works for deleted files too). This keeps watcher-delivered
/// paths and build-walk paths under the same map key regardless of:
///
/// * Relative vs. absolute input (on Windows watchers often send absolute paths
///   even when the index root was opened as relative).
/// * `./` prefixes or internal `foo/../bar` components.
/// * Drive-letter casing differences on Windows (not addressed here — relies on
///   the OS and the `WalkBuilder` emitting consistent casing).
///
/// Full `canonicalize` is intentionally *not* used: it resolves symlinks (wrong
/// — `WalkBuilder` does NOT follow symlinks, so we must stay consistent) and fails
/// for deleted files (wrong — `remove_file` is called *after* deletion).
fn normalize_path(path: &Path) -> PathBuf {
    // Make absolute relative to the current directory if needed.
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    // Lexically resolve `.` and `..` components.
    let mut cleaned = PathBuf::new();
    for component in abs.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                cleaned.pop();
            }
            c => cleaned.push(c),
        }
    }
    strip_verbatim(&cleaned)
}

/// Strip a Windows verbatim prefix (`\\?\C:\…` → `C:\…`, `\\?\UNC\srv\sh` →
/// `\\srv\sh`) so canonicalized roots and lexically-normalized watcher paths share
/// one key form. `std::fs::canonicalize` returns verbatim paths on Windows while
/// [`normalize_path`] does not; without this they would never compare equal and a
/// save/delete after the build would miss its indexed entry. A no-op on non-verbatim
/// paths and on every non-Windows platform.
fn strip_verbatim(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

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
/// (incremental update), so it isn't stored here. `meta` is the `(size, mtime)`
/// fingerprint captured when the file was last parsed — it travels into the
/// on-disk cache so a later open can decide, by a cheap `stat`, whether this
/// parse is still current and can be reused without reparsing.
#[derive(Debug, Clone)]
struct FileEntry {
    meta: FileMeta,
    parsed: ParsedFile,
}

/// The same admission policy the build walk applies, distilled to a single-path
/// check so the **incremental** update path stays in lock-step with the **build**
/// path. [`collect_source_files`] uses [`WalkBuilder`], which honors `.gitignore`,
/// the global gitignore, and skips hidden entries; a raw watcher event for a
/// gitignored or hidden file would otherwise sneak into the graph on save while a
/// fresh build skipped it — a silent build/incremental divergence.
///
/// Fidelity: this matches `WalkBuilder` semantics by aggregating **every**
/// `.gitignore` from the workspace root down to the file's parent directory (not
/// just the root one), the user's global gitignore, and the hidden-entry rule.
/// Per-directory matchers are parsed lazily and cached, so a watcher firing many
/// events under the same tree re-reads no `.gitignore`.
#[derive(Debug)]
struct IgnorePolicy {
    global: Gitignore,
    /// One compiled matcher per directory that has a `.gitignore`, keyed by the
    /// directory. `None` means "checked, no usable `.gitignore` there" so a missing
    /// file is cached as a negative and never re-stat'd. Interior mutability lets
    /// the otherwise-`&self` [`IgnorePolicy::is_ignored`] populate the cache.
    dir_matchers: RefCell<FxHashMap<PathBuf, Option<Gitignore>>>,
}

impl Default for IgnorePolicy {
    fn default() -> Self {
        Self {
            global: Gitignore::empty(),
            dir_matchers: RefCell::new(FxHashMap::default()),
        }
    }
}

impl IgnorePolicy {
    fn build() -> Self {
        let (global, _) = Gitignore::global();
        Self {
            global,
            dir_matchers: RefCell::new(FxHashMap::default()),
        }
    }

    /// The compiled `.gitignore` matcher for `dir`, if that directory has one.
    /// Cached (including the negative result) so repeated incremental events under
    /// the same subtree never re-read a `.gitignore` from disk.
    fn matcher_for_dir(&self, dir: &Path) -> Option<Gitignore> {
        if let Some(cached) = self.dir_matchers.borrow().get(dir) {
            return cached.clone();
        }
        let gitignore = dir.join(".gitignore");
        let compiled = if gitignore.is_file() {
            let mut builder = GitignoreBuilder::new(dir);
            // `add` returns `Some(err)` on a malformed/absent file — ignored: a
            // broken `.gitignore` simply contributes nothing.
            let _ = builder.add(&gitignore);
            builder.build().ok()
        } else {
            None
        };
        self.dir_matchers
            .borrow_mut()
            .insert(dir.to_path_buf(), compiled.clone());
        compiled
    }

    /// True when the build walk would have skipped `path`: a hidden path component
    /// (`WalkBuilder`'s default `hidden(true)`), the global gitignore, or any
    /// `.gitignore` from the root down to the file's parent directory — matching
    /// `WalkBuilder`'s nested-gitignore semantics so the incremental and build
    /// paths never diverge.
    fn is_ignored(&self, root: &Path, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(root) else {
            // Outside the root: the build walk never visits it, so treat as ignored.
            return true;
        };
        // Hidden component anywhere in the relative path → skipped by the walk.
        let hidden = relative.components().any(|component| match component {
            Component::Normal(name) => name.to_str().is_some_and(|name| name.starts_with('.')),
            _ => false,
        });
        if hidden {
            return true;
        }
        // Global gitignore first (cheap, already compiled).
        if self
            .global
            .matched_path_or_any_parents(path, false)
            .is_ignore()
        {
            return true;
        }
        // Walk root → … → parent(path), letting a deeper `.gitignore` override a
        // shallower one (last match wins), exactly as `WalkBuilder` aggregates them.
        // Each matcher is queried with the path relative to *its own* directory so
        // anchored patterns (`/build`) resolve against the right base.
        let mut decided: Option<bool> = None;
        let mut dir = root.to_path_buf();
        let mut walk_dirs = vec![dir.clone()];
        for component in relative.components() {
            if let Component::Normal(name) = component {
                dir = dir.join(name);
                // Only directories carry `.gitignore`; stop before the file itself.
                if dir == path {
                    break;
                }
                walk_dirs.push(dir.clone());
            }
        }
        for matcher_dir in &walk_dirs {
            let Some(matcher) = self.matcher_for_dir(matcher_dir) else {
                continue;
            };
            // `is_dir = false`: incremental events are always for files (a directory
            // never enters the graph), and the build walk only admits files.
            match matcher.matched_path_or_any_parents(path, false) {
                Match::Ignore(_) => decided = Some(true),
                Match::Whitelist(_) => decided = Some(false),
                Match::None => {}
            }
        }
        decided.unwrap_or(false)
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
    /// Whether the in-memory file set differs from what's on disk in the cache.
    /// A 100%-cache-hit warm build leaves this `false` so the caller can skip
    /// rewriting an identical (potentially large) cache; any parse, deletion, or
    /// incremental edit sets it `true`. Cleared by [`Index::mark_cache_clean`]
    /// after a successful save. Not persisted — purely a runtime write-skip hint.
    cache_dirty: bool,
}

impl Index {
    /// Index every supported file under `root`, parsing in parallel on a pool
    /// bounded by [`lux_core::concurrency::scan_threads`] so the UI keeps a core.
    pub fn build(root: impl AsRef<Path>) -> Result<Self, IndexError> {
        Self::build_inner(root.as_ref(), None)
    }

    /// Like [`Index::build`], but seeded from the persistent parse cache at
    /// `cache_path`. Every file whose on-disk `(size, mtime)` still matches the
    /// cache is reused **without reparsing**; only new or changed files hit
    /// tree-sitter. On a large, mostly-unchanged workspace this turns a multi-second
    /// cold build into a fast `stat`-and-diff warm build — the whole point of the
    /// cache. A missing, stale, or corrupt cache falls back to a full build (the
    /// load is best-effort and never fails the build).
    ///
    /// Persist the result afterwards with [`Index::save_cache`] so the next open is
    /// fast too.
    pub fn build_cached(
        root: impl AsRef<Path>,
        cache_path: impl AsRef<Path>,
    ) -> Result<Self, IndexError> {
        let root = root.as_ref();
        let prior = cache::load(cache_path.as_ref(), root);
        Self::build_inner(root, prior)
    }

    /// Shared build path. `prior` is the loaded parse cache, if any; files it
    /// covers (by matching fingerprint) are reused, the rest are parsed fresh.
    fn build_inner(root: &Path, prior: Option<PriorCache>) -> Result<Self, IndexError> {
        if !root.is_dir() {
            return Err(IndexError::NotADirectory(root.to_path_buf()));
        }
        // Canonicalize the root so the stored key matches whatever the OS
        // returns from watcher events, then strip any Windows verbatim prefix so
        // build-walk paths (root-joined) and `normalize_path`'d watcher paths share
        // one key form. Falls back to lexical normalization (which is still correct
        // for non-symlink paths and already verbatim-free).
        let root = root
            .canonicalize()
            .map_or_else(|_| normalize_path(root), |c| strip_verbatim(&c));
        let root = root.as_path();
        let paths = collect_source_files(root);
        let (files, cache_dirty) = assemble_files(&paths, prior)?;
        let graph = build_graph(&files);
        Ok(Self {
            ignore: IgnorePolicy::build(),
            root: root.to_path_buf(),
            files,
            graph,
            cache_dirty,
        })
    }

    /// Whether the on-disk cache would differ from the current in-memory state —
    /// i.e. whether [`Index::save_cache`] would actually change anything. `false`
    /// after a fully-cached warm build with no edits, so callers can skip a
    /// redundant whole-cache rewrite (the write-amplification that hurts on giant
    /// workspaces).
    #[must_use]
    pub const fn is_cache_dirty(&self) -> bool {
        self.cache_dirty
    }

    /// Mark the cache clean after the caller has successfully persisted it, so a
    /// later transition (workspace close/switch, app quit) won't re-save an
    /// identical cache until something actually changes again.
    pub const fn mark_cache_clean(&mut self) {
        self.cache_dirty = false;
    }

    /// Write the current per-file parses to the persistent cache at `cache_path`
    /// (atomically). Pair with [`Index::build_cached`] on the next open. Best-effort
    /// at the call site: a write failure leaves the previous cache intact and only
    /// means the next open reparses more.
    pub fn save_cache(&self, cache_path: impl AsRef<Path>) -> std::io::Result<()> {
        cache::save(
            cache_path.as_ref(),
            &self.root,
            self.files
                .iter()
                .map(|(path, entry)| (path.as_path(), entry.meta, &entry.parsed)),
        )
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
        // Normalize before any map lookup so watcher paths (possibly relative,
        // differently-cased, or symlink-resolved) always hit the same key as
        // the build walk used.
        let path = normalize_path(path.as_ref());
        let changed = self.stage_file(&path);
        if changed {
            self.cache_dirty = true;
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
            let path = normalize_path(path.as_ref());
            changed |= self.stage_file(&path);
        }
        if changed {
            self.cache_dirty = true;
            self.graph = build_graph(&self.files);
        }
        changed
    }

    /// Drop a file (e.g. deleted on disk) and rebuild. Returns `true` when the
    /// file was present and removed.
    pub fn remove_file(&mut self, path: impl AsRef<Path>) -> bool {
        // Normalize so the key matches what was indexed during the build walk.
        // For deleted files, `canonicalize` would fail, so we use the cheaper
        // normalize_path (absolute + lexical clean) instead.
        let path = normalize_path(path.as_ref());
        if self.files.remove(&path).is_some() {
            self.cache_dirty = true;
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
        // The build walk (`collect_source_files`) does not follow symlinks — a
        // symlinked entry's file type is `is_symlink()`, not `is_file()`, so it is
        // never admitted. Mirror that here so a watcher event for an in-root symlink
        // can't diverge the incremental graph from a fresh build.
        if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return self.files.remove(path).is_some();
        }
        let entry = Lang::from_path(path).and_then(|lang| {
            // Gate on the size first (cheap stat), then parse the bytes.
            let pre_meta = FileMeta::of(path)?;
            if pre_meta.size > MAX_FILE_BYTES {
                return None;
            }
            let source = std::fs::read_to_string(path).ok()?;
            let parsed = parse_source(lang, &source).ok()?;
            // Re-stat *after* the read to detect concurrent writes: if the
            // fingerprint changed between the read and this stat, the bytes we
            // parsed may already be stale. Retry once to get a stable pair;
            // if the file is still in flux, skip this cycle (the watcher will
            // deliver another event).
            let post_meta = FileMeta::of(path)?;
            if post_meta != pre_meta {
                // One retry with a fresh read.
                let source2 = std::fs::read_to_string(path).ok()?;
                let parsed2 = parse_source(lang, &source2).ok()?;
                let meta2 = FileMeta::of(path)?;
                if meta2 != post_meta {
                    // Still in flux — skip; watcher will fire again.
                    return None;
                }
                return Some(FileEntry {
                    meta: meta2,
                    parsed: parsed2,
                });
            }
            Some(FileEntry {
                meta: post_meta,
                parsed,
            })
        });
        match entry {
            Some(entry) => {
                // Skip the graph rebuild when the parse is byte-for-byte identical
                // to what we already hold. Autosave, format-on-save, and "touch"
                // writes bump the file mtime without changing the symbols/refs the
                // graph is built from; comparing `.parsed` (NOT `.meta`, whose mtime
                // always differs) lets those no-op saves return `false` so the
                // caller's `build_graph` over ALL files is avoided. The fresh
                // fingerprint is still stored so the on-disk cache stays warm.
                if let Some(existing) = self.files.get_mut(path) {
                    if existing.parsed == entry.parsed {
                        existing.meta = entry.meta;
                        return false;
                    }
                    *existing = entry;
                    return true;
                }
                self.files.insert(path.to_path_buf(), entry);
                true
            }
            None => self.files.remove(path).is_some(),
        }
    }
}

/// Walk `root` with standard ignore rules (.gitignore, hidden files, etc.),
/// returning the paths of files in a language the graph understands.
///
/// The walk runs in **parallel** — bounded by [`lux_core::scan_threads`] so the UI
/// keeps a core — because on a giant tree (the "thousand nested projects" case)
/// directory traversal and the per-entry `stat` it implies are themselves a real
/// cost, not just the parsing that follows. Each worker streams its hits down an
/// MPSC channel to avoid lock contention; the resulting order is irrelevant
/// because [`build_graph`] sorts paths for deterministic node ids.
fn collect_source_files(root: &Path) -> Vec<PathBuf> {
    let (tx, rx) = std::sync::mpsc::channel::<PathBuf>();
    // `WalkBuilder` honors .gitignore and skips hidden entries by default, which
    // matches the discovery policy elsewhere in the IDE (no node_modules/target).
    WalkBuilder::new(root)
        .threads(lux_core::scan_threads())
        .build_parallel()
        .run(|| {
            let tx = tx.clone();
            Box::new(move |entry| {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if entry.file_type().is_some_and(|t| t.is_file())
                        && Lang::from_path(path).is_some()
                    {
                        let _ = tx.send(path.to_path_buf());
                    }
                }
                WalkState::Continue
            })
        });
    drop(tx); // close the channel so the drain below terminates
    rx.into_iter().collect()
}

/// Build the file map, reusing cached parses where the on-disk fingerprint still
/// matches and parsing only the new/changed files. Three bounded-parallel passes:
///
/// 1. **stat + decide** (parallel): fingerprint every path and mark whether the
///    prior cache has a matching entry. Pure reads — no mutation of `prior`.
/// 2. **partition** (serial): move reused parses out of `prior` (no clone) into the
///    result; collect the rest as work for pass 3.
/// 3. **parse the diff** (parallel): read + tree-sitter only the files that changed
///    or are new. Too-large/unreadable/unparsable files are skipped, never fatal.
///
/// All parallelism runs on a pool capped at the scan-thread budget so a huge build
/// never saturates every core and stalls the UI.
///
/// Returns the file map plus a `cache_dirty` flag: `true` when the on-disk cache
/// would differ from this build (no prior cache, any file parsed, or any prior
/// entry not reused — a change or deletion), so the caller can skip rewriting an
/// identical cache after a fully-cached warm build.
fn assemble_files(
    paths: &[PathBuf],
    prior: Option<PriorCache>,
) -> Result<(FxHashMap<PathBuf, FileEntry>, bool), IndexError> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(lux_core::scan_threads())
        .build()?;
    let had_prior = prior.is_some();
    let mut prior = prior.unwrap_or_default();
    let prior_len = prior.len();

    // Pass 1 — fingerprint every path and decide reuse against the prior cache.
    // Reuse must also respect MAX_FILE_BYTES so this stays in lock-step with the
    // parse path (Pass 3) and `stage_file`: an over-cap file is never admitted by
    // any route. (Unreachable today — the cache can't contain an over-cap entry —
    // but it closes the gap if the cap is ever lowered without a cache-version bump.)
    let prior_ref = &prior;
    let decisions: Vec<(PathBuf, FileMeta, bool)> = pool.install(|| {
        paths
            .par_iter()
            .filter_map(|path| {
                let meta = FileMeta::of(path)?;
                let reuse = meta.size <= MAX_FILE_BYTES
                    && prior_ref
                        .get(path)
                        .is_some_and(|(cached, _)| *cached == meta);
                Some((path.clone(), meta, reuse))
            })
            .collect()
    });

    // Pass 2 — partition: reuse hits are moved straight out of the cache; the rest
    // become parse work.
    let mut files = FxHashMap::default();
    files.reserve(decisions.len());
    let mut to_parse: Vec<(PathBuf, FileMeta)> = Vec::new();
    for (path, meta, reuse) in decisions {
        if reuse {
            if let Some((_, parsed)) = prior.remove(&path) {
                files.insert(path, FileEntry { meta, parsed });
                continue;
            }
        }
        to_parse.push((path, meta));
    }
    drop(prior); // release the unused tail of the stale cache

    // The set of files reused verbatim from the cache. If fewer than the prior
    // held, some prior entries were changed or deleted → the on-disk cache is stale.
    let reused = files.len();

    // Pass 3 — parse only the changed/new files.
    let parsed: Vec<(PathBuf, FileEntry)> = pool.install(|| {
        to_parse
            .par_iter()
            .filter_map(|(path, meta)| {
                if meta.size > MAX_FILE_BYTES {
                    return None;
                }
                let lang = Lang::from_path(path)?;
                let source = std::fs::read_to_string(path).ok()?;
                let parsed = parse_source(lang, &source).ok()?;
                // Re-stat after the read so the cached fingerprint matches the bytes
                // we parsed, not a pre-read stat a concurrent write could invalidate.
                let meta = FileMeta::of(path).unwrap_or(*meta);
                Some((path.clone(), FileEntry { meta, parsed }))
            })
            .collect()
    });
    files.extend(parsed);

    // Dirty when there was no cache to begin with, anything was (re)parsed, or some
    // prior entry wasn't reused (a change or deletion). Only a 100%-reuse warm build
    // with no deletions leaves the on-disk cache already-correct.
    let cache_dirty = !had_prior || !to_parse.is_empty() || reused != prior_len;
    Ok((files, cache_dirty))
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
///
/// File-scope references (imports, top-level calls, etc.) that have no enclosing
/// definition are attributed to a synthetic per-file module node so that
/// file-level import/dependency edges are not silently dropped from the graph.
/// This is the main path through which `import` edges reach the graph and make
/// [`crate::detect::import_cycles`] meaningful.
fn add_reference_edges(
    graph: &mut CodeGraph,
    parsed: &ParsedFile,
    file_id: FileId,
    placed: &[Placed],
) {
    // Lazily create the synthetic file-module node the first time a file-scope
    // reference needs a source.  We avoid creating it eagerly so files with no
    // file-scope references add no extra node.
    let mut file_module: Option<NodeId> = None;
    let mut get_file_module = |graph: &mut CodeGraph| -> NodeId {
        if let Some(id) = file_module {
            return id;
        }
        // Name the synthetic node after the file stem so graph exports are readable.
        let name_str = graph
            .file_path(file_id)
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("<module>")
            .to_string();
        let name = graph.intern(&name_str);
        let id = graph.add_node(Node {
            name,
            kind: SymbolKind::Module,
            file: file_id,
            span: SYNTHETIC_SPAN,
            name_span: SYNTHETIC_SPAN,
        });
        file_module = Some(id);
        id
    };

    let mut edges = Vec::new();
    for reference in &parsed.refs {
        // A reference is not a node, so nothing to exclude — find the tightest
        // definition whose extent contains it. A file-scope reference (imports,
        // top-level uses, etc.) has no encloser, so it attaches to the synthetic
        // module node instead of being silently discarded.
        let source =
            enclosing_def(placed, reference.span, None).unwrap_or_else(|| get_file_module(graph));

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
        // For Call references, allow self-loops so recursive functions produce a
        // Call edge to themselves.  For other reference kinds keep the original
        // behaviour (exclude the enclosing definition to avoid noise from a def
        // whose extent covers its own name token being counted as a self-ref).
        let self_excl = match reference.kind {
            RefKind::Call => None,
            _ => Some(source),
        };
        let (targets, resolution) = resolve_targets(&same_file, global, self_excl);
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
    fn incremental_respects_nested_gitignore() {
        // A `.gitignore` in a *subdirectory* (not the root) excludes `build/`. A
        // fresh build's WalkBuilder honors it, so the incremental path must too —
        // otherwise saving a generated file would diverge the graph from a rebuild.
        let root = workspace(&[("src/.gitignore", "build/\n"), ("src/a.rs", "fn a() {}\n")]);
        let mut index = Index::build(&root).expect("build");

        let nested_ignored = root.join("src/build/gen.rs");
        std::fs::create_dir_all(nested_ignored.parent().unwrap()).unwrap();
        std::fs::write(&nested_ignored, "fn gen() {}\n").unwrap();
        assert!(
            !index.update_file(&nested_ignored),
            "a file ignored by a nested .gitignore must not enter the index incrementally"
        );
        assert_eq!(index.graph().nodes_by_name("gen").len(), 0);

        // A sibling file NOT under build/ still updates normally.
        std::fs::write(root.join("src/a.rs"), "fn a() { a2(); }\nfn a2() {}\n").unwrap();
        assert!(index.update_file(root.join("src/a.rs")));
        assert_eq!(index.graph().nodes_by_name("a2").len(), 1);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn recursive_call_produces_a_self_loop_edge() {
        // A function that calls itself must produce a Calls self-loop, not be
        // silently dropped (the recursion-handling fix in resolve_targets).
        let root = workspace(&[("r.rs", "fn fact(n: u32) -> u32 { fact(n - 1) }\n")]);
        let index = Index::build(&root).expect("build");
        let graph = index.graph();
        let fact = graph.nodes_by_name("fact")[0];
        let self_call = graph
            .out_neighbors(fact)
            .iter()
            .any(|adj| adj.node == fact && adj.kind == EdgeKind::Calls);
        assert!(self_call, "recursive call must be a Calls self-loop edge");

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

    // ── Persistent cache ──

    const SPAN: crate::parse::Span = crate::parse::Span {
        start_byte: 0,
        end_byte: 0,
        start_row: 0,
        start_col: 0,
        end_row: 0,
        end_col: 0,
    };

    /// A parse that names `symbol` — used to seed a prior cache with content that
    /// deliberately disagrees with what's on disk, so a reuse can be observed.
    fn fabricated(symbol: &str) -> crate::parse::ParsedFile {
        let mut parsed = crate::parse::ParsedFile::default();
        parsed.symbols.push(crate::parse::RawSymbol {
            name: symbol.to_string(),
            kind: crate::parse::SymbolKind::Function,
            span: SPAN,
            name_span: SPAN,
        });
        parsed
    }

    #[test]
    fn cache_hit_reuses_parse_without_reparsing() {
        // A prior-cache entry whose fingerprint matches disk is used verbatim —
        // even when its parse disagrees with the file's real contents. Proves the
        // reuse path skips tree-sitter entirely on a hit (the win for warm opens).
        let root = workspace(&[("a.rs", "fn from_disk() {}\n")]);
        let path = root.join("a.rs");
        let meta = super::FileMeta::of(&path).expect("stat");

        let mut prior = crate::cache::PriorCache::default();
        prior.insert(path, (meta, fabricated("from_cache")));

        let index = super::Index::build_inner(&root, Some(prior)).expect("build");
        assert_eq!(index.graph().nodes_by_name("from_cache").len(), 1);
        assert_eq!(index.graph().nodes_by_name("from_disk").len(), 0);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cache_miss_reparses_from_disk() {
        // A prior entry whose fingerprint does NOT match disk is rejected and the
        // file is reparsed from its real contents.
        let root = workspace(&[("a.rs", "fn from_disk() {}\n")]);
        let path = root.join("a.rs");

        let mut prior = crate::cache::PriorCache::default();
        prior.insert(
            path,
            (
                super::FileMeta {
                    size: 0,
                    mtime_ns: 0,
                },
                fabricated("from_cache"),
            ),
        );

        let index = super::Index::build_inner(&root, Some(prior)).expect("build");
        assert_eq!(index.graph().nodes_by_name("from_disk").len(), 1);
        assert_eq!(index.graph().nodes_by_name("from_cache").len(), 0);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn disk_cache_round_trips_and_tracks_changes() {
        let root = workspace(&[("a.rs", "fn a() {}\n")]);
        // The cache lives under `.lux/` (hidden), so the walk never indexes it.
        let cache = root.join(".lux").join("cache").join("code-graph.bin");

        let cold = Index::build(&root).expect("cold build");
        cold.save_cache(&cache).expect("save cache");
        assert!(cache.exists());

        // Warm rebuild from cache reproduces the same graph.
        let warm = Index::build_cached(&root, &cache).expect("warm build");
        assert_eq!(warm.graph().nodes_by_name("a").len(), 1);
        assert_eq!(warm.graph().node_count(), cold.graph().node_count());

        // Editing the file changes its size → the entry is invalidated and the next
        // warm build reparses it, picking up the new symbol and call edge.
        std::fs::write(root.join("a.rs"), "fn a() { b(); }\nfn b() {}\n").unwrap();
        let warm2 = Index::build_cached(&root, &cache).expect("warm build 2");
        assert_eq!(warm2.graph().nodes_by_name("b").len(), 1);
        assert_eq!(callees(&warm2, "a"), vec!["b"]);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_cache_falls_back_to_full_build() {
        let root = workspace(&[("a.rs", "fn solo() {}\n")]);
        let cache = root.join(".lux").join("cache").join("code-graph.bin");

        // No cache yet → full build with a correct result, which can then persist.
        let index = Index::build_cached(&root, &cache).expect("build");
        assert_eq!(index.graph().nodes_by_name("solo").len(), 1);
        index.save_cache(&cache).expect("save");
        assert!(cache.exists());

        std::fs::remove_dir_all(&root).ok();
    }

    // ── Dirty-gate (skip redundant cache writes) ──

    #[test]
    fn fresh_build_is_dirty_then_clean_warm_build() {
        let root = workspace(&[("a.rs", "fn a() {}\n")]);
        let cache = root.join(".lux").join("cache").join("code-graph.bin");

        let cold = Index::build(&root).expect("cold");
        assert!(cold.is_cache_dirty(), "a fresh build has nothing saved yet");
        cold.save_cache(&cache).expect("save");

        // 100% cache hit, no edits → not dirty, so the caller can skip re-saving.
        let warm = Index::build_cached(&root, &cache).expect("warm");
        assert!(
            !warm.is_cache_dirty(),
            "a fully-reused warm build must be clean"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn changed_or_deleted_file_marks_cache_dirty() {
        let root = workspace(&[("a.rs", "fn a() {}\n"), ("b.rs", "fn b() {}\n")]);
        let cache = root.join(".lux").join("cache").join("code-graph.bin");
        Index::build(&root)
            .expect("cold")
            .save_cache(&cache)
            .expect("save");

        // An edit (size change) invalidates one entry → dirty.
        std::fs::write(root.join("a.rs"), "fn a() { x(); }\nfn x() {}\n").unwrap();
        assert!(Index::build_cached(&root, &cache)
            .expect("warm")
            .is_cache_dirty());

        // Re-save the now-current cache, then delete a file → dirty again.
        let resaved = Index::build_cached(&root, &cache).expect("warm");
        resaved.save_cache(&cache).expect("save");
        std::fs::remove_file(root.join("b.rs")).unwrap();
        assert!(
            Index::build_cached(&root, &cache)
                .expect("warm")
                .is_cache_dirty(),
            "a deletion must mark the cache dirty"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn mark_cache_clean_then_incremental_edit_redirties() {
        let root = workspace(&[("a.rs", "fn a() {}\n")]);
        let mut index = Index::build(&root).expect("build");
        index.mark_cache_clean();
        assert!(!index.is_cache_dirty());

        std::fs::write(root.join("a.rs"), "fn a() { y(); }\nfn y() {}\n").unwrap();
        assert!(index.update_file(root.join("a.rs")));
        assert!(
            index.is_cache_dirty(),
            "an incremental edit must re-dirty the cache"
        );

        std::fs::remove_dir_all(&root).ok();
    }
}
