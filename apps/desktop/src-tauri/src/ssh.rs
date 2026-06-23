//! SSH engine glue — the process/runtime layer around [`lux_ssh`].
//!
//! Lux drives the system OpenSSH client (`ssh`/`scp`) so it honors the user's
//! existing `~/.ssh/config`, keys, `ssh-agent`, and `known_hosts` out of the box.
//! Every invocation is built by [`lux_ssh`] with hardened, non-interactive flags
//! (`BatchMode=yes`, no password prompts, fast `ConnectTimeout`, forwardings
//! disabled), which is what makes SSH usable by the agent: commands return
//! structured `{exitCode, stdout, stderr}` instead of hanging on a host-key or
//! password prompt the way a raw `ssh` through the Shell tool would.
//!
//! A "session" is a stored destination plus a sticky logical working directory;
//! Lux runs one short-lived `ssh` process per command, so nothing is left running
//! remotely and no credential is ever held in memory.

use std::{path::PathBuf, process::Stdio, time::Instant};

use lux_ssh::{
    build_scp_args, build_ssh_args, parse_probe_output, parse_ssh_config, probe_command,
    wrap_remote_command, HostKeyPolicy, SshConfigHost, SshOptions, SshSession, SshTarget,
    TransferDirection,
};
use serde::Serialize;
use tauri::State;
use tokio::time::{timeout, Duration};

use crate::{
    ai_tools::{kill_process_tree, truncate_shell_output},
    resolve_workspace_path, resolve_workspace_path_for_write, SharedState,
};

/// Settings key (user scope): when `true`, refuse hosts not already in
/// `known_hosts` (`StrictHostKeyChecking=yes`) instead of trust-on-first-use.
pub const STRICT_HOST_KEY_KEY: &str = "ai.ssh.strictHostKey";
/// Settings key (user scope): `ConnectTimeout` in seconds (1–120).
pub const CONNECT_TIMEOUT_KEY: &str = "ai.ssh.connectTimeoutSecs";

/// Extra wall-clock budget added on top of `ConnectTimeout` for the connect probe
/// (covers auth + the one-line probe command round-trip).
const CONNECT_TIMEOUT_BUFFER_SECS: u64 = 15;
/// Default per-command exec timeout (seconds).
const EXEC_DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Hard ceiling for any exec timeout (seconds).
const EXEC_MAX_TIMEOUT_SECS: u64 = 600;
/// File-transfer timeout (seconds). Large transfers may exceed this.
const TRANSFER_TIMEOUT_SECS: u64 = 600;
/// OpenSSH's own error exit code (could not connect / authenticate / host-key
/// mismatch). Any other code means the remote shell actually ran the command.
const SSH_CONNECTION_ERROR_CODE: i32 = 255;
/// Hard cap on how many bytes Lux reads from a single `ssh`/`scp` stream before
/// it stops and tree-kills the child. A runaway remote command (`yes`,
/// `cat /dev/urandom`) would otherwise buffer unbounded into memory. 8 MiB is
/// generous for legitimate command output while bounding the worst case; the
/// captured text is then truncated again to the display ceiling by
/// [`truncate_shell_output`].
const SSH_MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

// ── Result types (camelCase mirrors consumed by the TS layer) ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConnectResult {
    pub session: SshSession,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshExecResult {
    pub session_id: String,
    pub command: String,
    pub cwd: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u128,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTransferResult {
    pub session_id: String,
    pub direction: TransferDirection,
    pub local_path: String,
    pub remote_path: String,
    pub recursive: bool,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub duration_ms: u128,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshOverview {
    pub available: bool,
    pub version: Option<String>,
    pub sessions: Vec<SshSession>,
    pub config_hosts: Vec<SshConfigHost>,
    pub strict_host_key: bool,
    pub connect_timeout_secs: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshDisconnectResult {
    pub closed: usize,
    pub remaining: usize,
}

// ── Internal process runner ──

struct RawRun {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    duration_ms: u128,
    timed_out: bool,
}

/// Read at most `cap` bytes from `reader`, reporting whether the stream had MORE
/// than `cap` bytes (i.e. output was truncated).
///
/// A stream that ends exactly at `cap` is complete, not truncated — the flag is
/// only set when at least one byte beyond `cap` exists. On truncation the excess
/// is left unread in the pipe (the child blocks on a full pipe, then is killed by
/// the caller).
async fn read_capped<R>(reader: &mut R, cap: usize) -> (Vec<u8>, bool)
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        match reader.read(&mut chunk).await {
            // Clean EOF (`Ok(0)`) or a read error both end capture with whatever we
            // already have — untruncated.
            Ok(0) | Err(_) => return (buf, false),
            Ok(n) => {
                let room = cap - buf.len();
                // Strictly greater than the remaining room means real overflow:
                // keep exactly `cap` bytes and report truncation. Filling the cap
                // exactly (`n == room`) is not yet truncation — loop once more so a
                // following EOF stays untruncated while any extra byte trips it.
                if n > room {
                    buf.extend_from_slice(&chunk[..room]);
                    return (buf, true);
                }
                buf.extend_from_slice(&chunk[..n]);
            }
        }
    }
}

/// Append a clear, byte-accurate truncation marker to capped output.
fn mark_truncated(text: String) -> String {
    format!("{text}\n[output truncated at {SSH_MAX_OUTPUT_BYTES} bytes]")
}

/// Spawn `program` with `args`, drain both pipes concurrently, and enforce a hard
/// timeout with a full process-tree kill — the same battle-tested shape as the
/// `Shell` tool. `stdin` is null so OpenSSH can never block reading a prompt.
async fn run_program(program: &str, args: &[String], timeout_secs: u64) -> Result<RawRun, String> {
    let started = Instant::now();
    let mut process = tokio::process::Command::new(program);
    process.args(args);
    process.stdin(Stdio::null());
    process.stdout(Stdio::piped());
    process.stderr(Stdio::piped());
    process.kill_on_drop(true);
    #[cfg(windows)]
    process.creation_flags(crate::ai_tools::CREATE_NO_WINDOW);
    #[cfg(unix)]
    process.process_group(0);

    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "OpenSSH `{program}` was not found on PATH. Install the OpenSSH client (bundled with Windows 10+, macOS, and most Linux distros) and retry."
            ));
        }
        Err(error) => return Err(format!("Failed to start `{program}`: {error}")),
    };
    let child_pid = child.id();
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();

    // Drain both pipes concurrently (avoids a full-pipe deadlock), each bounded to
    // SSH_MAX_OUTPUT_BYTES so a runaway remote stream can't exhaust memory. If a
    // cap is hit we stop reading and tree-kill the child below so it stops
    // producing, then surface the truncated output with a marker.
    let collect = async {
        let read_stdout = async {
            match stdout_pipe.as_mut() {
                Some(pipe) => Box::pin(read_capped(pipe, SSH_MAX_OUTPUT_BYTES)).await,
                None => (Vec::new(), false),
            }
        };
        let read_stderr = async {
            match stderr_pipe.as_mut() {
                Some(pipe) => Box::pin(read_capped(pipe, SSH_MAX_OUTPUT_BYTES)).await,
                None => (Vec::new(), false),
            }
        };
        let ((stdout_buf, stdout_capped), (stderr_buf, stderr_capped)) =
            tokio::join!(read_stdout, read_stderr);
        let capped = stdout_capped || stderr_capped;
        // Only reap the child if neither stream overflowed; on a cap we skip the
        // potentially-unbounded wait and go straight to the tree-kill below.
        let wait = if capped {
            None
        } else {
            Some(child.wait().await)
        };
        let stdout = String::from_utf8_lossy(&stdout_buf).into_owned();
        let stderr = String::from_utf8_lossy(&stderr_buf).into_owned();
        (wait, stdout, stdout_capped, stderr, stderr_capped, capped)
    };

    match Box::pin(timeout(Duration::from_secs(timeout_secs), collect)).await {
        // A cap was hit: kill the still-running child so it stops producing, then
        // return the truncated output (exit code is unknowable — the process was
        // killed rather than allowed to finish). Each stream gets its own marker.
        Ok((_, stdout, stdout_capped, stderr, stderr_capped, true)) => {
            kill_process_tree(child_pid).await;
            let _ = child.start_kill();
            let stdout = truncate_shell_output(&stdout);
            let stderr = truncate_shell_output(&stderr);
            Ok(RawRun {
                exit_code: None,
                stdout: if stdout_capped {
                    mark_truncated(stdout)
                } else {
                    stdout
                },
                stderr: if stderr_capped {
                    mark_truncated(stderr)
                } else {
                    stderr
                },
                duration_ms: started.elapsed().as_millis(),
                timed_out: false,
            })
        }
        Ok((Some(Ok(status)), stdout, _, stderr, _, false)) => Ok(RawRun {
            exit_code: status.code(),
            stdout: truncate_shell_output(&stdout),
            stderr: truncate_shell_output(&stderr),
            duration_ms: started.elapsed().as_millis(),
            timed_out: false,
        }),
        Ok((Some(Err(error)), ..)) => Err(format!("Failed to run `{program}`: {error}")),
        // `wait` is `None` only when `capped` is true (handled above); this arm is
        // unreachable but keeps the match total without an explicit panic.
        Ok((None, ..)) => Err(format!("`{program}` ended without an exit status")),
        Err(_) => {
            kill_process_tree(child_pid).await;
            let _ = child.start_kill();
            Ok(RawRun {
                exit_code: None,
                stdout: String::new(),
                stderr: format!("{program} timed out after {timeout_secs} seconds"),
                duration_ms: started.elapsed().as_millis(),
                timed_out: true,
            })
        }
    }
}

// ── Settings + environment helpers ──

fn ssh_options_from_settings(state: &State<'_, SharedState>) -> SshOptions {
    let mut opts = SshOptions::default();
    if let Ok(guard) = state.settings.lock() {
        if let Some(store) = guard.as_ref() {
            if let Some(value) = store.get(lux_core::SettingsScope::User, STRICT_HOST_KEY_KEY) {
                if value.value.as_bool() == Some(true) {
                    opts.host_key_policy = HostKeyPolicy::Strict;
                }
            }
            if let Some(value) = store.get(lux_core::SettingsScope::User, CONNECT_TIMEOUT_KEY) {
                if let Some(secs) = value.value.as_u64() {
                    opts.connect_timeout_secs =
                        u16::try_from(secs.clamp(1, 120)).unwrap_or(opts.connect_timeout_secs);
                }
            }
        }
    }
    opts
}

/// The user's home directory, used to locate `~/.ssh/config`.
fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            return Some(PathBuf::from(profile));
        }
        let drive = std::env::var_os("HOMEDRIVE")?;
        let path = std::env::var_os("HOMEPATH")?;
        let mut home = PathBuf::from(drive);
        home.push(path);
        Some(home)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

/// Parse `~/.ssh/config` for discoverable host aliases (best-effort; empty when
/// the file is absent or unreadable).
fn read_config_hosts() -> Vec<SshConfigHost> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    let path = home.join(".ssh").join("config");
    std::fs::read_to_string(path)
        .map(|text| parse_ssh_config(&text))
        .unwrap_or_default()
}

/// Detect the OpenSSH client and its version (`ssh -V` prints to stderr).
async fn ssh_version() -> Option<String> {
    let run = Box::pin(run_program("ssh", &["-V".to_string()], 10))
        .await
        .ok()?;
    let banner = if run.stderr.trim().is_empty() {
        run.stdout.trim()
    } else {
        run.stderr.trim()
    };
    (!banner.is_empty()).then(|| banner.to_string())
}

/// Split a `user@host` destination when no explicit user was supplied, so either
/// form works for the `host` argument.
fn split_user_host(host: &str, user: Option<String>) -> (String, Option<String>) {
    if user.as_ref().is_some_and(|u| !u.trim().is_empty()) {
        return (host.to_string(), user);
    }
    if let Some((parsed_user, parsed_host)) = host.split_once('@') {
        if !parsed_user.is_empty() && !parsed_host.is_empty() {
            return (parsed_host.to_string(), Some(parsed_user.to_string()));
        }
    }
    (host.to_string(), None)
}

fn clean_stderr(stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        "(no error output)".to_string()
    } else {
        trimmed.chars().take(600).collect()
    }
}

// ── Tauri commands ──

/// Open and verify an SSH session, capturing the remote identity + home dir.
#[tauri::command]
pub async fn ssh_connect(
    state: State<'_, SharedState>,
    host: String,
    user: Option<String>,
    port: Option<u16>,
    identity_file: Option<String>,
    label: Option<String>,
) -> Result<SshConnectResult, String> {
    let host = host.trim().to_string();
    if host.is_empty() {
        return Err("SshConnect requires a host (an alias from ~/.ssh/config, a hostname/IP, or user@host).".to_string());
    }
    let (host, user) = split_user_host(&host, user);
    let identity_file = identity_file
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    // Treat a non-positive port as "unset" so the ssh_config/default port is used
    // (mirrors the TS path) — `ssh -p 0` is invalid and would just fail.
    let port = port.filter(|value| *value > 0);
    let target = SshTarget {
        host: host.clone(),
        user: user.clone(),
        port,
        identity_file,
    };
    let opts = ssh_options_from_settings(&state);
    let args = build_ssh_args(&target, opts, Some(probe_command()));
    let run = Box::pin(run_program(
        "ssh",
        &args,
        u64::from(opts.connect_timeout_secs) + CONNECT_TIMEOUT_BUFFER_SECS,
    ))
    .await?;

    if run.timed_out {
        return Err(format!(
            "SSH connection to {} timed out. Check the host is reachable and the port is correct.",
            target.destination()
        ));
    }
    // A non-timeout `None` exit means the ssh process was terminated abnormally
    // (e.g. killed by a signal) — never treat that as a successful login.
    if run.exit_code.is_none() {
        return Err(format!(
            "SSH connection to {} did not complete (the ssh process was terminated before exiting). {}",
            target.destination(),
            clean_stderr(&run.stderr)
        ));
    }
    // 255 is OpenSSH's own failure (connect/auth/host-key). Any other code means
    // the remote shell ran — even a non-POSIX shell where the probe came back
    // empty — so the session is still usable.
    if run.exit_code == Some(SSH_CONNECTION_ERROR_CODE) {
        return Err(format!(
            "SSH connection to {} failed: {}\nLux connects non-interactively (BatchMode), so it cannot answer password or passphrase prompts. Use an SSH key via ssh-agent or an identityFile, and make sure the host key is accepted. For a changed/unknown host key with strict checking on, fix ~/.ssh/known_hosts first.",
            target.destination(),
            clean_stderr(&run.stderr)
        ));
    }

    let probe = parse_probe_output(&run.stdout);
    // Only adopt an absolute path as the starting directory; anything else (empty
    // or a non-POSIX remote) leaves cwd unset so commands use the login default.
    let cwd = probe
        .cwd
        .filter(|dir| dir.starts_with('/'))
        .unwrap_or_default();
    let label = label
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| target.destination());

    let session = state.ssh.insert(
        label,
        target.clone(),
        cwd,
        probe.system.clone(),
        probe.user.clone(),
    );
    let note = format!(
        "Connected to {}{}{}.",
        target.destination(),
        probe
            .system
            .map(|sys| format!(" ({sys})"))
            .unwrap_or_default(),
        if session.cwd.is_empty() {
            String::new()
        } else {
            format!(" at {}", session.cwd)
        }
    );
    Ok(SshConnectResult { session, note })
}

/// Run a command on an existing session. The remote command is wrapped so it runs
/// in the session's sticky working directory; passing `cwd` updates that
/// directory for this and following commands. Catastrophic commands are refused
/// by the same classifier the local `Shell` tool uses.
#[tauri::command]
pub async fn ssh_exec(
    state: State<'_, SharedState>,
    session_id: String,
    command: String,
    cwd: Option<String>,
    timeout_secs: Option<u64>,
) -> Result<SshExecResult, String> {
    let id = uuid::Uuid::parse_str(session_id.trim())
        .map_err(|_| "SshExec requires a valid sessionId from SshConnect.".to_string())?;
    let session = state.ssh.get(id).ok_or_else(|| {
        "No such SSH session. Call SshConnect first (or it was disconnected).".to_string()
    })?;

    let command = command.trim().to_string();
    if command.is_empty() {
        return Err("SshExec requires a non-empty command.".to_string());
    }
    let safety = crate::ai_shell_safety::classify_shell_command(&command);
    if let Some(reason) = safety.blocked {
        return Err(format!(
            "Lux blocked this remote command for safety ({reason}). If it is genuinely intended, run it yourself."
        ));
    }

    // A provided cwd is sticky: persist it to the session before running. Require
    // an absolute remote path (like the connect probe's home dir) so a relative
    // path can't make the session non-deterministic across commands.
    let effective_cwd = match cwd.map(|value| value.trim().to_string()) {
        Some(dir) if !dir.is_empty() => {
            if !dir.starts_with('/') {
                return Err(format!(
                    "SshExec cwd must be an absolute remote path starting with '/' (got: {dir}). Omit cwd to keep the session's current directory."
                ));
            }
            state.ssh.set_cwd(id, dir.clone());
            dir
        }
        _ => session.cwd.clone(),
    };

    let wrapped = wrap_remote_command(&effective_cwd, &command);
    let opts = ssh_options_from_settings(&state);
    let args = build_ssh_args(&session.target, opts, Some(&wrapped));
    let exec_timeout = timeout_secs
        .unwrap_or(EXEC_DEFAULT_TIMEOUT_SECS)
        .clamp(1, EXEC_MAX_TIMEOUT_SECS);
    let run = Box::pin(run_program("ssh", &args, exec_timeout)).await?;

    Ok(SshExecResult {
        session_id: id.to_string(),
        command,
        cwd: effective_cwd,
        exit_code: run.exit_code,
        duration_ms: run.duration_ms,
        stdout: run.stdout,
        stderr: run.stderr,
        timed_out: run.timed_out,
        warnings: safety.warnings,
    })
}

/// Upload or download a file/directory over `scp` for an existing session. The
/// local path is confined to the active workspace.
#[tauri::command]
pub async fn ssh_transfer(
    state: State<'_, SharedState>,
    session_id: String,
    direction: TransferDirection,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
) -> Result<SshTransferResult, String> {
    let id = uuid::Uuid::parse_str(session_id.trim())
        .map_err(|_| "SshTransfer requires a valid sessionId from SshConnect.".to_string())?;
    let session = state.ssh.get(id).ok_or_else(|| {
        "No such SSH session. Call SshConnect first (or it was disconnected).".to_string()
    })?;

    let remote_path = remote_path.trim().to_string();
    if remote_path.is_empty() {
        return Err("SshTransfer requires a remotePath.".to_string());
    }
    if local_path.trim().is_empty() {
        return Err("SshTransfer requires a localPath inside the workspace.".to_string());
    }
    // Confine the local side to the workspace: uploads read an existing path;
    // downloads write a (possibly new) path, so its parent must resolve.
    let local = match direction {
        TransferDirection::Upload => {
            resolve_workspace_path(&state, std::path::Path::new(&local_path))?
        }
        TransferDirection::Download => {
            resolve_workspace_path_for_write(&state, std::path::Path::new(&local_path))?
        }
    };
    if direction == TransferDirection::Upload && !local.exists() {
        return Err(format!("Upload source does not exist: {}", local.display()));
    }
    let recursive = recursive.unwrap_or(false);
    let local_str = local.to_string_lossy().to_string();
    let opts = ssh_options_from_settings(&state);
    let args = build_scp_args(
        &session.target,
        opts,
        &local_str,
        &remote_path,
        direction,
        recursive,
    );
    let run = Box::pin(run_program("scp", &args, TRANSFER_TIMEOUT_SECS)).await?;
    let mut success = run.exit_code == Some(0);
    let mut stderr = run.stderr;

    // Defence in depth on top of `scp -s`: after a download, re-walk everything
    // that landed locally and refuse anything that escaped the intended
    // destination directory. A hostile server (legacy-scp filename injection,
    // CVE-2019-6111) or a planted symlink could otherwise drop files outside the
    // workspace; if one did, fail the whole transfer rather than report success.
    if success && direction == TransferDirection::Download {
        if let Err(reason) = verify_download_confined(&local) {
            success = false;
            stderr = if stderr.trim().is_empty() {
                reason
            } else {
                format!("{stderr}\n{reason}")
            };
        }
    }

    Ok(SshTransferResult {
        session_id: id.to_string(),
        direction,
        local_path: local_str,
        remote_path,
        recursive,
        success,
        exit_code: run.exit_code,
        duration_ms: run.duration_ms,
        stdout: run.stdout,
        stderr,
        timed_out: run.timed_out,
    })
}

/// Confirm that a completed download stayed inside its intended destination.
///
/// The containment boundary is the canonical destination directory: the
/// destination itself when it is (or became) a directory, otherwise the
/// directory that holds the downloaded file. Every entry reachable under that
/// boundary is canonicalized (resolving symlinks) and checked with
/// [`crate::path_starts_with`]; the first escape fails the whole transfer.
fn verify_download_confined(dest: &std::path::Path) -> Result<(), String> {
    // Nothing on disk → nothing could have escaped (e.g. the source was empty).
    let Ok(canonical) = dest.canonicalize() else {
        return Ok(());
    };
    let boundary = if canonical.is_dir() {
        canonical.clone()
    } else {
        canonical
            .parent()
            .map_or_else(|| canonical.clone(), std::path::Path::to_path_buf)
    };

    let mut stack = vec![canonical];
    // Canonical dirs already descended into, so a symlink cycle inside the
    // boundary (e.g. `dest/loop -> dest`) can't spin the walk forever.
    let mut visited = std::collections::HashSet::new();
    while let Some(entry) = stack.pop() {
        let Ok(real) = entry.canonicalize() else {
            // A path we cannot canonicalize (broken symlink, race) is treated as
            // suspect and rejected rather than silently trusted.
            return Err(format!(
                "Download verification failed: could not resolve {} — transfer rejected.",
                entry.display()
            ));
        };
        if !crate::path_starts_with(&real, &boundary) {
            return Err(format!(
                "Download verification failed: {} escaped the destination directory {} — transfer rejected.",
                real.display(),
                boundary.display()
            ));
        }
        // Recurse into real directories only (canonicalize already resolved any
        // symlink, so a symlinked dir is followed once to its real target), and
        // only the first time we reach each canonical directory.
        if real.is_dir() && visited.insert(real.clone()) {
            match std::fs::read_dir(&real) {
                Ok(children) => {
                    for child in children.flatten() {
                        stack.push(child.path());
                    }
                }
                Err(error) => {
                    return Err(format!(
                        "Download verification failed: could not read {}: {error} — transfer rejected.",
                        real.display()
                    ));
                }
            }
        }
    }
    Ok(())
}

/// List active sessions and discoverable `~/.ssh/config` hosts, plus OpenSSH
/// availability. Read-only; available in every agent mode.
#[tauri::command]
pub async fn ssh_list(state: State<'_, SharedState>) -> Result<SshOverview, String> {
    let opts = ssh_options_from_settings(&state);
    let sessions = state.ssh.list();
    let config_hosts = read_config_hosts();
    let version = Box::pin(ssh_version()).await;
    Ok(SshOverview {
        available: version.is_some(),
        version,
        sessions,
        config_hosts,
        strict_host_key: opts.host_key_policy == HostKeyPolicy::Strict,
        connect_timeout_secs: opts.connect_timeout_secs,
    })
}

/// Close one session (`sessionId`) or every session (`all`).
#[tauri::command]
pub fn ssh_disconnect(
    state: State<'_, SharedState>,
    session_id: Option<String>,
    all: Option<bool>,
) -> Result<SshDisconnectResult, String> {
    let closed = if all.unwrap_or(false) {
        state.ssh.clear()
    } else {
        let raw = session_id
            .ok_or_else(|| "SshDisconnect requires a sessionId (or all=true).".to_string())?;
        let id = uuid::Uuid::parse_str(raw.trim())
            .map_err(|_| "SshDisconnect requires a valid sessionId.".to_string())?;
        usize::from(state.ssh.remove(id))
    };
    Ok(SshDisconnectResult {
        closed,
        remaining: state.ssh.count(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_user_host_prefers_explicit_user() {
        let (host, user) = split_user_host("admin@box", Some("deploy".into()));
        assert_eq!(host, "admin@box");
        assert_eq!(user.as_deref(), Some("deploy"));
    }

    #[test]
    fn split_user_host_parses_embedded_user() {
        let (host, user) = split_user_host("deploy@example.com", None);
        assert_eq!(host, "example.com");
        assert_eq!(user.as_deref(), Some("deploy"));
    }

    #[test]
    fn split_user_host_plain_host() {
        let (host, user) = split_user_host("example.com", None);
        assert_eq!(host, "example.com");
        assert_eq!(user, None);
    }

    #[test]
    fn clean_stderr_handles_empty() {
        assert_eq!(clean_stderr("   "), "(no error output)");
        assert_eq!(clean_stderr(" boom "), "boom");
    }

    #[tokio::test]
    async fn read_capped_stops_at_cap_and_flags_truncation() {
        // Input longer than the cap: we read exactly `cap` bytes and report it.
        let data = vec![b'x'; 100];
        let (buf, capped) = Box::pin(read_capped(&mut data.as_slice(), 40)).await;
        assert_eq!(buf.len(), 40);
        assert!(capped, "hitting the cap must be reported");
    }

    #[tokio::test]
    async fn read_capped_returns_all_when_under_cap() {
        let data = b"hello world".to_vec();
        let (buf, capped) = Box::pin(read_capped(&mut data.as_slice(), 1024)).await;
        assert_eq!(buf, data);
        assert!(!capped, "output below the cap is not truncated");
    }

    #[tokio::test]
    async fn read_capped_handles_exact_cap_without_flagging() {
        let data = vec![b'a'; 64];
        let (buf, capped) = Box::pin(read_capped(&mut data.as_slice(), 64)).await;
        // Exactly `cap` bytes available: we consume them and the next read sees
        // EOF, so this is NOT a truncation.
        assert_eq!(buf.len(), 64);
        assert!(
            !capped,
            "an input that exactly fills the cap is complete, not truncated"
        );
    }

    #[test]
    fn mark_truncated_appends_byte_accurate_marker() {
        let marked = mark_truncated("partial".to_string());
        assert!(marked.starts_with("partial"));
        assert!(marked.contains(&SSH_MAX_OUTPUT_BYTES.to_string()));
        assert!(marked.contains("output truncated"));
    }

    #[test]
    fn verify_download_confined_accepts_files_inside_destination() {
        let dir = std::env::temp_dir().join(format!("lux-ssh-dl-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"hi").unwrap();
        std::fs::write(dir.join("sub").join("b.txt"), b"yo").unwrap();
        assert!(verify_download_confined(&dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_download_confined_missing_path_is_ok() {
        // A destination that never materialized cannot have leaked anything.
        let missing =
            std::env::temp_dir().join(format!("lux-ssh-missing-{}", uuid::Uuid::new_v4()));
        assert!(verify_download_confined(&missing).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn verify_download_confined_rejects_symlink_escape() {
        let base = std::env::temp_dir().join(format!("lux-ssh-esc-{}", uuid::Uuid::new_v4()));
        let dest = base.join("dest");
        let outside = base.join("outside");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), b"leak").unwrap();
        // A symlink inside the destination that points outside it must be rejected:
        // canonicalizing the link resolves to `outside`, which is not under `dest`.
        std::os::unix::fs::symlink(&outside, dest.join("escape")).unwrap();
        assert!(verify_download_confined(&dest).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }
}
