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
        if to.starts_with(from) {
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
        Command::new("open").arg("-R").arg(path).spawn()?;
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
        Command::new("xdg-open").arg(target).spawn()?;
        Ok(())
    }
}

fn copy_dir_recursive(from: &Path, to: &Path) -> AppResult<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            copy_dir_recursive(&source, &target)?;
        } else if metadata.is_file() {
            fs::copy(source, target)?;
        }
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
