#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{fs, path::Path, process::Command};

#[cfg(all(unix, not(target_os = "macos")))]
use std::path::PathBuf;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

use chrono::{DateTime, Utc};
use ignore::WalkBuilder;
use lux_core::{AppResult, FsEntry, FsEntryKind};

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
    let mut entries = Vec::new();
    let mut stack = vec![root];

    while let Some(path) = stack.pop() {
        let Ok(children) = read_dir(&path) else {
            continue;
        };

        for child in children {
            if child.kind == FsEntryKind::Directory {
                stack.push(child.path.clone());
            }
            entries.push(child);
        }
    }

    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

pub fn list_files(root: impl AsRef<Path>, max_results: usize) -> AppResult<Vec<FsEntry>> {
    let root = root.as_ref();
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true);

    let mut entries = Vec::new();
    for entry in builder.build().filter_map(Result::ok) {
        if entries.len() >= max_results {
            break;
        }

        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }

        let path = entry.into_path();
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        let Some(name) = path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
        else {
            continue;
        };

        entries.push(FsEntry {
            is_hidden: path
                .components()
                .any(|component| component.as_os_str().to_string_lossy().starts_with('.')),
            name,
            path,
            kind: FsEntryKind::File,
            size: metadata.len(),
            modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
        });
    }

    entries.sort_by(|left, right| left.path.cmp(&right.path));
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
