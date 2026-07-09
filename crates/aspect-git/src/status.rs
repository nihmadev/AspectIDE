use std::path::Path;

use aspect_core::{AppResult, GitFileStatus, GitStatus};

use crate::command::{git_command, run_git_capture};

const STATUS_PATH_OFFSET: usize = 3;

pub fn status(root: impl AsRef<Path>) -> AppResult<GitStatus> {
    let output = run_git_capture(git_command(root.as_ref()).args([
        "status",
        "--porcelain=v1",
        "-z",
        "--branch",
    ]))?;
    Ok(parse_status(&String::from_utf8_lossy(&output)))
}

pub(crate) fn parse_status(raw: &str) -> GitStatus {
    let mut branch = None;
    let mut ahead = 0;
    let mut behind = 0;
    let mut files = Vec::new();

    let mut records = raw.split('\0');
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if let Some(rest) = record.strip_prefix("## ") {
            let (branch_name, tracking) = rest.split_once("...").unwrap_or((rest, ""));
            branch = Some(branch_name.to_string());
            ahead = parse_tracking_count(tracking, "ahead ");
            behind = parse_tracking_count(tracking, "behind ");
            continue;
        }
        if record.len() < STATUS_PATH_OFFSET + 1 {
            continue;
        }
        let index_status = &record[0..1];
        let worktree_status = &record[1..2];
        let path = &record[STATUS_PATH_OFFSET..];
        if is_rename_or_copy(index_status) || is_rename_or_copy(worktree_status) {
            let _ = records.next();
        }
        files.push(GitFileStatus {
            index_status: index_status.to_string(),
            worktree_status: worktree_status.to_string(),
            path: Path::new(path).to_path_buf(),
        });
    }

    GitStatus {
        branch,
        ahead,
        behind,
        files,
    }
}

fn parse_tracking_count(tracking: &str, marker: &str) -> u32 {
    tracking
        .find(marker)
        .and_then(|index| {
            tracking[index + marker.len()..]
                .split(|character: char| !character.is_ascii_digit())
                .next()
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(0)
}

fn is_rename_or_copy(status: &str) -> bool {
    status == "R" || status == "C"
}
