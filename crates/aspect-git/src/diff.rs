use std::{
    collections::BTreeMap,
    path::{Component, Path},
};

use aspect_core::{AppError, AppResult, GitDiff, GitDiffFile};

use crate::command::{
    git_command, run_git_capture, run_git_patch_streamed, EMPTY_TREE_HASH, MAX_DIFF_PATCH_CHARS,
};

const MAX_UNTRACKED_SCAN_BYTES: u64 = 5 * 1024 * 1024;

pub fn diff(root: impl AsRef<Path>) -> AppResult<GitDiff> {
    let root = root.as_ref();
    let base = diff_base(root);

    let stat_output = run_git_capture(git_command(root).args([
        "diff",
        "--numstat",
        "-z",
        "--find-renames",
        &base,
        "--",
    ]))?;
    let name_status_output = run_git_capture(git_command(root).args([
        "diff",
        "--name-status",
        "-z",
        "--find-renames",
        &base,
        "--",
    ]))?;

    let mut files = parse_diff_files(
        &String::from_utf8_lossy(&stat_output),
        &String::from_utf8_lossy(&name_status_output),
    );

    let (raw_patch, patch_capped) = run_git_patch_streamed(git_command(root).args([
        "diff",
        "--find-renames",
        "--patch",
        "--unified=3",
        &base,
        "--",
    ]))?;

    merge_untracked_files(root, &mut files);

    let raw_patch_chars = raw_patch.chars().count();
    let truncated = patch_capped || raw_patch_chars > MAX_DIFF_PATCH_CHARS;
    let patch = if raw_patch_chars > MAX_DIFF_PATCH_CHARS {
        let kept: String = raw_patch.chars().take(MAX_DIFF_PATCH_CHARS).collect();
        let suffix = if patch_capped {
            format!(
                "\n...[truncated at {MAX_DIFF_PATCH_CHARS} chars; diff exceeds the streamed limit]"
            )
        } else {
            format!(
                "\n...[truncated {} chars]",
                raw_patch_chars - MAX_DIFF_PATCH_CHARS
            )
        };
        format!("{kept}{suffix}")
    } else if patch_capped {
        format!("{raw_patch}\n...[truncated at the streamed diff limit]")
    } else {
        raw_patch
    };

    Ok(GitDiff {
        additions: files.iter().map(|file| file.additions).sum(),
        deletions: files.iter().map(|file| file.deletions).sum(),
        files,
        patch,
        truncated,
    })
}

pub fn file_diff(root: impl AsRef<Path>, path: &str) -> AppResult<(String, String)> {
    let root = root.as_ref();
    let absolute = confine_to_root(root, path)?;
    let rel = relative_spec(root, &absolute);
    let head = git_command(root)
        .args(["show", &format!("HEAD:{rel}")])
        .output()?;
    let head_text = if head.status.success() {
        String::from_utf8_lossy(&head.stdout).to_string()
    } else {
        String::new()
    };
    let working_text = std::fs::read_to_string(&absolute).unwrap_or_default();
    Ok((head_text, working_text))
}

fn diff_base(root: &Path) -> String {
    let has_head = git_command(root)
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .output()
        .is_ok_and(|output| output.status.success());
    if has_head {
        "HEAD".to_string()
    } else {
        EMPTY_TREE_HASH.to_string()
    }
}

fn merge_untracked_files(root: &Path, files: &mut Vec<GitDiffFile>) {
    let Ok(output) = run_git_capture(git_command(root).args([
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
    ])) else {
        return;
    };
    let text = String::from_utf8_lossy(&output);
    for record in text.split('\0') {
        let Some(path) = record.strip_prefix("?? ") else {
            continue;
        };
        let path = path.trim();
        if path.is_empty() || files.iter().any(|file| file.path == Path::new(path)) {
            continue;
        }
        let (additions, binary) = untracked_line_stats(root, path);
        files.push(GitDiffFile {
            path: Path::new(path).to_path_buf(),
            old_path: None,
            status: "A".to_string(),
            additions,
            deletions: 0,
            binary,
        });
    }
}

fn untracked_line_stats(root: &Path, path: &str) -> (u32, bool) {
    let absolute = root.join(path);
    let Ok(metadata) = std::fs::metadata(&absolute) else {
        return (0, true);
    };
    if !metadata.is_file() || metadata.len() > MAX_UNTRACKED_SCAN_BYTES {
        return (0, true);
    }
    let Ok(bytes) = std::fs::read(&absolute) else {
        return (0, true);
    };
    if bytes.contains(&0) {
        return (0, true);
    }
    #[allow(
        clippy::naive_bytecount,
        reason = "Bounded (<=5 MiB) scan; avoids a new dependency for a one-shot count."
    )]
    let newlines = bytes.iter().filter(|&&byte| byte == b'\n').count();
    let has_unterminated_tail = bytes.last().is_some_and(|&byte| byte != b'\n');
    let additions =
        u32::try_from(newlines + usize::from(has_unterminated_tail)).unwrap_or(u32::MAX);
    (additions, false)
}

pub(crate) fn parse_diff_files(numstat: &str, name_status: &str) -> Vec<GitDiffFile> {
    let statuses = parse_name_status(name_status);
    let mut files = Vec::new();
    let mut records = numstat.split('\0').filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        let mut parts = record.split('\t');
        let (Some(additions), Some(deletions)) = (parts.next(), parts.next()) else {
            continue;
        };
        let inline_path = parts.next().unwrap_or("");
        let path = if inline_path.is_empty() {
            let (Some(_old_path), Some(new_path)) = (records.next(), records.next()) else {
                break;
            };
            new_path.to_string()
        } else {
            inline_path.to_string()
        };
        let (status, old_path) = statuses
            .get(&path)
            .cloned()
            .unwrap_or_else(|| ("M".to_string(), None));
        let binary = additions == "-" || deletions == "-";
        files.push(GitDiffFile {
            path: Path::new(&path).to_path_buf(),
            old_path: old_path.map(|value| Path::new(&value).to_path_buf()),
            status,
            additions: additions.parse().unwrap_or(0),
            deletions: deletions.parse().unwrap_or(0),
            binary,
        });
    }
    files
}

fn parse_name_status(
    raw: &str,
) -> BTreeMap<String, (String, Option<String>)> {
    let mut statuses = BTreeMap::new();
    let mut records = raw.split('\0').filter(|record| !record.is_empty());
    while let Some(status_field) = records.next() {
        let normalized_status = status_field.chars().next().unwrap_or('M').to_string();
        if is_rename_or_copy(&normalized_status) {
            let (Some(old_path), Some(new_path)) = (records.next(), records.next()) else {
                break;
            };
            statuses.insert(
                new_path.to_string(),
                (normalized_status, Some(old_path.to_string())),
            );
        } else if let Some(path) = records.next() {
            statuses.insert(path.to_string(), (normalized_status, None));
        }
    }
    statuses
}

fn is_rename_or_copy(status: &str) -> bool {
    status == "R" || status == "C"
}

fn confine_to_root(root: &Path, path: &str) -> AppResult<std::path::PathBuf> {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return Err(AppError::InvalidPath(format!(
            "path must be relative: {path}"
        )));
    }

    let base = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut resolved = base.clone();
    for component in candidate.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => resolved.push(segment),
            Component::ParentDir => {
                if !resolved.pop() || !resolved.starts_with(&base) {
                    return Err(AppError::InvalidPath(format!(
                        "path escapes workspace: {path}"
                    )));
                }
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(AppError::InvalidPath(format!(
                    "path must be relative: {path}"
                )));
            }
        }
    }
    if !resolved.starts_with(&base) {
        return Err(AppError::InvalidPath(format!(
            "path escapes workspace: {path}"
        )));
    }
    Ok(resolved)
}

fn relative_spec(root: &Path, absolute: &Path) -> String {
    let base = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let rel = absolute.strip_prefix(&base).unwrap_or(absolute);
    rel.to_string_lossy().replace('\\', "/")
}
