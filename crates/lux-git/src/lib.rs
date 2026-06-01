#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{path::Path, process::Command};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use lux_core::{AppError, AppResult, GitDiff, GitDiffFile, GitFileStatus, GitStatus};

const MAX_DIFF_PATCH_CHARS: usize = 120_000;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn status(root: impl AsRef<Path>) -> AppResult<GitStatus> {
    let output = git_command()
        .arg("-C")
        .arg(root.as_ref())
        .args(["status", "--porcelain=v1", "--branch"])
        .output()?;

    if !output.status.success() {
        return Err(AppError::Service(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    Ok(parse_status(&String::from_utf8_lossy(&output.stdout)))
}

pub fn diff(root: impl AsRef<Path>) -> AppResult<GitDiff> {
    let root = root.as_ref();
    let stat_output = git_command()
        .arg("-C")
        .arg(root)
        .args(["diff", "--numstat", "--find-renames", "HEAD", "--"])
        .output()?;

    if !stat_output.status.success() {
        return Err(AppError::Service(
            String::from_utf8_lossy(&stat_output.stderr)
                .trim()
                .to_string(),
        ));
    }

    let name_status_output = git_command()
        .arg("-C")
        .arg(root)
        .args(["diff", "--name-status", "--find-renames", "HEAD", "--"])
        .output()?;

    if !name_status_output.status.success() {
        return Err(AppError::Service(
            String::from_utf8_lossy(&name_status_output.stderr)
                .trim()
                .to_string(),
        ));
    }

    let patch_output = git_command()
        .arg("-C")
        .arg(root)
        .args([
            "diff",
            "--find-renames",
            "--patch",
            "--unified=3",
            "HEAD",
            "--",
        ])
        .output()?;

    if !patch_output.status.success() {
        return Err(AppError::Service(
            String::from_utf8_lossy(&patch_output.stderr)
                .trim()
                .to_string(),
        ));
    }

    let files = parse_diff_files(
        &String::from_utf8_lossy(&stat_output.stdout),
        &String::from_utf8_lossy(&name_status_output.stdout),
    );
    let raw_patch = String::from_utf8_lossy(&patch_output.stdout);
    let raw_patch_chars = raw_patch.chars().count();
    let truncated = raw_patch_chars > MAX_DIFF_PATCH_CHARS;
    let patch = if truncated {
        format!(
            "{}\n...[truncated {} chars]",
            raw_patch
                .chars()
                .take(MAX_DIFF_PATCH_CHARS)
                .collect::<String>(),
            raw_patch_chars - MAX_DIFF_PATCH_CHARS
        )
    } else {
        raw_patch.to_string()
    };

    Ok(GitDiff {
        additions: files.iter().map(|file| file.additions).sum(),
        deletions: files.iter().map(|file| file.deletions).sum(),
        files,
        patch,
        truncated,
    })
}

fn git_command() -> Command {
    let mut command = Command::new("git");
    hide_process_window(&mut command);
    command
}

fn hide_process_window(command: &mut Command) {
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn parse_status(raw: &str) -> GitStatus {
    let mut branch = None;
    let mut ahead = 0;
    let mut behind = 0;
    let mut files = Vec::new();

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            let (branch_name, tracking) = rest.split_once("...").unwrap_or((rest, ""));
            branch = Some(branch_name.to_string());
            if let Some(index) = tracking.find("ahead ") {
                ahead = tracking[index + 6..]
                    .split(|character: char| !character.is_ascii_digit())
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
            }
            if let Some(index) = tracking.find("behind ") {
                behind = tracking[index + 7..]
                    .split(|character: char| !character.is_ascii_digit())
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
            }
            continue;
        }

        if line.len() >= 4 {
            files.push(GitFileStatus {
                index_status: line[0..1].to_string(),
                worktree_status: line[1..2].to_string(),
                path: Path::new(line[3..].trim()).to_path_buf(),
            });
        }
    }

    GitStatus {
        branch,
        ahead,
        behind,
        files,
    }
}

fn parse_diff_files(numstat: &str, name_status: &str) -> Vec<GitDiffFile> {
    let statuses = parse_name_status(name_status);
    numstat
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let additions = parts.next()?;
            let deletions = parts.next()?;
            let path = parts.next()?.trim();
            let path = normalize_rename_path(path);
            let (status, old_path) = statuses
                .get(&path)
                .cloned()
                .unwrap_or_else(|| ("M".to_string(), None));
            let binary = additions == "-" || deletions == "-";
            Some(GitDiffFile {
                path: Path::new(&path).to_path_buf(),
                old_path: old_path.map(|value| Path::new(&value).to_path_buf()),
                status,
                additions: additions.parse().unwrap_or(0),
                deletions: deletions.parse().unwrap_or(0),
                binary,
            })
        })
        .collect()
}

fn parse_name_status(raw: &str) -> std::collections::BTreeMap<String, (String, Option<String>)> {
    let mut statuses = std::collections::BTreeMap::new();
    for line in raw.lines() {
        let mut parts = line.split('\t');
        let Some(status) = parts.next() else { continue };
        let normalized_status = status.chars().next().unwrap_or('M').to_string();
        if normalized_status == "R" || normalized_status == "C" {
            let Some(old_path) = parts.next() else {
                continue;
            };
            let Some(new_path) = parts.next() else {
                continue;
            };
            statuses.insert(
                new_path.to_string(),
                (normalized_status, Some(old_path.to_string())),
            );
        } else if let Some(path) = parts.next() {
            statuses.insert(path.to_string(), (normalized_status, None));
        }
    }
    statuses
}

fn normalize_rename_path(path: &str) -> String {
    if let Some(open) = path.find('{') {
        if let Some(close_offset) = path[open + 1..].find('}') {
            let close = open + 1 + close_offset;
            let inside = &path[open + 1..close];
            if let Some((_, new_name)) = inside.split_once(" => ") {
                return format!("{}{}{}", &path[..open], new_name, &path[close + 1..]);
            }
        }
    }
    if let Some((_, new_path)) = path.split_once(" => ") {
        return new_path.to_string();
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_and_files() {
        let status = parse_status(
            "## main...origin/main [ahead 1, behind 2]\n M src/main.rs\nA  README.md\n",
        );
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.ahead, 1);
        assert_eq!(status.behind, 2);
        assert_eq!(status.files.len(), 2);
    }

    #[test]
    fn parses_diff_numstat_and_status() {
        let files = parse_diff_files(
            "4\t2\tsrc/main.rs\n-\t-\tassets/logo.png\n1\t0\tnew.rs\n",
            "M\tsrc/main.rs\nD\tassets/logo.png\nA\tnew.rs\n",
        );

        assert_eq!(files.len(), 3);
        assert_eq!(files[0].path, Path::new("src/main.rs"));
        assert_eq!(files[0].additions, 4);
        assert_eq!(files[0].deletions, 2);
        assert_eq!(files[1].status, "D");
        assert!(files[1].binary);
        assert_eq!(files[2].status, "A");
    }

    #[test]
    fn parses_renamed_diff_file() {
        let files = parse_diff_files(
            "5\t1\tsrc/{old.rs => new.rs}\n",
            "R100\tsrc/old.rs\tsrc/new.rs\n",
        );

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, "R");
        assert_eq!(files[0].old_path.as_deref(), Some(Path::new("src/old.rs")));
        assert_eq!(files[0].path, Path::new("src/new.rs"));
    }
}
