#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use lux_core::{AppError, AppResult, GitDiff, GitDiffFile, GitFileStatus, GitStatus};

const MAX_DIFF_PATCH_CHARS: usize = 120_000;
/// Byte budget for the streamed patch. We stop reading the child's stdout once we
/// have this many bytes so a huge generated/vendored diff cannot be fully allocated
/// before the char cap is applied. It is sized comfortably above
/// `MAX_DIFF_PATCH_CHARS` so multi-byte UTF-8 within the char budget still fits.
const MAX_DIFF_PATCH_BYTES: usize = MAX_DIFF_PATCH_CHARS * 4;
/// Wall-clock ceiling for a single git invocation. Credential/SSH prompts, hooks,
/// or slow remotes would otherwise freeze an AI turn indefinitely; on timeout we
/// kill the child and return a structured error instead of blocking forever.
const GIT_TIMEOUT: Duration = Duration::from_secs(30);
/// The empty-tree object hash (`git hash-object -t tree /dev/null`). Diffing against
/// it yields "everything is an addition", which is exactly what we want in an unborn
/// repository that has no `HEAD` commit yet.
const EMPTY_TREE_HASH: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn status(root: impl AsRef<Path>) -> AppResult<GitStatus> {
    // `-z` makes status NUL-delimit records and emit verbatim (unquoted) paths, so
    // filenames containing spaces, quotes, or newlines parse unambiguously.
    let output = run_git_capture(
        git_command(root.as_ref()).args(["status", "--porcelain=v1", "-z", "--branch"]),
    )?;
    Ok(parse_status(&String::from_utf8_lossy(&output)))
}

pub fn diff(root: impl AsRef<Path>) -> AppResult<GitDiff> {
    let root = root.as_ref();
    // In an unborn repo there is no `HEAD`, so `git diff HEAD` errors out and hides
    // all staged/working changes. Diff against the empty tree instead so a brand-new
    // AI-scaffolded project still shows every file as an addition.
    let base = diff_base(root);

    // `-z` NUL-delimits records and disables path quoting, making rename/odd-name
    // parsing robust (see `parse_diff_files`).
    let stat_output = run_git_capture(
        git_command(root).args(["diff", "--numstat", "-z", "--find-renames", &base, "--"]),
    )?;
    let name_status_output = run_git_capture(
        git_command(root).args(["diff", "--name-status", "-z", "--find-renames", &base, "--"]),
    )?;

    let mut files = parse_diff_files(
        &String::from_utf8_lossy(&stat_output),
        &String::from_utf8_lossy(&name_status_output),
    );

    // Stream the patch and stop at the byte budget so an enormous diff is never
    // fully materialized just to truncate it afterwards.
    let (raw_patch, patch_capped) = run_git_patch_streamed(
        git_command(root).args([
            "diff",
            "--find-renames",
            "--patch",
            "--unified=3",
            &base,
            "--",
        ]),
    )?;

    // Untracked files are invisible to `git diff`; merge them in as additions so AI
    // review and working-tree context see newly created source files.
    merge_untracked_files(root, &mut files);

    let raw_patch_chars = raw_patch.chars().count();
    let truncated = patch_capped || raw_patch_chars > MAX_DIFF_PATCH_CHARS;
    let patch = if raw_patch_chars > MAX_DIFF_PATCH_CHARS {
        let kept: String = raw_patch.chars().take(MAX_DIFF_PATCH_CHARS).collect();
        let suffix = if patch_capped {
            format!("\n...[truncated at {MAX_DIFF_PATCH_CHARS} chars; diff exceeds the streamed limit]")
        } else {
            format!("\n...[truncated {} chars]", raw_patch_chars - MAX_DIFF_PATCH_CHARS)
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

/// The diff base: `HEAD` for a normal repo, or the empty-tree hash when `HEAD` is
/// unborn (no commits yet) so staged/working changes are still reported.
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

/// Add untracked working-tree files (status `??`) to `files` as additions, unless
/// already present. Their line counts come from a follow-up numstat against the
/// empty blob; unreadable/binary files are marked accordingly.
fn merge_untracked_files(root: &Path, files: &mut Vec<GitDiffFile>) {
    let Ok(output) = run_git_capture(
        git_command(root).args(["status", "--porcelain=v1", "-z", "--untracked-files=all"]),
    ) else {
        return;
    };
    let text = String::from_utf8_lossy(&output);
    for record in text.split('\0') {
        // Untracked records are exactly `?? <path>` with no rename second field.
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

/// Largest untracked file we read to count lines. Beyond this we report it as a
/// present addition without a (misleading) huge line count, mirroring how git caps
/// effort on large/binary blobs and keeping a single AI turn from reading a giant
/// generated file into memory.
const MAX_UNTRACKED_SCAN_BYTES: u64 = 5 * 1024 * 1024;

/// Added-line count for an untracked file, read directly from disk (cheaper and more
/// portable than spawning another git process per file). Returns `(additions,
/// binary)`. A NUL byte marks the file binary; an unreadable or oversized file is
/// reported as a binary addition with no line count so it still appears in totals
/// without guessing.
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
        return (0, true); // NUL byte ⇒ treat as binary.
    }
    // Count added lines the way git's numstat does: the number of newline-terminated
    // lines, plus one for a trailing line with no final newline. The file is already
    // capped at `MAX_UNTRACKED_SCAN_BYTES`, so a manual newline count is cheap enough
    // that pulling in a SIMD `bytecount` dependency would not pay for itself.
    #[allow(
        clippy::naive_bytecount,
        reason = "Bounded (<=5 MiB) scan; avoids a new dependency for a one-shot count."
    )]
    let newlines = bytes.iter().filter(|&&byte| byte == b'\n').count();
    let has_unterminated_tail = bytes.last().is_some_and(|&byte| byte != b'\n');
    let additions = u32::try_from(newlines + usize::from(has_unterminated_tail)).unwrap_or(u32::MAX);
    (additions, false)
}

// ── Mutations (stage / unstage / discard / commit / sync / branches) ──
// Every mutation returns the freshly recomputed `GitStatus` so a single IPC call
// both performs the action and gives the UI the new state to render.

/// Run a prepared git invocation, mapping a non-zero exit to a Service error that
/// carries git's own stderr (so the panel shows the real reason — failed hook,
/// nothing staged, rejected push, …). Bounded by [`GIT_TIMEOUT`].
fn run_git(command: &mut Command) -> AppResult<String> {
    let output = run_git_output(command)?;
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

/// Like [`run_git`] but returns raw stdout bytes (no trimming), used for `-z`
/// NUL-delimited output where trailing bytes are significant.
fn run_git_capture(command: &mut Command) -> AppResult<Vec<u8>> {
    let output = run_git_output(command)?;
    if !output.status.success() {
        return Err(AppError::Service(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(output.stdout)
}

/// Spawn a git command and wait for it with a [`GIT_TIMEOUT`] ceiling. On timeout we
/// kill the child and return a structured error so a credential/SSH prompt or a slow
/// remote can never freeze the AI turn loop indefinitely.
fn run_git_output(command: &mut Command) -> AppResult<std::process::Output> {
    command.stdin(Stdio::null());
    let mut child = command.spawn()?;
    let deadline = Instant::now() + GIT_TIMEOUT;
    loop {
        // The child has exited: collect its full output.
        if child.try_wait()?.is_some() {
            return Ok(child.wait_with_output()?);
        }
        // Still running: kill it once the deadline passes so a credential/SSH prompt
        // or a slow remote can never freeze the AI turn loop indefinitely.
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::Service(format!(
                "git command timed out after {GIT_TIMEOUT:?} (a prompt or slow remote may be blocking)"
            )));
        }
        std::thread::sleep(GIT_POLL_INTERVAL);
    }
}

/// How often [`run_git_output`] polls a running child while waiting for the deadline.
const GIT_POLL_INTERVAL: Duration = Duration::from_millis(15);

/// Run a patch-producing git command, streaming stdout and stopping after
/// [`MAX_DIFF_PATCH_BYTES`]. Returns `(patch, capped)` where `capped` is true when we
/// stopped early — so the byte cap is enforced *before* the whole patch is allocated,
/// not after. Bounded by [`GIT_TIMEOUT`].
fn run_git_patch_streamed(command: &mut Command) -> AppResult<(String, bool)> {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn()?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Service("failed to capture git stdout".to_string()))?;

    let mut buffer = Vec::with_capacity(8 * 1024);
    let mut chunk = [0_u8; 16 * 1024];
    let mut capped = false;
    let deadline = Instant::now() + GIT_TIMEOUT;
    loop {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::Service(format!(
                "git diff timed out after {GIT_TIMEOUT:?}"
            )));
        }
        let read = stdout.read(&mut chunk)?;
        if read == 0 {
            break; // EOF
        }
        let remaining = MAX_DIFF_PATCH_BYTES.saturating_sub(buffer.len());
        buffer.extend_from_slice(&chunk[..read.min(remaining)]);
        if buffer.len() >= MAX_DIFF_PATCH_BYTES {
            // Budget hit: stop draining and terminate the child so a giant diff is
            // never fully produced or held in memory.
            capped = true;
            let _ = child.kill();
            break;
        }
    }
    let _ = child.wait();

    // `from_utf8_lossy` keeps the result valid even if the byte cap split a multi-byte
    // sequence; the trailing replacement char is harmless in a display patch.
    Ok((String::from_utf8_lossy(&buffer).into_owned(), capped))
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

/// Byte offset of the path within a porcelain v1 status record: a two-char `XY`
/// code plus one separator space (`XY <path>`).
const STATUS_PATH_OFFSET: usize = 3;

/// Whether a single porcelain status code denotes a rename or copy, whose record
/// is followed by a separate original-path field under `-z`.
fn is_rename_or_copy(status: &str) -> bool {
    status == "R" || status == "C"
}

/// Parse the digit run following `marker` (e.g. `"ahead "`) inside a branch
/// tracking suffix like `[ahead 1, behind 2]`. A missing marker yields `0`.
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

fn parse_status(raw: &str) -> GitStatus {
    let mut branch = None;
    let mut ahead = 0;
    let mut behind = 0;
    let mut files = Vec::new();

    // `-z` makes records NUL-delimited with verbatim (unquoted) paths, so a name
    // containing spaces, quotes, or newlines stays a single intact record. A
    // rename/copy is two records: `XY <new>` immediately followed by a bare
    // `<old>` record we must consume so it is not misread as its own file. We
    // therefore drive the iterator manually instead of `for line in raw.lines()`.
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
        // Verbatim path: never trim — under `-z` a filename may legitimately have
        // leading/trailing spaces, and quoting is disabled.
        let path = &record[STATUS_PATH_OFFSET..];
        if is_rename_or_copy(index_status) || is_rename_or_copy(worktree_status) {
            // Consume (and discard) the original-path field that always trails a
            // rename/copy record in `-z` output.
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

fn parse_diff_files(numstat: &str, name_status: &str) -> Vec<GitDiffFile> {
    let statuses = parse_name_status(name_status);
    let mut files = Vec::new();
    // `--numstat -z` is NUL-delimited; a normal record is `adds\tdels\tpath`, but a
    // rename/copy record ends with an *empty* path field and is followed by two
    // separate NUL records (old path, then new path). Drive the iterator manually so
    // those trailing path records are consumed with their owning entry rather than
    // misread as standalone files. The legacy `{old => new}` brace form never occurs
    // under `-z`, so no path rewriting is needed.
    let mut records = numstat.split('\0').filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        let mut parts = record.split('\t');
        let (Some(additions), Some(deletions)) = (parts.next(), parts.next()) else {
            continue;
        };
        let inline_path = parts.next().unwrap_or("");
        let path = if inline_path.is_empty() {
            // Rename/copy: the next two records are the old and new paths.
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
        // Git reports a binary file's line counts as `-`/`-` rather than numbers.
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

fn parse_name_status(raw: &str) -> std::collections::BTreeMap<String, (String, Option<String>)> {
    let mut statuses = std::collections::BTreeMap::new();
    // `--name-status -z` emits the status code and each path as separate NUL records.
    // A rename/copy (`R<score>`/`C<score>`) is `status\0old\0new`; everything else is
    // `status\0path`. Drive the iterator manually so paired records stay together.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_and_files() {
        // `-z`: records are NUL-delimited (no trailing newlines), paths verbatim.
        let status = parse_status(
            "## main...origin/main [ahead 1, behind 2]\0 M src/main.rs\0A  README.md\0",
        );
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.ahead, 1);
        assert_eq!(status.behind, 2);
        assert_eq!(status.files.len(), 2);
        assert_eq!(status.files[0].path, Path::new("src/main.rs"));
        assert_eq!(status.files[1].path, Path::new("README.md"));
    }

    #[test]
    fn parses_status_rename_and_paths_with_spaces() {
        // A rename is `XY <new>` followed by a bare `<old>` record that must be
        // consumed, and `-z` keeps spaced names intact as a single record.
        let status = parse_status(
            "## master\0R  renamed.txt\0a.txt\0M  weird name.txt\0?? b.txt\0",
        );
        assert_eq!(status.branch.as_deref(), Some("master"));
        assert_eq!(status.ahead, 0);
        assert_eq!(status.behind, 0);
        // The trailing old-path record must NOT become its own file entry.
        assert_eq!(status.files.len(), 3);
        assert_eq!(status.files[0].index_status, "R");
        assert_eq!(status.files[0].path, Path::new("renamed.txt"));
        assert_eq!(status.files[1].path, Path::new("weird name.txt"));
        assert_eq!(status.files[2].path, Path::new("b.txt"));
    }

    #[test]
    fn parses_diff_numstat_and_status() {
        // `-z`: numstat records are NUL-delimited, name-status splits status and path
        // into separate NUL records.
        let files = parse_diff_files(
            "4\t2\tsrc/main.rs\0-\t-\tassets/logo.png\01\t0\tnew.rs\0",
            "M\0src/main.rs\0D\0assets/logo.png\0A\0new.rs\0",
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
        // `-z` rename: the numstat record ends with an empty path, then `old`,`new`
        // follow as separate NUL records; name-status is `R<score>\0old\0new`.
        let files = parse_diff_files(
            "5\t1\t\0src/old.rs\0src/new.rs\0",
            "R100\0src/old.rs\0src/new.rs\0",
        );

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, "R");
        assert_eq!(files[0].old_path.as_deref(), Some(Path::new("src/old.rs")));
        assert_eq!(files[0].path, Path::new("src/new.rs"));
        assert_eq!(files[0].additions, 5);
        assert_eq!(files[0].deletions, 1);
    }
}
