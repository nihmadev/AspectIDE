#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{
    collections::BinaryHeap,
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

use chrono::{DateTime, Utc};
use ignore::{WalkBuilder, WalkState};
use lux_core::{acquire_scan_workers, AppError, AppResult, FsEntry, FsEntryKind, ScanWorkers};

/// Hard ceiling on entries materialized by an unbounded [`read_tree`] crawl.
///
/// `read_tree` deliberately disables ignore handling (it mirrors a raw recursive
/// `read_dir`), so on a monorepo or a tree containing `node_modules`, build output,
/// or a network mount it could otherwise allocate an unbounded vector and block for
/// a long time. The cap turns a pathological workspace into a truncated-but-bounded
/// result instead of an OOM/stall. It is generous enough that ordinary projects are
/// never truncated.
const MAX_TREE_ENTRIES: usize = 200_000;

/// Returns whether `path` is hidden relative to `root`: any path component below
/// `root` that begins with a dot makes the whole entry hidden (e.g. a file inside
/// `.git/` or `.venv/`). Components of `root` itself are never considered — only the
/// portion the walk descended into — so opening a workspace whose own folder starts
/// with a dot does not mark every file hidden.
///
/// Centralizing this here keeps `read_dir`, `read_tree`, and the `list_files*`
/// family consistent; previously name-only checks and full-path checks disagreed for
/// files living under a hidden ancestor.
fn is_hidden_path(path: &Path, root: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative.components().any(|component| {
        matches!(component, Component::Normal(segment)
            if segment.to_string_lossy().starts_with('.'))
    })
}

/// Number of worker threads a walk should use, reserved from the process-global
/// scan budget so concurrent scans/searches share it instead of each spawning the
/// full count. The returned guard must be kept alive for the whole walk.
fn reserve_walk_workers() -> ScanWorkers {
    acquire_scan_workers()
}

/// Upper bound on the heap capacity pre-reserved by [`list_files`] so a caller that
/// passes an enormous `max_results` cannot force a giant up-front allocation; the
/// heap still grows on demand up to `max_results` if that many files actually exist.
const MAX_LIST_HEAP_RESERVE: usize = 4_096;

pub fn read_dir(path: impl AsRef<Path>) -> AppResult<Vec<FsEntry>> {
    let root = path.as_ref();
    let mut entries = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let metadata = entry.metadata()?;
        let name = entry.file_name().to_string_lossy().to_string();
        let entry_path = entry.path();
        let kind = if file_type.is_dir() {
            FsEntryKind::Directory
        } else if file_type.is_file() {
            FsEntryKind::File
        } else if file_type.is_symlink() {
            FsEntryKind::Symlink
        } else {
            FsEntryKind::Other
        };

        entries.push(FsEntry {
            is_hidden: is_hidden_path(&entry_path, root),
            name,
            path: entry_path,
            kind,
            size: metadata.len(),
            modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
        });
    }

    entries.sort_by(|left, right| match (left.kind, right.kind) {
        (FsEntryKind::Directory, FsEntryKind::File) => std::cmp::Ordering::Less,
        (FsEntryKind::File, FsEntryKind::Directory) => std::cmp::Ordering::Greater,
        _ => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
    });

    Ok(entries)
}

/// Result of a bounded tree crawl.
///
/// Carries the collected entries plus whether the walk hit its entry cap
/// (`truncated`) and how many entries were skipped due to walker or metadata errors
/// (`skipped_errors`) instead of being silently dropped.
#[derive(Debug, Default)]
pub struct TreeScan {
    pub entries: Vec<FsEntry>,
    pub truncated: bool,
    pub skipped_errors: usize,
}

pub fn read_tree(root: impl AsRef<Path>) -> AppResult<Vec<FsEntry>> {
    // The legacy entry point keeps its `Vec<FsEntry>` contract for existing callers;
    // it is now backed by the bounded crawl so a pathological workspace can no longer
    // allocate without limit. Truncation/error metadata is available via
    // [`read_tree_bounded`] for callers that want to surface it.
    Ok(read_tree_bounded(root, MAX_TREE_ENTRIES).entries)
}

/// Parallel full-tree crawl (no ignore/hidden filtering — mirrors a raw recursive
/// `read_dir` and includes directories themselves), bounded to `max_entries`.
///
/// Per-thread visitors push into a shared buffer; metadata `stat`s run across worker
/// threads, the dominant cost on large or networked trees. The walk stops early via
/// [`WalkState::Quit`] once `max_entries` is reached so an unbounded monorepo or a
/// tree full of `node_modules`/build output cannot stall the IDE or exhaust memory.
/// Walker/metadata errors are counted rather than dropped, so callers can tell the
/// difference between "no such files" and "files we could not read".
#[must_use]
pub fn read_tree_bounded(root: impl AsRef<Path>, max_entries: usize) -> TreeScan {
    let root = root.as_ref().to_path_buf();
    if max_entries == 0 {
        return TreeScan::default();
    }

    let workers = reserve_walk_workers();
    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .git_exclude(false)
        .git_global(false)
        .parents(false)
        .threads(workers.count());

    let collected: Arc<Mutex<Vec<FsEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let errors = Arc::new(AtomicUsize::new(0));
    builder.build_parallel().run(|| {
        let collected = Arc::clone(&collected);
        let errors = Arc::clone(&errors);
        let root = root.clone();
        Box::new(move |result| {
            // A directory we could not descend / a vanished path: record it as a
            // skipped error instead of silently omitting it.
            let Ok(entry) = result else {
                errors.fetch_add(1, Ordering::Relaxed);
                return WalkState::Continue;
            };
            // The walker yields the root itself first; skip it to match the
            // previous behavior, which only recorded descendants.
            if entry.path() == root {
                return WalkState::Continue;
            }
            match entry_to_fs_entry(&entry, &root) {
                Some(record) => {
                    if let Ok(mut buffer) = collected.lock() {
                        buffer.push(record);
                        // Cap reached: tell every worker to stop walking so we never
                        // materialize the rest of the tree.
                        if buffer.len() >= max_entries {
                            return WalkState::Quit;
                        }
                    }
                }
                None => {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            }
            WalkState::Continue
        })
    });
    drop(workers);

    // `run` has returned, so every worker thread is joined and no other `Arc`
    // clone survives — reclaim the buffer without cloning.
    let mut entries = Arc::try_unwrap(collected)
        .ok()
        .and_then(|mutex| mutex.into_inner().ok())
        .unwrap_or_default();
    let truncated = entries.len() >= max_entries;
    entries.truncate(max_entries);
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    TreeScan {
        entries,
        truncated,
        skipped_errors: errors.load(Ordering::Relaxed),
    }
}

/// Builds an [`FsEntry`] from a walker entry, classifying its kind and reading
/// metadata. `root` anchors ancestor-aware hidden detection. Returns `None` when
/// the entry cannot be stat-ed or named.
fn entry_to_fs_entry(entry: &ignore::DirEntry, root: &Path) -> Option<FsEntry> {
    let file_type = entry.file_type()?;
    let path = entry.path();
    let metadata = entry.metadata().ok();
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())?;
    let kind = if file_type.is_dir() {
        FsEntryKind::Directory
    } else if file_type.is_file() {
        FsEntryKind::File
    } else if file_type.is_symlink() {
        FsEntryKind::Symlink
    } else {
        FsEntryKind::Other
    };
    Some(FsEntry {
        is_hidden: is_hidden_path(path, root),
        name,
        path: path.to_path_buf(),
        kind,
        size: metadata.as_ref().map_or(0, std::fs::Metadata::len),
        modified_at: metadata
            .and_then(|value| value.modified().ok())
            .map(DateTime::<Utc>::from),
    })
}

/// An [`FsEntry`] ordered solely by path so it can live in a [`BinaryHeap`]. The
/// heap is a max-heap on path, which lets [`list_files`] keep only the
/// lexicographically smallest `max_results` paths and evict the current largest in
/// `O(log n)` without ever holding the whole workspace in memory.
struct PathOrdered(FsEntry);

impl PartialEq for PathOrdered {
    fn eq(&self, other: &Self) -> bool {
        self.0.path == other.0.path
    }
}
impl Eq for PathOrdered {}
impl PartialOrd for PathOrdered {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PathOrdered {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.path.cmp(&other.0.path)
    }
}

/// Push `record` into a bounded max-heap that retains only the smallest
/// `max_results` paths: fill until full, then replace the current maximum whenever a
/// smaller path arrives. Caps memory at `max_results` regardless of workspace size.
fn offer_to_bounded_heap(heap: &mut BinaryHeap<PathOrdered>, record: FsEntry, max_results: usize) {
    if heap.len() < max_results {
        heap.push(PathOrdered(record));
    } else if let Some(largest) = heap.peek() {
        if record.path < largest.0.path {
            heap.pop();
            heap.push(PathOrdered(record));
        }
    }
}

/// Result of a bounded file listing.
///
/// Carries the deterministic lexicographically-smallest `entries` plus whether the
/// workspace held *more* matching files than the cap (`truncated`). Surfacing
/// truncation lets callers that build aggregates (language mix, directory counts,
/// "largest" files) avoid silently presenting a lexicographically-biased sample as
/// if it were the whole project.
#[derive(Debug, Default)]
pub struct FileListing {
    pub entries: Vec<FsEntry>,
    pub truncated: bool,
}

pub fn list_files(root: impl AsRef<Path>, max_results: usize) -> AppResult<Vec<FsEntry>> {
    // The legacy entry point keeps its `Vec<FsEntry>` contract; truncation metadata
    // is available via [`list_files_scanned`] for callers that want to surface it.
    Ok(list_files_scanned(root, max_results).entries)
}

/// Like [`list_files`] but reports whether the listing was truncated by `max_results`.
///
/// Returns the deterministic lexicographically-smallest `max_results` file paths
/// (identical to [`list_files`]) together with a `truncated` flag that is set when the
/// workspace contains strictly more than `max_results` matching files. A shared
/// [`AtomicUsize`] counts every matching file the parallel walk encounters — including
/// the ones the bounded heap evicts — so the flag is exact regardless of which paths
/// survive to the output. Lexicographic ordering of `entries` is preserved exactly.
#[must_use]
pub fn list_files_scanned(root: impl AsRef<Path>, max_results: usize) -> FileListing {
    let root = root.as_ref().to_path_buf();
    if max_results == 0 {
        return FileListing::default();
    }
    let workers = reserve_walk_workers();
    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .threads(workers.count());

    // Parallel walk: each worker thread offers matching files into a shared bounded
    // max-heap, so the many `metadata` syscalls fan out across cores while memory
    // stays capped at `max_results` instead of holding every path in a giant `Vec`
    // before truncating. The result is the deterministic lexicographically-smallest
    // `max_results` paths. A separate atomic counts *every* matching file seen so we
    // can tell whether the heap dropped any (i.e. the listing is truncated).
    let heap: Arc<Mutex<BinaryHeap<PathOrdered>>> = Arc::new(Mutex::new(
        BinaryHeap::with_capacity(max_results.min(MAX_LIST_HEAP_RESERVE)),
    ));
    let seen = Arc::new(AtomicUsize::new(0));
    builder.build_parallel().run(|| {
        let heap = Arc::clone(&heap);
        let seen = Arc::clone(&seen);
        let root = root.clone();
        Box::new(move |result| {
            let Ok(entry) = result else {
                return WalkState::Continue;
            };
            let Some(file_type) = entry.file_type() else {
                return WalkState::Continue;
            };
            if !file_type.is_file() {
                return WalkState::Continue;
            }
            let path = entry.into_path();
            let Ok(metadata) = fs::metadata(&path) else {
                return WalkState::Continue;
            };
            let Some(name) = path
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
            else {
                return WalkState::Continue;
            };
            seen.fetch_add(1, Ordering::Relaxed);
            let record = FsEntry {
                is_hidden: is_hidden_path(&path, &root),
                name,
                path,
                kind: FsEntryKind::File,
                size: metadata.len(),
                modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
            };
            if let Ok(mut buffer) = heap.lock() {
                offer_to_bounded_heap(&mut buffer, record, max_results);
            }
            WalkState::Continue
        })
    });
    drop(workers);

    // `run` has returned, so every worker thread is joined and no other `Arc`
    // clone survives — reclaim the heap without cloning, then emit in path order.
    let heap = Arc::try_unwrap(heap)
        .ok()
        .and_then(|mutex| mutex.into_inner().ok())
        .unwrap_or_default();
    let mut entries: Vec<FsEntry> = heap.into_iter().map(|ordered| ordered.0).collect();
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let truncated = seen.load(Ordering::Relaxed) > max_results;
    FileListing { entries, truncated }
}

/// Like [`list_files`] but applies `predicate` to each file path during the walk,
/// stopping once `max_matches` files have matched.
///
/// Non-matching files are never materialized, so callers that only need a bounded
/// set of matches (e.g. a glob substring filter) do not heap-allocate every path in
/// the workspace first.
///
/// The result is sorted by path. When the workspace contains more than
/// `max_matches` matching files, the returned subset is walk-order dependent (the
/// walk short-circuits early); when it contains fewer, every match is returned.
pub fn list_files_matching(
    root: impl AsRef<Path>,
    predicate: impl Fn(&Path) -> bool + Send + Sync,
    max_matches: usize,
) -> AppResult<Vec<FsEntry>> {
    let root = root.as_ref().to_path_buf();
    if max_matches == 0 {
        return Ok(Vec::new());
    }
    let workers = reserve_walk_workers();
    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .threads(workers.count());

    let collected: Arc<Mutex<Vec<FsEntry>>> = Arc::new(Mutex::new(Vec::new()));
    builder.build_parallel().run(|| {
        let collected = Arc::clone(&collected);
        let predicate = &predicate;
        let root = root.clone();
        Box::new(move |result| {
            let Ok(entry) = result else {
                return WalkState::Continue;
            };
            let Some(file_type) = entry.file_type() else {
                return WalkState::Continue;
            };
            if !file_type.is_file() {
                return WalkState::Continue;
            }
            let path = entry.into_path();
            if !predicate(&path) {
                return WalkState::Continue;
            }
            let Ok(metadata) = fs::metadata(&path) else {
                return WalkState::Continue;
            };
            let Some(name) = path
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
            else {
                return WalkState::Continue;
            };
            let record = FsEntry {
                is_hidden: is_hidden_path(&path, &root),
                name,
                path,
                kind: FsEntryKind::File,
                size: metadata.len(),
                modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
            };
            if let Ok(mut buffer) = collected.lock() {
                buffer.push(record);
                // Enough matches gathered: tell every worker to stop walking so
                // we never enumerate the rest of the tree.
                if buffer.len() >= max_matches {
                    return WalkState::Quit;
                }
            }
            WalkState::Continue
        })
    });
    drop(workers);

    let mut entries = Arc::try_unwrap(collected)
        .ok()
        .and_then(|mutex| mutex.into_inner().ok())
        .unwrap_or_default();
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries.truncate(max_matches);
    Ok(entries)
}

/// A root-bound view over the filesystem mutation APIs.
///
/// The bare `create_file`/`create_dir`/`rename`/`copy_path`/`delete` free functions
/// accept arbitrary paths and run destructive `std::fs` operations directly — fine
/// for a trusted caller that has already validated its paths, but unsafe as the
/// surface for renderer/AI/extension-supplied paths. `WorkspaceFs` is the safe form:
/// every requested path is resolved against `root` and proven to stay inside it
/// (rejecting absolute inputs and `..` escapes) *before* any byte is touched, so an
/// agent-supplied `../../etc/passwd` or `C:\Windows\...` is turned into
/// [`AppError::InvalidPath`] instead of a write outside the workspace.
#[derive(Debug, Clone)]
pub struct WorkspaceFs {
    root: PathBuf,
}

impl WorkspaceFs {
    /// Bind mutations to `root` (the opened workspace directory).
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The workspace root these mutations are confined to.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Create an empty file at a workspace-relative path, materializing any
    /// intermediate directories. Creating `notes/today.md` in an empty workspace
    /// must succeed and produce the `notes/` folder — the bare `create_file` free
    /// function only opens the leaf and would fail when an ancestor is missing. The
    /// parent is an ancestor of the already-confined target, so it is provably inside
    /// the workspace and safe to create.
    pub fn create_file(&self, relative: impl AsRef<Path>) -> AppResult<PathBuf> {
        let path = self.confine(relative.as_ref())?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        create_file(&path)?;
        Ok(path)
    }

    /// Create a directory (and parents) at a workspace-relative path.
    pub fn create_dir(&self, relative: impl AsRef<Path>) -> AppResult<PathBuf> {
        let path = self.confine(relative.as_ref())?;
        create_dir(&path)?;
        Ok(path)
    }

    /// Rename/move within the workspace; both endpoints are confined.
    pub fn rename(
        &self,
        from: impl AsRef<Path>,
        to: impl AsRef<Path>,
    ) -> AppResult<(PathBuf, PathBuf)> {
        let from = self.confine(from.as_ref())?;
        let to = self.confine(to.as_ref())?;
        rename(&from, &to)?;
        Ok((from, to))
    }

    /// Copy within the workspace; both endpoints are confined.
    pub fn copy_path(
        &self,
        from: impl AsRef<Path>,
        to: impl AsRef<Path>,
    ) -> AppResult<(PathBuf, PathBuf)> {
        let from = self.confine(from.as_ref())?;
        let to = self.confine(to.as_ref())?;
        copy_path(&from, &to)?;
        Ok((from, to))
    }

    /// Delete a workspace-relative file or directory tree.
    pub fn delete(&self, relative: impl AsRef<Path>) -> AppResult<PathBuf> {
        let path = self.confine(relative.as_ref())?;
        delete(&path)?;
        Ok(path)
    }

    /// Resolve `candidate` against the workspace root and prove it stays inside.
    fn confine(&self, candidate: &Path) -> AppResult<PathBuf> {
        confine_to_root(&self.root, candidate)
    }
}

/// Join `candidate` onto `root` and prove the result stays inside `root`, returning
/// the absolute path. Rejects absolute inputs (which would make `Path::join` discard
/// `root`) and any `..` traversal that climbs above the workspace. The candidate's
/// own components are normalized lexically before the check so a not-yet-existing
/// target (a file we are about to create) still validates — only `root` must exist
/// to be canonicalized.
fn confine_to_root(root: &Path, candidate: &Path) -> AppResult<PathBuf> {
    if candidate.is_absolute() {
        return Err(AppError::InvalidPath(format!(
            "path must be relative to the workspace: {}",
            candidate.display()
        )));
    }

    let base = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut resolved = base.clone();
    for component in candidate.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => resolved.push(segment),
            Component::ParentDir => {
                // Pop only within the workspace; never climb above `root`.
                if !resolved.pop() || !resolved.starts_with(&base) {
                    return Err(AppError::InvalidPath(format!(
                        "path escapes the workspace: {}",
                        candidate.display()
                    )));
                }
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(AppError::InvalidPath(format!(
                    "path must be relative to the workspace: {}",
                    candidate.display()
                )));
            }
        }
    }
    if !resolved.starts_with(&base) {
        return Err(AppError::InvalidPath(format!(
            "path escapes the workspace: {}",
            candidate.display()
        )));
    }
    Ok(resolved)
}

pub fn create_file(path: impl AsRef<Path>) -> AppResult<()> {
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    Ok(())
}

pub fn create_dir(path: impl AsRef<Path>) -> AppResult<()> {
    fs::create_dir_all(path)?;
    Ok(())
}

pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> AppResult<()> {
    fs::rename(from, to)?;
    Ok(())
}

pub fn copy_path(from: impl AsRef<Path>, to: impl AsRef<Path>) -> AppResult<()> {
    let from = from.as_ref();
    let to = to.as_ref();
    if to.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("destination already exists: {}", to.display()),
        )
        .into());
    }

    if from.is_dir() {
        // Reject copying a directory into itself. The lexical `starts_with`
        // catches the common direct-nesting case; we additionally compare
        // canonicalized paths so a destination reached via `..` or a symlink
        // into `from` cannot slip past and drive `copy_dir_recursive` into
        // unbounded recursion. `to` must not exist yet, so we canonicalize its
        // parent and re-append the final component; any canonicalize failure
        // falls back to the raw path (preserving the cheap lexical guard).
        let from_real = fs::canonicalize(from).unwrap_or_else(|_| from.to_path_buf());
        let to_real = to
            .parent()
            .and_then(|parent| fs::canonicalize(parent).ok())
            .map_or_else(
                || to.to_path_buf(),
                |parent| parent.join(to.file_name().unwrap_or_default()),
            );
        if to.starts_with(from) || to_real.starts_with(&from_real) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "cannot copy a directory into itself",
            )
            .into());
        }
        copy_dir_recursive(from, to)?;
    } else {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(from, to)?;
    }

    Ok(())
}

pub fn delete(path: impl AsRef<Path>) -> AppResult<()> {
    let path = path.as_ref();
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn reveal_in_file_explorer(path: impl AsRef<Path>) -> AppResult<()> {
    let path = path.as_ref();

    #[cfg(target_os = "windows")]
    {
        let argument = format!("/select,{}", path.display());
        let mut command = Command::new("explorer.exe");
        command
            .arg(argument)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        // The launcher exits almost immediately; a dropped `Child` is never
        // waited on, so on Unix it lingers as a zombie until the IDE exits.
        // Reap it on a detached thread so each reveal call cleans up after
        // itself.
        let mut child = Command::new("open").arg("-R").arg(path).spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let target = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
        };
        // Reap the short-lived launcher on a detached thread; otherwise the
        // dropped `Child` is never waited on and lingers as a zombie until the
        // IDE exits, accumulating across reveal calls.
        let mut child = Command::new("xdg-open").arg(target).spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

fn copy_dir_recursive(from: &Path, to: &Path) -> AppResult<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        // Classify with the no-follow `DirEntry::file_type` so a symlink is
        // detected *as a symlink* rather than resolved to its target. Following
        // links here is unsafe during recursion: a link pointing at `from`
        // itself or an ancestor would drive this function without bound
        // (stack-overflow abort / endless nested dirs), and a link to an
        // external tree (e.g. `/` or `/etc`) would be duplicated wholesale into
        // the destination — neither is caught by the one-shot canonicalize guard
        // in `copy_path`. We therefore recreate symlinks verbatim and only
        // recurse into / copy *real* directories and files.
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            copy_symlink(&source, &target)?;
        } else if file_type.is_dir() {
            copy_dir_recursive(&source, &target)?;
        } else if file_type.is_file() {
            fs::copy(source, target)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(source: &Path, target: &Path) -> AppResult<()> {
    let link_target = fs::read_link(source)?;
    std::os::unix::fs::symlink(link_target, target)?;
    Ok(())
}

#[cfg(windows)]
fn copy_symlink(source: &Path, target: &Path) -> AppResult<()> {
    let link_target = fs::read_link(source)?;
    // Windows needs the matching constructor for directory vs file links.
    // Resolve the link from its real location to decide; a dangling link (whose
    // target cannot be stat-ed) falls back to a file symlink.
    if fs::metadata(source).is_ok_and(|meta| meta.is_dir()) {
        std::os::windows::fs::symlink_dir(link_target, target)?;
    } else {
        std::os::windows::fs::symlink_file(link_target, target)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use lux_core::FsEntryKind;

    use super::{list_files, list_files_scanned, read_tree, read_tree_bounded, WorkspaceFs};

    fn test_root(tag: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("lux-fs-{tag}-{}-{suffix}", std::process::id()))
    }

    fn build_fixture(root: &Path) {
        std::fs::create_dir_all(root.join("src/inner")).expect("dirs");
        std::fs::create_dir_all(root.join("target")).expect("target dir");
        // `ignore` only honors .gitignore inside a git repo (require_git default).
        // A bare `.git` dir is enough for repo detection — this mirrors how the
        // setting behaves on real workspaces and exercises the gitignore path.
        std::fs::create_dir_all(root.join(".git")).expect("git dir");
        std::fs::write(root.join(".gitignore"), "target\n").expect("gitignore");
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("main");
        std::fs::write(root.join("src/inner/util.rs"), "pub fn util() {}\n").expect("util");
        std::fs::write(root.join("readme.md"), "# readme\n").expect("readme");
        std::fs::write(root.join("target/generated.rs"), "// generated\n").expect("generated");
    }

    #[test]
    fn list_files_respects_gitignore_and_is_sorted() {
        let root = test_root("list");
        build_fixture(&root);

        let entries = list_files(&root, 1000).expect("list_files should succeed");
        let paths: Vec<String> = entries
            .iter()
            .map(|entry| {
                entry
                    .path
                    .strip_prefix(&root)
                    .unwrap_or(&entry.path)
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();

        // gitignored `target/` is excluded; all entries are files.
        assert!(paths.contains(&"src/main.rs".to_string()));
        assert!(paths.contains(&"src/inner/util.rs".to_string()));
        assert!(paths.contains(&"readme.md".to_string()));
        assert!(
            !paths.iter().any(|path| path.contains("generated.rs")),
            "gitignored file leaked: {paths:?}"
        );
        assert!(entries.iter().all(|entry| entry.kind == FsEntryKind::File));

        // Deterministic: sorted by path, and stable across repeated runs.
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted, "results must be path-sorted");
        let again = list_files(&root, 1000).expect("second list_files");
        let again_paths: Vec<_> = again.iter().map(|entry| entry.path.clone()).collect();
        let first_paths: Vec<_> = entries.iter().map(|entry| entry.path.clone()).collect();
        assert_eq!(
            first_paths, again_paths,
            "parallel walk must be deterministic"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_files_honors_max_results() {
        let root = test_root("max");
        build_fixture(&root);
        let limited = list_files(&root, 2).expect("list_files should succeed");
        assert_eq!(limited.len(), 2, "max_results must cap output");
        // With sort-before-truncate the cap takes the lexicographically first paths.
        let mut all = list_files(&root, 1000).expect("full list");
        all.truncate(2);
        assert_eq!(
            limited.iter().map(|entry| &entry.path).collect::<Vec<_>>(),
            all.iter().map(|entry| &entry.path).collect::<Vec<_>>(),
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_files_scanned_reports_truncation() {
        let root = test_root("scanned");
        build_fixture(&root);
        // The fixture has more than two non-ignored files; a cap of 2 must yield
        // exactly two entries and flag the listing as truncated.
        let capped = list_files_scanned(&root, 2);
        assert_eq!(capped.entries.len(), 2, "cap must hold the entry count");
        assert!(
            capped.truncated,
            "more matching files than the cap must set truncated"
        );
        // A generous cap returns everything and is not flagged truncated.
        let full = list_files_scanned(&root, 100_000);
        assert!(!full.truncated, "a generous cap must not be truncated");
        assert!(full.entries.len() > 2);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn read_tree_includes_dirs_and_ignored_paths() {
        let root = test_root("tree");
        build_fixture(&root);
        let entries = read_tree(&root).expect("read_tree should succeed");
        let rels: Vec<String> = entries
            .iter()
            .map(|entry| {
                entry
                    .path
                    .strip_prefix(&root)
                    .unwrap_or(&entry.path)
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();

        // read_tree mirrors a raw recursive read_dir: directories are present and
        // gitignored paths are NOT filtered out.
        assert!(rels.contains(&"src".to_string()));
        assert!(rels.contains(&"target".to_string()));
        assert!(
            rels.iter().any(|path| path.contains("generated.rs")),
            "read_tree should include ignored files"
        );
        assert!(entries
            .iter()
            .any(|entry| entry.kind == FsEntryKind::Directory));
        // Root itself is not included.
        assert!(!rels.iter().any(std::string::String::is_empty));
        // Sorted by path.
        let mut sorted = rels.clone();
        sorted.sort();
        assert_eq!(rels, sorted);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn read_tree_bounded_caps_entries_and_reports_truncation() {
        let root = test_root("tree-bounded");
        build_fixture(&root);
        // The fixture has more than two entries; a cap of 2 must truncate and flag it.
        let scan = read_tree_bounded(&root, 2);
        assert_eq!(scan.entries.len(), 2, "entry cap must hold");
        assert!(scan.truncated, "hitting the cap must set truncated");
        // A generous cap returns everything and is not flagged truncated.
        let full = read_tree_bounded(&root, 100_000);
        assert!(!full.truncated);
        assert!(full.entries.len() > 2);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_files_bounded_heap_keeps_lexicographically_smallest() {
        let root = test_root("list-bounded");
        build_fixture(&root);
        // With a cap of 2 the bounded heap must return exactly the two smallest
        // (non-ignored) paths, identical to taking the first 2 of the full sort.
        let limited = list_files(&root, 2).expect("list_files should succeed");
        let mut full = list_files(&root, 100_000).expect("full list");
        full.truncate(2);
        assert_eq!(
            limited.iter().map(|entry| &entry.path).collect::<Vec<_>>(),
            full.iter().map(|entry| &entry.path).collect::<Vec<_>>(),
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn files_under_hidden_directory_are_marked_hidden_everywhere() {
        let root = test_root("hidden");
        std::fs::create_dir_all(root.join(".config/nested")).expect("hidden dir");
        std::fs::write(root.join(".config/nested/app.toml"), "x = 1\n").expect("hidden file");
        std::fs::write(root.join("visible.txt"), "hi\n").expect("visible file");

        // read_tree must flag the file under `.config/` as hidden via its ancestor,
        // not just dotfiles by name.
        let tree = read_tree(&root).expect("read_tree should succeed");
        let hidden_under_dir = tree
            .iter()
            .find(|entry| entry.path.ends_with("app.toml"))
            .expect("nested file present");
        assert!(
            hidden_under_dir.is_hidden,
            "file under a hidden ancestor must be hidden in read_tree"
        );
        let visible = tree
            .iter()
            .find(|entry| entry.path.ends_with("visible.txt"))
            .expect("visible file present");
        assert!(!visible.is_hidden);

        // list_files must agree with read_tree on the same file.
        let listed = list_files(&root, 1000).expect("list_files should succeed");
        let listed_hidden = listed
            .iter()
            .find(|entry| entry.path.ends_with("app.toml"))
            .expect("nested file present in list_files");
        assert!(
            listed_hidden.is_hidden,
            "list_files and read_tree must agree on hidden status"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn workspace_fs_confines_mutations_to_the_root() {
        let root = test_root("confine");
        std::fs::create_dir_all(&root).expect("root dir");
        let workspace = WorkspaceFs::new(&root);

        // A normal relative path is created inside the workspace.
        let created = workspace
            .create_file("notes/today.md")
            .expect("relative create should succeed");
        assert!(created.starts_with(root.canonicalize().unwrap_or_else(|_| root.clone())));
        assert!(created.exists());

        // `..` traversal that escapes the workspace is rejected before touching disk.
        let escape = workspace.create_file("../escape.md");
        assert!(escape.is_err(), "parent-dir escape must be rejected");

        // An absolute path is rejected (it would otherwise discard the root on join).
        let absolute_target = std::env::temp_dir().join("lux-fs-should-not-exist.md");
        let absolute = workspace.create_file(&absolute_target);
        assert!(absolute.is_err(), "absolute path must be rejected");
        assert!(!absolute_target.exists());

        std::fs::remove_dir_all(&root).ok();
    }
}
