use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use aspect_core::{AppError, AppResult};

pub(crate) const MAX_DIFF_PATCH_CHARS: usize = 120_000;
pub(crate) const MAX_DIFF_PATCH_BYTES: usize = MAX_DIFF_PATCH_CHARS * 4;
pub(crate) const GIT_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const EMPTY_TREE_HASH: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
pub(crate) const GIT_POLL_INTERVAL: Duration = Duration::from_millis(15);
#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(crate) fn run_git(command: &mut Command) -> AppResult<String> {
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

pub(crate) fn run_git_capture(command: &mut Command) -> AppResult<Vec<u8>> {
    let output = run_git_output(command)?;
    if !output.status.success() {
        return Err(AppError::Service(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(output.stdout)
}

pub(crate) fn run_git_output(command: &mut Command) -> AppResult<std::process::Output> {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let mut stdout_reader = child.stdout.take().map(spawn_pipe_drain);
    let mut stderr_reader = child.stderr.take().map(spawn_pipe_drain);
    let deadline = Instant::now() + GIT_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(std::process::Output {
                status,
                stdout: stdout_reader.take().map_or_else(Vec::new, join_pipe_drain),
                stderr: stderr_reader.take().map_or_else(Vec::new, join_pipe_drain),
            });
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.take().map(join_pipe_drain);
            let _ = stderr_reader.take().map(join_pipe_drain);
            return Err(AppError::Service(format!(
                "git command timed out after {GIT_TIMEOUT:?} (a prompt or slow remote may be blocking)"
            )));
        }
        std::thread::sleep(GIT_POLL_INTERVAL);
    }
}

pub(crate) fn run_git_patch_streamed(command: &mut Command) -> AppResult<(String, bool)> {
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
            break;
        }
        let remaining = MAX_DIFF_PATCH_BYTES.saturating_sub(buffer.len());
        buffer.extend_from_slice(&chunk[..read.min(remaining)]);
        if buffer.len() >= MAX_DIFF_PATCH_BYTES {
            capped = true;
            let _ = child.kill();
            break;
        }
    }
    let _ = child.wait();
    Ok((String::from_utf8_lossy(&buffer).into_owned(), capped))
}

pub(crate) fn spawn_pipe_drain<R: Read + Send + 'static>(
    mut pipe: R,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = pipe.read_to_end(&mut buffer);
        buffer
    })
}

pub(crate) fn join_pipe_drain(handle: std::thread::JoinHandle<Vec<u8>>) -> Vec<u8> {
    handle.join().unwrap_or_default()
}

pub(crate) fn git_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    hide_process_window(&mut command);
    apply_non_interactive_env(&mut command);
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
            "-c",
            "credential.interactive=false",
            "-c",
            "core.askPass=",
        ]);
    command
}

fn apply_non_interactive_env(command: &mut Command) {
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("GCM_INTERACTIVE", "Never")
        .env_remove("DISPLAY")
        .env(
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=10",
        );
}

#[cfg(windows)]
fn hide_process_window(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
const fn hide_process_window(_command: &mut Command) {}

pub(crate) fn add_pathspecs(command: &mut Command, paths: &[String]) {
    command.arg("--");
    for path in paths {
        command.arg(path);
    }
}
