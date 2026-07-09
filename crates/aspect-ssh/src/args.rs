//! Pure construction of hardened, non-interactive OpenSSH argument vectors.
//!
//! This is the security-critical core of the SSH engine, kept I/O-free so it can
//! be exhaustively unit-tested. Every `ssh`/`scp` invocation AspectIDE spawns goes
//! through here, which guarantees the same hardening on all of them:
//!
//! - `BatchMode=yes` + `NumberOfPasswordPrompts=0`: never block on an interactive
//!   password/passphrase prompt вЂ” the #1 reason naive `ssh` calls hang an agent.
//! - `StrictHostKeyChecking=accept-new` (default): trust-on-first-use, but refuse
//!   a changed key. Never the unsafe `no`/`off`.
//! - `ConnectTimeout`: a dead host fails fast instead of stalling.
//! - Agent/X11 forwarding and all port forwardings are force-disabled regardless
//!   of what `~/.ssh/config` requests, so a tool call can't open a tunnel.
//! - `LogLevel=ERROR`: keep banner/warning noise out of captured output.

use crate::model::{SshOptions, SshTarget, TransferDirection};

/// Shared `-o key=value` hardening flags applied to both `ssh` and `scp`.
fn hardening_options(opts: SshOptions) -> Vec<String> {
    vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "NumberOfPasswordPrompts=0".into(),
        "-o".into(),
        format!(
            "StrictHostKeyChecking={}",
            opts.host_key_policy.ssh_option_value()
        ),
        "-o".into(),
        format!("ConnectTimeout={}", opts.connect_timeout_secs.max(1)),
        "-o".into(),
        "ForwardAgent=no".into(),
        "-o".into(),
        "ForwardX11=no".into(),
        "-o".into(),
        "ClearAllForwardings=yes".into(),
        "-o".into(),
        "LogLevel=ERROR".into(),
    ]
}

/// Identity flags: when an explicit key file is set, use *only* that key
/// (`IdentitiesOnly=yes`) so auth doesn't wander through every agent identity.
fn identity_args(target: &SshTarget) -> Vec<String> {
    match &target.identity_file {
        Some(path) if !path.trim().is_empty() => vec![
            "-i".into(),
            path.clone(),
            "-o".into(),
            "IdentitiesOnly=yes".into(),
        ],
        _ => Vec::new(),
    }
}

/// Build the `ssh` argv (without the `ssh` program name itself).
///
/// `remote_command` is passed as a single trailing argument; OpenSSH hands it to
/// the remote login shell verbatim. `None` opens no command (used only for pure
/// connectivity checks where the caller appends its own probe).
#[must_use]
pub fn build_ssh_args(
    target: &SshTarget,
    opts: SshOptions,
    remote_command: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::with_capacity(24);
    // `-T`: never request a PTY for an exec вЂ” keeps output free of terminal
    // escape sequences and stops the remote shell from going interactive.
    args.push("-T".into());
    args.extend(hardening_options(opts));
    args.extend(identity_args(target));
    if let Some(port) = target.port {
        args.push("-p".into());
        args.push(port.to_string());
    }
    // `--` terminates ssh's own option parsing: it both stops a destination that
    // begins with `-` from being read as a flag (option injection) and ensures the
    // remote command is NOT prefixed with a stray `--` (ssh joins everything after
    // the destination into the remote command, so `--` must precede the host, not
    // the command).
    args.push("--".into());
    args.push(target.destination());
    if let Some(command) = remote_command {
        if !command.is_empty() {
            args.push(command.to_string());
        }
    }
    args
}

/// Build the `scp` argv (without the `scp` program name itself).
///
/// Note `scp` selects the port with `-P` (capital), unlike `ssh`'s `-p`.
#[must_use]
pub fn build_scp_args(
    target: &SshTarget,
    opts: SshOptions,
    local_path: &str,
    remote_path: &str,
    direction: TransferDirection,
    recursive: bool,
) -> Vec<String> {
    let mut args = Vec::with_capacity(24);
    // `-s`: drive the transfer over the modern **SFTP** protocol instead of the
    // legacy SCP/RCP wire protocol. The legacy protocol lets the *remote* server
    // dictate the filenames written locally, so a malicious server can smuggle
    // `../` or absolute paths and write outside the requested destination
    // (CVE-2019-6111 path-traversal class). SFTP transfers only the paths the
    // client asked for. `ssh.rs` re-walks downloads afterwards as defence in depth.
    args.push("-s".into());
    if recursive {
        args.push("-r".into());
    }
    args.extend(hardening_options(opts));
    args.extend(identity_args(target));
    if let Some(port) = target.port {
        args.push("-P".into());
        args.push(port.to_string());
    }
    // `--` terminates options so a leading-dash path is never read as a flag.
    args.push("--".into());
    // `scp_destination` brackets an IPv6 literal host so scp's first-colon split
    // doesn't mistake the address for `host:path`.
    let remote = format!("{}:{remote_path}", target.scp_destination());
    match direction {
        TransferDirection::Upload => {
            args.push(local_path.to_string());
            args.push(remote);
        }
        TransferDirection::Download => {
            args.push(remote);
            args.push(local_path.to_string());
        }
    }
    args
}

/// Wrap a remote command so it runs in `cwd` (the sticky working directory).
///
/// An empty `cwd` runs the command as-is in the login default directory. The
/// path is POSIX single-quoted; if `cd` fails the `&&` short-circuits and the
/// failure surfaces as a clean non-zero exit with a message on stderr.
#[must_use]
pub fn wrap_remote_command(cwd: &str, command: &str) -> String {
    let cwd = cwd.trim();
    if cwd.is_empty() {
        return command.to_string();
    }
    format!("cd {} && {command}", posix_single_quote(cwd))
}

/// POSIX-safe single-quoting: wrap in `'вЂ¦'`, and render embedded single quotes as
/// the standard `'\''` escape so no metacharacter can break out of the literal.
#[must_use]
pub fn posix_single_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

/// The probe command run once at connect time.
///
/// It confirms auth works and captures the remote identity + starting directory
/// in a single round-trip. Each field is emitted on its own prefixed line so
/// arbitrary login banners can't confuse the parser ([`parse_probe_output`]).
#[must_use]
pub const fn probe_command() -> &'static str {
    "printf 'ASPECTSSH_CWD=%s\\n' \"$(pwd 2>/dev/null)\"; \
printf 'ASPECTSSH_SYS=%s\\n' \"$(uname -srm 2>/dev/null)\"; \
printf 'ASPECTSSH_USR=%s\\n' \"$(id -un 2>/dev/null || whoami 2>/dev/null)\""
}

/// Parse [`probe_command`] output into a [`ProbeInfo`], scanning for the prefixed
/// lines and ignoring any surrounding banner/MOTD text.
#[must_use]
pub fn parse_probe_output(stdout: &str) -> crate::model::ProbeInfo {
    let mut info = crate::model::ProbeInfo::default();
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("ASPECTSSH_CWD=") {
            let value = value.trim();
            if !value.is_empty() {
                info.cwd = Some(value.to_string());
            }
        } else if let Some(value) = line.strip_prefix("ASPECTSSH_SYS=") {
            let value = value.trim();
            if !value.is_empty() {
                info.system = Some(value.to_string());
            }
        } else if let Some(value) = line.strip_prefix("ASPECTSSH_USR=") {
            let value = value.trim();
            if !value.is_empty() {
                info.user = Some(value.to_string());
            }
        }
    }
    info
}

