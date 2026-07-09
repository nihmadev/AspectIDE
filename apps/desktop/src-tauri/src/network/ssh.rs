//! SSH engine glue вЂ” the process/runtime layer around [`aspect_ssh`].
//!
//! AspectIDE drives the system OpenSSH client (`ssh`/`scp`) so it honors the user's
//! existing `~/.ssh/config`, keys, `ssh-agent`, and `known_hosts` out of the box.
//! Every invocation is built by [`aspect_ssh`] with hardened, non-interactive flags
//! (`BatchMode=yes`, no password prompts, fast `ConnectTimeout`, forwardings
//! disabled), which is what makes SSH usable by the agent: commands return
//! structured `{exitCode, stdout, stderr}` instead of hanging on a host-key or
//! password prompt the way a raw `ssh` through the Shell tool would.
//!
//! A "session" is a stored destination plus a sticky logical working directory;
//! AspectIDE runs one short-lived `ssh` process per command, so nothing is left running
//! remotely and no credential is ever held in memory.

use std::{path::PathBuf, process::Stdio, time::Instant};

use aspect_ssh::{
    build_scp_args, build_ssh_args, parse_probe_output, parse_ssh_config, probe_command,
    wrap_remote_command, HostKeyPolicy, SshConfigHost, SshOptions, SshSession, SshTarget,
    TransferDirection,
};
use serde::Serialize;
use tauri::State;
use tokio::time::{timeout, Duration};

use aspect_agent_tools::{
    process_kill::kill_process_tree,
    output_truncate::truncate_shell_output,
};
use crate::{
    resolve_workspace_path, resolve_workspace_path_for_write, SharedState,
};

/// Settings key (user scope): when `true`, refuse hosts not already in
/// `known_hosts` (`StrictHostKeyChecking=yes`) instead of trust-on-first-use.
pub const STRICT_HOST_KEY_KEY: &str = "ai.ssh.strictHostKey";
/// Settings key (user scope): `ConnectTimeout` in seconds (1вЂ“120).
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
/// Hard cap on how many bytes AspectIDE reads from a single `ssh`/`scp` stream before
/// it stops and tree-kills the child. A runaway remote command (`yes`,
/// `cat /dev/urandom`) would otherwise buffer unbounded into memory. 8 MiB is
/// generous for legitimate command output while bounding the worst case; the
/// captured text is then truncated again to the display ceiling by
/// [`truncate_shell_output`].
const SSH_MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

// в”Ђв”Ђ Result types (camelCase mirrors consumed by the TS layer) в”Ђв”Ђ

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConnectResult {
    pub session: SshSession,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshExecResult {
    /// The session id, echoed under the same `session` key the model was shown as
    /// the INPUT parameter (E42), so re-passing the returned id under that name
    /// works. `session_id` (below) is kept for the existing TS `SshExecResult`.
    pub session: String,
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
    /// Echoed under the `session` input-param key (E42), alongside `session_id`.
    pub session: String,
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

// в”Ђв”Ђ Internal process runner в”Ђв”Ђ

struct RawRun {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    duration_ms: u128,
    timed_out: bool,
}

/// Append a clear, byte-accurate truncation marker to capped output.
fn mark_truncated(text: String) -> String {
    format!("{text}\n[output truncated at {SSH_MAX_OUTPUT_BYTES} bytes]")
}

/// Read from `reader` in chunks, appending into the shared, mutex-guarded `sink`
/// as bytes arrive so whatever was captured before a timeout is preserved.
///
/// The capture is bounded to `cap` bytes (the append is gated on the current
/// length) while the read loop keeps draining to EOF, so a producer that
/// out-runs the cap never blocks on a full pipe. Returns whether the stream had
/// MORE than `cap` bytes (i.e. output was truncated); a stream that ends exactly
/// at `cap` is complete, not truncated.
async fn stream_capped_into<R>(
    reader: &mut R,
    sink: &std::sync::Arc<tokio::sync::Mutex<Vec<u8>>>,
    cap: usize,
) -> bool
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut chunk = [0_u8; 16 * 1024];
    let mut capped = false;
    loop {
        match reader.read(&mut chunk).await {
            // Clean EOF (`Ok(0)`) or a read error both end capture with whatever
            // was already appended.
            Ok(0) | Err(_) => return capped,
            Ok(n) => {
                let mut buffer = sink.lock().await;
                let room = cap.saturating_sub(buffer.len());
                // Strictly more than the remaining room means real overflow: keep
                // exactly `cap` bytes and flag truncation. Filling the cap exactly
                // (`n == room`) is not yet truncation вЂ” keep looping so a following
                // EOF stays untruncated while any extra byte trips the flag.
                if n > room {
                    buffer.extend_from_slice(&chunk[..room]);
                    capped = true;
                } else {
                    buffer.extend_from_slice(&chunk[..n]);
                }
            }
        }
    }
}

/// Render the captured bytes for one stream: truncate to the display ceiling and
/// append the byte-accurate marker when the stream overflowed the hard cap.
fn finish_stream(buf: &[u8], capped: bool) -> String {
    let text = truncate_shell_output(&String::from_utf8_lossy(buf));
    if capped {
        mark_truncated(text)
    } else {
        text
    }
}

/// Spawn `program` with `args`, drain both pipes concurrently, and enforce a hard
/// timeout with a full process-tree kill вЂ” the same battle-tested shape as the
/// `Shell` tool. `stdin` is null so OpenSSH can never block reading a prompt.
///
/// E40: partial output is preserved. Both pipes stream into shared, mutex-guarded
/// buffers OUTSIDE the timed future, so on a timeout (or an output-cap kill) we
/// return whatever stdout/stderr arrived before the deadline instead of dropping
/// it, with `timed_out` set. This mirrors the `Shell` tool's timeout handling.
async fn run_program(program: &str, args: &[String], timeout_secs: u64) -> Result<RawRun, String> {
    let started = Instant::now();
    let mut process = tokio::process::Command::new(program);
    process.args(args);
    process.stdin(Stdio::null());
    process.stdout(Stdio::piped());
    process.stderr(Stdio::piped());
    process.kill_on_drop(true);
    #[cfg(windows)]
    process.creation_flags(aspect_agent_tools::shell_command::CREATE_NO_WINDOW);
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

    // E40: keep the capture buffers outside the timed future so partial output is
    // returned on timeout instead of being discarded. The collect future fills
    // them as bytes arrive; on timeout we read what landed so far via the Arc.
    let shared_stdout: std::sync::Arc<tokio::sync::Mutex<Vec<u8>>> =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let shared_stderr: std::sync::Arc<tokio::sync::Mutex<Vec<u8>>> =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    // Report truncation back out of the collect future without needing the buffers.
    let shared_capped: std::sync::Arc<tokio::sync::Mutex<(bool, bool)>> =
        std::sync::Arc::new(tokio::sync::Mutex::new((false, false)));

    // Drain both pipes concurrently (avoids a full-pipe deadlock), each bounded to
    // SSH_MAX_OUTPUT_BYTES so a runaway remote stream can't exhaust memory. If a
    // cap is hit we stop appending and tree-kill the child below so it stops
    // producing, then surface the truncated output with a marker.
    let collect_stdout = std::sync::Arc::clone(&shared_stdout);
    let collect_stderr = std::sync::Arc::clone(&shared_stderr);
    let collect_capped = std::sync::Arc::clone(&shared_capped);
    let collect = async {
        let read_stdout = async {
            if let Some(pipe) = stdout_pipe.as_mut() {
                Box::pin(stream_capped_into(
                    pipe,
                    &collect_stdout,
                    SSH_MAX_OUTPUT_BYTES,
                ))
                .await
            } else {
                false
            }
        };
        let read_stderr = async {
            if let Some(pipe) = stderr_pipe.as_mut() {
                Box::pin(stream_capped_into(
                    pipe,
                    &collect_stderr,
                    SSH_MAX_OUTPUT_BYTES,
                ))
                .await
            } else {
                false
            }
        };
        let (stdout_capped, stderr_capped) = tokio::join!(read_stdout, read_stderr);
        *collect_capped.lock().await = (stdout_capped, stderr_capped);
        let capped = stdout_capped || stderr_capped;
        // Only reap the child if neither stream overflowed; on a cap we skip the
        // potentially-unbounded wait and go straight to the tree-kill below.
        if capped {
            None
        } else {
            Some(child.wait().await)
        }
    };

    let outcome = Box::pin(timeout(Duration::from_secs(timeout_secs), collect)).await;
    let duration_ms = started.elapsed().as_millis();
    let (stdout_capped, stderr_capped) = *shared_capped.lock().await;
    let stdout_buf = shared_stdout.lock().await.clone();
    let stderr_buf = shared_stderr.lock().await.clone();

    match outcome {
        // Ran to completion (neither stream overflowed): report the real exit code.
        Ok(Some(Ok(status))) => Ok(RawRun {
            exit_code: status.code(),
            stdout: finish_stream(&stdout_buf, stdout_capped),
            stderr: finish_stream(&stderr_buf, stderr_capped),
            duration_ms,
            timed_out: false,
        }),
        // A cap was hit (`wait` is `None`): kill the still-running child so it stops
        // producing, then return the truncated output. Exit code is unknowable вЂ”
        // the process was killed rather than allowed to finish.
        Ok(None) => {
            kill_process_tree(child_pid).await;
            let _ = child.start_kill();
            Ok(RawRun {
                exit_code: None,
                stdout: finish_stream(&stdout_buf, stdout_capped),
                stderr: finish_stream(&stderr_buf, stderr_capped),
                duration_ms,
                timed_out: false,
            })
        }
        Ok(Some(Err(error))) => Err(format!("Failed to run `{program}`: {error}")),
        // E40: timed out вЂ” `child` is still alive (only the collect future's borrow
        // was dropped). Kill the whole tree, then return whatever partial stdout /
        // stderr was captured before the deadline with `timed_out` set, appending a
        // timeout notice to stderr rather than discarding the partial output.
        Err(_) => {
            kill_process_tree(child_pid).await;
            let _ = child.start_kill();
            let partial_stdout = finish_stream(&stdout_buf, stdout_capped);
            let partial_stderr = finish_stream(&stderr_buf, stderr_capped);
            let timeout_note = format!("{program} timed out after {timeout_secs} seconds");
            Ok(RawRun {
                exit_code: None,
                stdout: partial_stdout,
                stderr: if partial_stderr.is_empty() {
                    timeout_note
                } else {
                    format!("{partial_stderr}\n---\n{timeout_note}")
                },
                duration_ms,
                timed_out: true,
            })
        }
    }
}

// в”Ђв”Ђ Settings + environment helpers в”Ђв”Ђ

fn ssh_options_from_settings(state: &State<'_, SharedState>) -> SshOptions {
    let mut opts = SshOptions::default();
    if let Ok(guard) = state.settings.lock() {
        if let Some(store) = guard.as_ref() {
            if let Some(value) = store.get(aspect_core::SettingsScope::User, STRICT_HOST_KEY_KEY) {
                if value.value.as_bool() == Some(true) {
                    opts.host_key_policy = HostKeyPolicy::Strict;
                }
            }
            if let Some(value) = store.get(aspect_core::SettingsScope::User, CONNECT_TIMEOUT_KEY) {
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

/// Neutralize the tokens that trip the shared classifier's LOCAL Windows/cmd
/// catastrophic rules, WITHOUT disturbing any POSIX signal, so a re-classification
/// yields a pure-POSIX verdict (E41).
///
/// The Windows drive-erase rules all require a Windows drive-root operand
/// (`c:`, `c:\`, `c:/`, `%SystemDrive%` вЂ¦); the two operand-free rules key on the
/// verbs `diskpart` and `cipher /w`. Replacing exactly those tokens with an inert
/// placeholder defuses every Windows rule while leaving POSIX targets (which are
/// `/`-rooted paths, `/dev/*` devices, `~`, `$HOME`, etc.) and all warning
/// signals untouched вЂ” so a masked POSIX catastrophe in a compound line
/// (`format c: ; rm -rf /`) still surfaces and blocks.
fn neutralize_windows_tokens(command: &str) -> Option<String> {
    let mut changed = false;
    let rewritten = command
        .split_whitespace()
        .map(|token| {
            let bare = token
                .trim_matches(|c| c == '"' || c == '\'')
                .trim_end_matches(['\\', '/']);
            let lower = bare.to_ascii_lowercase();
            // cmd env-var drive roots, or the operand-free Windows disk verbs
            // (`diskpart`, `cipher` for `cipher /w`).
            let is_win_verb_or_env = matches!(
                lower.as_str(),
                "%systemdrive%" | "%systemroot%" | "%windir%" | "diskpart" | "cipher"
            );
            // `X:` / `X:\` / `X:/` drive-letter roots for any drive letter.
            let bytes = bare.as_bytes();
            let is_drive_root =
                bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
            if is_win_verb_or_env || is_drive_root {
                changed = true;
                "__win__".to_string()
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    changed.then_some(rewritten)
}

/// Classify a command bound for a REMOTE POSIX host, scoping the shared
/// classifier's catastrophic verdict to the POSIX surface (E41).
///
/// The shared classifier models the combined POSIX + cmd surface (it drives the
/// local `cmd /C` `Shell` tool). Applied verbatim to a command that will run on a
/// POSIX remote, its cmd-specific rules produce false-positive BLOCKS the model
/// cannot explain (e.g. a valid POSIX command whose tokens trip a Windows
/// drive-root rule). When such Windows-only tokens are present we classify a
/// neutralized copy so only the POSIX guards can fire (fork bomb, `rm -rf /` /
/// protected paths, `mkfs`, `dd`/redirect to a block device, recursive
/// chmod/chown at root, `--no-preserve-root`); a masked POSIX catastrophe in the
/// same line still surfaces because only the Windows tokens are replaced. When no
/// Windows tokens are present the original command is classified verbatim, so
/// nothing about the POSIX-only path changes.
fn classify_remote_command(command: &str) -> aspect_security::ShellSafetyReport {
    match neutralize_windows_tokens(command) {
        Some(neutralized) => aspect_security::classify_shell_command(&neutralized),
        None => aspect_security::classify_shell_command(command),
    }
}

// в”Ђв”Ђ Tauri commands в”Ђв”Ђ

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
    // (mirrors the TS path) вЂ” `ssh -p 0` is invalid and would just fail.
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
    // (e.g. killed by a signal) вЂ” never treat that as a successful login.
    if run.exit_code.is_none() {
        return Err(format!(
            "SSH connection to {} did not complete (the ssh process was terminated before exiting). {}",
            target.destination(),
            clean_stderr(&run.stderr)
        ));
    }
    // 255 is OpenSSH's own failure (connect/auth/host-key). Any other code means
    // the remote shell ran вЂ” even a non-POSIX shell where the probe came back
    // empty вЂ” so the session is still usable.
    if run.exit_code == Some(SSH_CONNECTION_ERROR_CODE) {
        return Err(format!(
            "SSH connection to {} failed: {}\nAspectIDE connects non-interactively (BatchMode), so it cannot answer password or passphrase prompts. Use an SSH key via ssh-agent or an identityFile, and make sure the host key is accepted. For a changed/unknown host key with strict checking on, fix ~/.ssh/known_hosts first.",
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
        .map_err(|_| "SshExec requires a valid session id from SshConnect.".to_string())?;
    let session = state.ssh.get(id).ok_or_else(|| {
        "No such SSH session. Call SshConnect first (or it was disconnected).".to_string()
    })?;

    let command = command.trim().to_string();
    if command.is_empty() {
        return Err("SshExec requires a non-empty command.".to_string());
    }
    // E41: classify against the POSIX surface only вЂ” the shared classifier's
    // Windows/cmd-specific catastrophic rules don't apply to a remote POSIX host
    // and would otherwise false-positive-block legitimate remote commands.
    let safety = classify_remote_command(&command);
    if let Some(reason) = safety.blocked {
        return Err(format!(
            "AspectIDE blocked this remote command for safety ({reason}). If it is genuinely intended, run it yourself."
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

    let id_str = id.to_string();
    Ok(SshExecResult {
        // E42: echo the id under BOTH keys so re-passing the returned id works
        // whether the model reuses `session` (the input param it was shown) or
        // `sessionId` (the TS `SshExecResult` field).
        session: id_str.clone(),
        session_id: id_str,
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
        .map_err(|_| "SshTransfer requires a valid session id from SshConnect.".to_string())?;
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
    // The resolved local path is `canonicalize`d, which on Windows yields the
    // extended-length verbatim form (`\\?\E:\вЂ¦`). scp splits every non-flag
    // argument into `host:path` at the first `:` before any separator, so the
    // verbatim prefix makes scp read `\\?\E` as a hostname ("Could not resolve
    // hostname"). `local_scp_path` strips the `\\?\` prefix (mirrors `ai_tools`'
    // `dunce::simplified` usage) back to a plain `E:\вЂ¦` the Win32 OpenSSH client
    // treats as local, so only the REMOTE side is ever `host:path`.
    let local_str = local_scp_path(&local);
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

    let id_str = id.to_string();
    Ok(SshTransferResult {
        // E42: echo the id under both `session` and `sessionId` (see SshExecResult).
        session: id_str.clone(),
        session_id: id_str,
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

/// Render a resolved LOCAL path as a plain filesystem string safe to hand to
/// `scp`/`sftp`.
///
/// The path arrives from [`resolve_workspace_path`], which `canonicalize`s it вЂ”
/// and on Windows `canonicalize` returns the extended-length **verbatim** form
/// (`\\?\E:\a\b.py`). That form is doubly wrong for scp:
///
/// - OpenSSH does not accept a verbatim path as a filename; and, worse,
/// - scp splits every non-flag argument into `host:path` at the first `:` that
///   precedes any `/` (its `colon()` scan). In `\\?\E:\вЂ¦` the first `:` follows
///   `\\?\E`, so scp reads `\\?\E` as a **hostname** and fails with
///   "Could not resolve hostname" вЂ” the exact smoke-test failure.
///
/// [`dunce::simplified`] strips the `\\?\` prefix back to the ordinary
/// `E:\a\b.py` (mirrors how `ai_tools`/`aspect-terminal` normalize a Windows path
/// for external tools). The bundled Win32 OpenSSH client вЂ” the one this engine
/// targets вЂ” special-cases a leading drive letter (`X:`) as a local path, so the
/// simplified form is handed to scp verbatim and only the REMOTE side is ever
/// `host:path`. On non-Windows targets there is no verbatim prefix or drive
/// letter, so the path passes through unchanged.
fn local_scp_path(path: &std::path::Path) -> String {
    dunce::simplified(path).to_string_lossy().into_owned()
}

/// Confirm that a completed download stayed inside its intended destination.
///
/// The containment boundary is the canonical destination directory: the
/// destination itself when it is (or became) a directory, otherwise the
/// directory that holds the downloaded file. Every entry reachable under that
/// boundary is canonicalized (resolving symlinks) and checked with
/// [`crate::path_starts_with`]; the first escape fails the whole transfer.
fn verify_download_confined(dest: &std::path::Path) -> Result<(), String> {
    // Nothing on disk в†’ nothing could have escaped (e.g. the source was empty).
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
                "Download verification failed: could not resolve {} вЂ” transfer rejected.",
                entry.display()
            ));
        };
        if !crate::path_starts_with(&real, &boundary) {
            return Err(format!(
                "Download verification failed: {} escaped the destination directory {} вЂ” transfer rejected.",
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
                        "Download verification failed: could not read {}: {error} вЂ” transfer rejected.",
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

/// Close one session (by `session` id) or every session (`all`).
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
            .ok_or_else(|| "SshDisconnect requires a session id (or all=true).".to_string())?;
        let id = uuid::Uuid::parse_str(raw.trim())
            .map_err(|_| "SshDisconnect requires a valid session id.".to_string())?;
        usize::from(state.ssh.remove(id))
    };
    Ok(SshDisconnectResult {
        closed,
        remaining: state.ssh.count(),
    })
}

