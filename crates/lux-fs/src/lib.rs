#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{
    fs,
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
};

#[cfg(all(unix, not(target_os = "macos")))]
use std::path::PathBuf;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

use chrono::{DateTime, Utc};
use ignore::{WalkBuilder, WalkState};
use lux_core::{scan_threads, AppResult, FsEntry, FsEntryKind};

pub fn read_dir(path: impl AsRef<Path>) -> AppResult<Vec<FsEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let metadata = entry.metadata()?;
        let name = entry.file_name().to_string_lossy().to_string();
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
            is_hidden: name.starts_with('.'),
            name,
            path: entry.path(),
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

pub fn read_tree(root: impl AsRef<Path>) -> AppResult<Vec<FsEntry>> {
    let root = root.as_ref().to_path_buf();

    // Parallel walk of the full tree (no ignore/hidden filtering — `read_tree`
    // mirrors a raw recursive `read_dir` and includes directories themselves).
    // Per-thread visitors push into a shared buffer; metadata `stat`s run across
    // worker threads, which is the dominant cost on large or networked trees.
    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .git_exclude(false)
        .git_global(false)
        .parents(false)
        .threads(scan_threads());

    let collected: Arc<Mutex<Vec<FsEntry>>> = Arc::new(Mutex::new(Vec::new()));
    builder.build_parallel().run(|| {
        let collected = Arc::clone(&collected);
        let root = root.clone();
        Box::new(move |result| {
            let Ok(entry) = result else {
                return WalkState::Continue;
            };
            // The walker yields the root itself first; skip it to match the
            // previous behavior, which only recorded descendants.
            if entry.path() == root {
                return WalkState::Continue;
            }
            if let Some(record) = entry_to_fs_entry(&entry) {
                if let Ok(mut buffer) = collected.lock() {
                    buffer.push(record);
                }
            }
            WalkState::Continue
        })
    });

    // `run` has returned, so every worker thread is joined and no other `Arc`
    // clone survives — reclaim the buffer without cloning.
    let mut entries = Arc::try_unwrap(collected)
        .ok()
        .and_then(|mutex| mutex.into_inner().ok())
        .unwrap_or_default();
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

/// Builds an [`FsEntry`] from a walker entry, classifying its kind and reading
/// metadata. Returns `None` when the entry cannot be stat-ed or named.
fn entry_to_fs_entry(entry: &ignore::DirEntry) -> Option<FsEntry> {
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
        is_hidden: name.starts_with('.'),
        name,
        path: path.to_path_buf(),
        kind,
        size: metadata.as_ref().map_or(0, std::fs::Metadata::len),
        modified_at: metadata
            .and_then(|value| value.modified().ok())
            .map(DateTime::<Utc>::from),
    })
}

pub fn list_files(root: impl AsRef<Path>, max_results: usize) -> AppResult<Vec<FsEntry>> {
    let root = root.as_ref().to_path_buf();
    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .threads(scan_threads());

    // Parallel walk: each worker thread collects matching files into a shared
    // buffer, so the many `metadata` syscalls fan out across cores. We gather all
    // matches first, then sort by path and truncate — this makes the result
    // deterministic (the previous serial walk truncated in nondeterministic walk
    // order before sorting), while still honoring `max_results`.
    let collected: Arc<Mutex<Vec<FsEntry>>> = Arc::new(Mutex::new(Vec::new()));
    builder.build_parallel().run(|| {
        let collected = Arc::clone(&collected);
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
            let record = FsEntry {
                is_hidden: path
                    .components()
                    .any(|component| component.as_os_str().to_string_lossy().starts_with('.')),
                name,
                path,
                kind: FsEntryKind::File,
                size: metadata.len(),
                modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
            };
            if let Ok(mut buffer) = collected.lock() {
                buffer.push(record);
            }
            WalkState::Continue
        })
    });

    // `run` has returned, so every worker thread is joined and no other `Arc`
    // clone survives — reclaim the buffer without cloning.
    let mut entries = Arc::try_unwrap(collected)
        .ok()
        .and_then(|mutex| mutex.into_inner().ok())
        .unwrap_or_default();
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries.truncate(max_results);
    Ok(entries)
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
    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .threads(scan_threads());

    let collected: Arc<Mutex<Vec<FsEntry>>> = Arc::new(Mutex::new(Vec::new()));
    builder.build_parallel().run(|| {
        let collected = Arc::clone(&collected);
        let predicate = &predicate;
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
                is_hidden: path
                    .components()
                    .any(|component| component.as_os_str().to_string_lossy().starts_with('.')),
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

    let mut entries = Arc::try_unwrap(collected)
        .ok()
        .and_then(|mutex| mutex.into_inner().ok())
        .unwrap_or_default();
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries.truncate(max_matches);
    Ok(entries)
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
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."))
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

    use super::{list_files, read_tree};

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
}
