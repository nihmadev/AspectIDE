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
    let output = git_command(root.as_ref())
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
    let stat_output = git_command(root)
        .args(["diff", "--numstat", "--find-renames", "HEAD", "--"])
        .output()?;

    if !stat_output.status.success() {
        return Err(AppError::Service(
            String::from_utf8_lossy(&stat_output.stderr)
                .trim()
                .to_string(),
        ));
    }

    let name_status_output = git_command(root)
        .args(["diff", "--name-status", "--find-renames", "HEAD", "--"])
        .output()?;

    if !name_status_output.status.success() {
        return Err(AppError::Service(
            String::from_utf8_lossy(&name_status_output.stderr)
                .trim()
                .to_string(),
        ));
    }

    let patch_output = git_command(root)
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

// ── Mutations (stage / unstage / discard / commit / sync / branches) ──
// Every mutation returns the freshly recomputed `GitStatus` so a single IPC call
// both performs the action and gives the UI the new state to render.

/// Run a prepared git invocation, mapping a non-zero exit to a Service error that
/// carries git's own stderr (so the panel shows the real reason — failed hook,
/// nothing staged, rejected push, …).
fn run_git(command: &mut Command) -> AppResult<String> {
    let output = command.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let message = if stderr.is_empty() { stdout } else { stderr };
        return Err(AppError::Service(if message.is_empty() {
            "git command failed".to_string()
        } else {
            message
        }));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Append literal pathspecs after a `--` separator so leading dashes or odd names
/// are never parsed as flags.
fn add_pathspecs(command: &mut Command, paths: &[String]) {
    command.arg("--");
    for path in paths {
        command.arg(path);
    }
}

/// Stage the given paths, or everything (`git add -A`) when `paths` is empty.
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

/// Unstage the given paths, or everything (`git reset`) when `paths` is empty.
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

/// Discard worktree (and staged) changes for the given paths.
///
/// Untracked files are deleted; staged-new files are unstaged then deleted;
/// tracked changes are reset to HEAD. Destructive — the UI confirms first.
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

/// Best-effort removal of a worktree path (file or directory).
fn remove_path(path: &Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

/// Commit the staged changes with `message`.
pub fn commit(root: impl AsRef<Path>, message: &str) -> AppResult<GitStatus> {
    let root = root.as_ref();
    run_git(git_command(root).args(["commit", "-m", message]))?;
    status(root)
}

/// Push the current branch to its upstream.
pub fn push(root: impl AsRef<Path>) -> AppResult<GitStatus> {
    let root = root.as_ref();
    run_git(git_command(root).arg("push"))?;
    status(root)
}

/// Fast-forward pull from the upstream (never creates a merge commit silently).
pub fn pull(root: impl AsRef<Path>) -> AppResult<GitStatus> {
    let root = root.as_ref();
    run_git(git_command(root).args(["pull", "--ff-only"]))?;
    status(root)
}

/// Local branch names (current branch is reported separately by `status`).
pub fn branches(root: impl AsRef<Path>) -> AppResult<Vec<String>> {
    let output = run_git(git_command(root.as_ref()).args(["branch", "--format=%(refname:short)"]))?;
    Ok(parse_branches(&output))
}

/// Switch to an existing branch.
pub fn checkout_branch(root: impl AsRef<Path>, name: &str) -> AppResult<GitStatus> {
    let root = root.as_ref();
    run_git(git_command(root).args(["switch", name]))?;
    status(root)
}

/// Create and switch to a new branch.
pub fn create_branch(root: impl AsRef<Path>, name: &str) -> AppResult<GitStatus> {
    let root = root.as_ref();
    run_git(git_command(root).args(["switch", "-c", name]))?;
    status(root)
}

/// HEAD vs working-tree text for one file, for the side-by-side diff view.
/// `head_text` is empty for an untracked/new file (or empty repo); `working_text`
/// is empty for a deleted file.
///
/// `path` is treated as workspace-relative and is confined to `root`: an absolute
/// path or one that escapes via `..` is rejected so the panel can never read a
/// file outside the repository.
pub fn file_diff(root: impl AsRef<Path>, path: &str) -> AppResult<(String, String)> {
    let root = root.as_ref();
    let absolute = confine_to_root(root, path)?;
    // `git show HEAD:<path>` wants a repo-relative, forward-slash spec; build it
    // from the validated path so traversal segments can never reach this arg.
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

/// Join a workspace-relative `path` onto `root` and prove the result stays inside
/// `root`, returning the absolute path. Rejects absolute inputs (which would make
/// `Path::join` discard `root`) and any `..` traversal that escapes the workspace.
///
/// The path's own components are normalized before the check so a not-yet-existing
/// file (e.g. a deleted worktree entry) still validates — only `root` needs to
/// exist on disk to be canonicalized.
fn confine_to_root(root: &Path, path: &str) -> AppResult<std::path::PathBuf> {
    use std::path::Component;

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
                // Pop only within the workspace; never climb above `root`.
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

/// Derive the `root`-relative, forward-slash path for a git pathspec from an
/// already-confined absolute path (falls back to the raw display form if the
/// strip somehow fails — it won't for paths produced by `confine_to_root`).
fn relative_spec(root: &Path, absolute: &Path) -> String {
    let base = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let rel = absolute.strip_prefix(&base).unwrap_or(absolute);
    rel.to_string_lossy().replace('\\', "/")
}

fn parse_branches(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "(HEAD detached)")
        .map(ToString::to_string)
        .collect()
}

/// Builds a `git` invocation scoped to `root` with the process-spawning side
/// channels git opens on Windows turned off.
///
/// The IDE polls `status`/`diff` on every filesystem change, so each extra
/// helper git forks is multiplied into a flood. On Git-for-Windows the worst
/// offenders are the fsmonitor hook (a shell script that shells out to
/// `find.exe`) and background `gc --auto`/maintenance — both fired from
/// read-only commands. Disabling them per-invocation keeps a single `git
/// status` to a single process. `--no-optional-locks` additionally stops
/// `status` from rewriting the index just to refresh it.
fn git_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    hide_process_window(&mut command);
    command
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.quotePath=false",
        ]);
    command
}

#[cfg(windows)]
fn hide_process_window(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
const fn hide_process_window(_command: &mut Command) {}

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
            let index_status = &line[0..1];
            let worktree_status = &line[1..2];
            let remainder = line[3..].trim();
            // Porcelain v1 emits renames/copies as `ORIG -> NEW`; keep the new path.
            let path = if index_status == "R"
                || index_status == "C"
                || worktree_status == "R"
                || worktree_status == "C"
            {
                remainder.rsplit(" -> ").next().unwrap_or(remainder)
            } else {
                remainder
            };
            files.push(GitFileStatus {
                index_status: index_status.to_string(),
                worktree_status: worktree_status.to_string(),
                path: Path::new(path).to_path_buf(),
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
    fn parses_branch_list() {
        let branches = parse_branches("main\nfeature/x\n  release \n(HEAD detached)\n\n");
        assert_eq!(branches, vec!["main", "feature/x", "release"]);
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
