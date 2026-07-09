use std::path::Path;

use aspect_core::{AppResult, GitStatus};

use crate::command::{git_command, run_git};
use crate::status::status;

pub fn branches(root: impl AsRef<Path>) -> AppResult<Vec<String>> {
    let output =
        run_git(git_command(root.as_ref()).args(["branch", "--format=%(refname:short)"]))?;
    Ok(parse_branches(&output))
}

pub fn checkout_branch(root: impl AsRef<Path>, name: &str) -> AppResult<GitStatus> {
    let root = root.as_ref();
    run_git(git_command(root).args(["switch", name]))?;
    status(root)
}

pub fn create_branch(root: impl AsRef<Path>, name: &str) -> AppResult<GitStatus> {
    let root = root.as_ref();
    run_git(git_command(root).args(["switch", "-c", name]))?;
    status(root)
}

pub(crate) fn parse_branches(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "(HEAD detached)")
        .map(ToString::to_string)
        .collect()
}
