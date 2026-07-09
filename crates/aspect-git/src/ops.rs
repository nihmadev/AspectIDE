use std::path::Path;

use aspect_core::{AppResult, GitStatus};

use crate::command::{add_pathspecs, git_command, run_git};
use crate::status::status;

pub fn stage(root: impl AsRef<Path>, paths: &[String]) -> AppResult<GitStatus> {
    let root = root.as_ref();
    if paths.is_empty() {
        run_git(git_command(root).args(["add", "-A"]))?;
    } else {
        let mut command = git_command(root);
        command.arg("add");
        add_pathspecs(&mut command, paths);
        run_git(&mut command)?;
    }
    status(root)
}

pub fn unstage(root: impl AsRef<Path>, paths: &[String]) -> AppResult<GitStatus> {
    let root = root.as_ref();
    if paths.is_empty() {
        run_git(git_command(root).args(["reset", "-q"]))?;
    } else {
        let mut command = git_command(root);
        command.args(["reset", "-q", "HEAD"]);
        add_pathspecs(&mut command, paths);
        run_git(&mut command)?;
    }
    status(root)
}

pub fn discard(root: impl AsRef<Path>, paths: &[String]) -> AppResult<GitStatus> {
    let root = root.as_ref();
    let current = status(root)?;
    for path in paths {
        let entry = current
            .files
            .iter()
            .find(|file| file.path == Path::new(path));
        let untracked = entry.is_some_and(|file| file.worktree_status == "?");
        let staged_new = entry.is_some_and(|file| file.index_status == "A");
        let absolute = root.join(path);
        if untracked {
            remove_path(&absolute);
        } else if staged_new {
            let mut command = git_command(root);
            command.args(["reset", "-q", "HEAD"]);
            add_pathspecs(&mut command, std::slice::from_ref(path));
            run_git(&mut command)?;
            remove_path(&absolute);
        } else {
            let mut command = git_command(root);
            command.args(["checkout", "-q", "HEAD"]);
            add_pathspecs(&mut command, std::slice::from_ref(path));
            run_git(&mut command)?;
        }
    }
    status(root)
}

pub fn commit(root: impl AsRef<Path>, message: &str) -> AppResult<GitStatus> {
    let root = root.as_ref();
    run_git(git_command(root).args(["commit", "-m", message]))?;
    status(root)
}

pub fn push(root: impl AsRef<Path>) -> AppResult<GitStatus> {
    let root = root.as_ref();
    run_git(git_command(root).arg("push"))?;
    status(root)
}

pub fn pull(root: impl AsRef<Path>) -> AppResult<GitStatus> {
    let root = root.as_ref();
    run_git(git_command(root).args(["pull", "--ff-only"]))?;
    status(root)
}

fn remove_path(path: &Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}
