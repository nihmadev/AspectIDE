//! Workspace indexing: walk a root, parse every supported file in parallel
//! (bounded so the UI keeps a core via [`aspect_core::scan_threads`]), and assemble
//! the resolved [`CodeGraph`].
//!
//! The [`Index`] retains each file's [`ParsedFile`] so a single-file change can be
//! re-parsed and the graph rebuilt without touching the rest of the workspace вЂ”
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
/// * Drive-letter casing differences on Windows (not addressed here вЂ” relies on
///   the OS and the `WalkBuilder` emitting consistent casing).
///
/// Full `canonicalize` is intentionally *not* used: it resolves symlinks (wrong
/// вЂ” `WalkBuilder` does NOT follow symlinks, so we must stay consistent) and fails
/// for deleted files (wrong вЂ” `remove_file` is called *after* deletion).
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

/// Strip a Windows verbatim prefix (`\\?\C:\вЂ¦` в†’ `C:\вЂ¦`, `\\?\UNC\srv\sh` в†’
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
/// Per-file read/parse problems are *not* errors вЂ” they are skipped so one bad
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
/// fingerprint captured when the file was last parsed вЂ” it travels into the
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
/// fresh build skipped it вЂ” a silent build/incremental divergence.
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
            // `add` returns `Some(err)` on a malformed/absent file вЂ” ignored: a
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
    /// `.gitignore` from the root down to the file's parent directory вЂ” matching
    /// `WalkBuilder`'s nested-gitignore semantics so the incremental and build
    /// paths never diverge.
    fn is_ignored(&self, root: &Path, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(root) else {
            // Outside the root: the build walk never visits it, so treat as ignored.
            return true;
        };
        // Hidden component anywhere in the relative path в†’ skipped by the walk.
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
        // Walk root в†’ вЂ¦ в†’ parent(path), letting a deeper `.gitignore` override a
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
    /// after a successful save. Not persisted вЂ” purely a runtime write-skip hint.
    cache_dirty: bool,
}

impl Index {
    /// Index every supported file under `root`, parsing in parallel on a pool
    /// bounded by [`aspect_core::concurrency::scan_threads`] so the UI keeps a core.
    pub fn build(root: impl AsRef<Path>) -> Result<Self, IndexError> {
        Self::build_inner(root.as_ref(), None)
    }

    /// Like [`Index::build`], but seeded from the persistent parse cache at
    /// `cache_path`. Every file whose on-disk `(size, mtime)` still matches the
    /// cache is reused **without reparsing**; only new or changed files hit
    /// tree-sitter. On a large, mostly-unchanged workspace this turns a multi-second
    /// cold build into a fast `stat`-and-diff warm build вЂ” the whole point of the
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
        // Normalize the root the SAME way per-file paths are normalized in the
        // incremental path (`update_file`/`remove_file`/`stage_file`): lexical,
        // verbatim-stripped, NOT symlink-resolved. `canonicalize` here would
        // resolve symlinks (e.g. macOS `/var` в†’ `/private/var`) while the
        // incremental path does not вЂ” diverging the stored keys and breaking
        // `is_ignored`'s `strip_prefix(root)` on symlinked roots. Staying lexical
        // keeps the build walk and incremental updates in one key form.
        let root = normalize_path(root);
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

    /// Whether the on-disk cache would differ from the current in-memory state вЂ”
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
        // The build walk (`collect_source_files`) does not follow symlinks вЂ” a
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
                    // Still in flux вЂ” skip; watcher will fire again.
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
/// The walk runs in **parallel** вЂ” bounded by [`aspect_core::scan_threads`] so the UI
/// keeps a core вЂ” because on a giant tree (the "thousand nested projects" case)
/// directory traversal and the per-entry `stat` it implies are themselves a real
/// cost, not just the parsing that follows. Each worker streams its hits down an
/// MPSC channel to avoid lock contention; the resulting order is irrelevant
/// because [`build_graph`] sorts paths for deterministic node ids.
fn collect_source_files(root: &Path) -> Vec<PathBuf> {
    let (tx, rx) = std::sync::mpsc::channel::<PathBuf>();
    // `WalkBuilder` honors .gitignore and skips hidden entries by default, which
    // matches the discovery policy elsewhere in the IDE (no node_modules/target).
    WalkBuilder::new(root)
        .threads(aspect_core::scan_threads())
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
///    prior cache has a matching entry. Pure reads вЂ” no mutation of `prior`.
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
/// entry not reused вЂ” a change or deletion), so the caller can skip rewriting an
/// identical cache after a fully-cached warm build.
fn assemble_files(
    paths: &[PathBuf],
    prior: Option<PriorCache>,
) -> Result<(FxHashMap<PathBuf, FileEntry>, bool), IndexError> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(aspect_core::scan_threads())
        .build()?;
    let had_prior = prior.is_some();
    let mut prior = prior.unwrap_or_default();
    let prior_len = prior.len();

    // Pass 1 вЂ” fingerprint every path and decide reuse against the prior cache.
    // Reuse must also respect MAX_FILE_BYTES so this stays in lock-step with the
    // parse path (Pass 3) and `stage_file`: an over-cap file is never admitted by
    // any route. (Unreachable today вЂ” the cache can't contain an over-cap entry вЂ”
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

    // Pass 2 вЂ” partition: reuse hits are moved straight out of the cache; the rest
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
    // held, some prior entries were changed or deleted в†’ the on-disk cache is stale.
    let reused = files.len();

    // Pass 3 вЂ” parse only the changed/new files.
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

    // Pass 1 вЂ” definitions become nodes. Sort paths so node ids are deterministic
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

    // Pass 2 вЂ” edges. For each file, locate the enclosing definition of every
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
/// it (struct в†’ method, module в†’ function, вЂ¦). Always [`Confidence::Extracted`] вЂ”
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
        // A reference is not a node, so nothing to exclude вЂ” find the tightest
        // definition whose extent contains it. A file-scope reference (imports,
        // top-level uses, etc.) has no encloser, so it attaches to the synthetic
        // module node instead of being silently discarded.
        let source =
            enclosing_def(placed, reference.span, None).unwrap_or_else(|| get_file_module(graph));

        let global = graph.nodes_by_name(&reference.name);
        if global.is_empty() {
            continue; // external / stdlib name вЂ” no node to point at
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

