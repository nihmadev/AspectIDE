//! Pure construction of hardened, non-interactive OpenSSH argument vectors.
//!
//! This is the security-critical core of the SSH engine, kept I/O-free so it can
//! be exhaustively unit-tested. Every `ssh`/`scp` invocation Lux spawns goes
//! through here, which guarantees the same hardening on all of them:
//!
//! - `BatchMode=yes` + `NumberOfPasswordPrompts=0`: never block on an interactive
//!   password/passphrase prompt — the #1 reason naive `ssh` calls hang an agent.
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
    // `-T`: never request a PTY for an exec — keeps output free of terminal
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

/// POSIX-safe single-quoting: wrap in `'…'`, and render embedded single quotes as
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
    "printf 'LUXSSH_CWD=%s\\n' \"$(pwd 2>/dev/null)\"; \
printf 'LUXSSH_SYS=%s\\n' \"$(uname -srm 2>/dev/null)\"; \
printf 'LUXSSH_USR=%s\\n' \"$(id -un 2>/dev/null || whoami 2>/dev/null)\""
}

/// Parse [`probe_command`] output into a [`ProbeInfo`], scanning for the prefixed
/// lines and ignoring any surrounding banner/MOTD text.
#[must_use]
pub fn parse_probe_output(stdout: &str) -> crate::model::ProbeInfo {
    let mut info = crate::model::ProbeInfo::default();
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("LUXSSH_CWD=") {
            let value = value.trim();
            if !value.is_empty() {
                info.cwd = Some(value.to_string());
            }
        } else if let Some(value) = line.strip_prefix("LUXSSH_SYS=") {
            let value = value.trim();
            if !value.is_empty() {
                info.system = Some(value.to_string());
            }
        } else if let Some(value) = line.strip_prefix("LUXSSH_USR=") {
            let value = value.trim();
            if !value.is_empty() {
                info.user = Some(value.to_string());
            }
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HostKeyPolicy;

    fn target() -> SshTarget {
        SshTarget {
            host: "example.com".into(),
            user: Some("deploy".into()),
            port: Some(2222),
            identity_file: None,
        }
    }

    #[test]
    fn ssh_args_are_non_interactive_and_hardened() {
        let args = build_ssh_args(&target(), SshOptions::default(), Some("uptime"));
        let joined = args.join(" ");
        assert!(joined.contains("-T"));
        assert!(joined.contains("BatchMode=yes"));
        assert!(joined.contains("NumberOfPasswordPrompts=0"));
        assert!(joined.contains("StrictHostKeyChecking=accept-new"));
        assert!(joined.contains("ConnectTimeout=12"));
        assert!(joined.contains("ForwardAgent=no"));
        assert!(joined.contains("ClearAllForwardings=yes"));
        // Destination, port, and the command (after `--`) are all present.
        assert!(args.contains(&"deploy@example.com".to_string()));
        assert_eq!(
            args.windows(2).find(|w| w[0] == "-p").map(|w| &w[1]),
            Some(&"2222".to_string())
        );
        assert_eq!(args.last().unwrap(), "uptime");
        // `--` must precede the destination (terminates option parsing); the remote
        // command then follows the destination with no stray `--` of its own.
        let dashdash = args.iter().position(|a| a == "--").unwrap();
        let dest = args.iter().position(|a| a == "deploy@example.com").unwrap();
        assert!(dashdash < dest, "-- must come before the destination");
        assert_eq!(
            args.iter().filter(|a| *a == "--").count(),
            1,
            "exactly one -- separator"
        );
    }

    #[test]
    fn strict_policy_emits_yes() {
        let opts = SshOptions {
            host_key_policy: HostKeyPolicy::Strict,
            ..SshOptions::default()
        };
        let args = build_ssh_args(&target(), opts, None);
        assert!(args.join(" ").contains("StrictHostKeyChecking=yes"));
        // No command → the destination is the final argument, right after `--`.
        assert_eq!(args.last().unwrap(), "deploy@example.com");
        assert!(args.contains(&"--".to_string()));
    }

    #[test]
    fn identity_file_pins_identities_only() {
        let with_key = SshTarget {
            identity_file: Some("/home/u/.ssh/id_ed25519".into()),
            ..target()
        };
        let args = build_ssh_args(&with_key, SshOptions::default(), Some("ls"));
        let joined = args.join(" ");
        assert!(joined.contains("-i /home/u/.ssh/id_ed25519"));
        assert!(joined.contains("IdentitiesOnly=yes"));
    }

    #[test]
    fn no_user_uses_bare_host() {
        let anon = SshTarget {
            user: None,
            ..target()
        };
        assert_eq!(anon.destination(), "example.com");
    }

    #[test]
    fn scp_upload_orders_local_then_remote_with_capital_p() {
        let args = build_scp_args(
            &target(),
            SshOptions::default(),
            "/tmp/local.txt",
            "/srv/app/remote.txt",
            TransferDirection::Upload,
            false,
        );
        assert!(args.contains(&"-P".to_string()));
        assert!(!args.contains(&"-p".to_string()));
        // Every transfer is forced onto the SFTP protocol (see CVE-2019-6111).
        assert!(
            args.contains(&"-s".to_string()),
            "scp must force SFTP with -s"
        );
        let local = args.iter().position(|a| a == "/tmp/local.txt").unwrap();
        let remote = args
            .iter()
            .position(|a| a == "deploy@example.com:/srv/app/remote.txt")
            .unwrap();
        assert!(
            local < remote,
            "upload: local source precedes remote target"
        );
    }

    #[test]
    fn scp_download_orders_remote_then_local_and_recursive() {
        let args = build_scp_args(
            &target(),
            SshOptions::default(),
            "/tmp/dir",
            "/srv/dir",
            TransferDirection::Download,
            true,
        );
        assert!(args.contains(&"-r".to_string()));
        assert!(
            args.contains(&"-s".to_string()),
            "scp must force SFTP with -s"
        );
        let remote = args
            .iter()
            .position(|a| a == "deploy@example.com:/srv/dir")
            .unwrap();
        let local = args.iter().position(|a| a == "/tmp/dir").unwrap();
        assert!(
            remote < local,
            "download: remote source precedes local target"
        );
    }

    #[test]
    fn scp_brackets_ipv6_literal_host() {
        let v6 = SshTarget {
            host: "2001:db8::1".into(),
            user: Some("deploy".into()),
            port: None,
            identity_file: None,
        };
        let args = build_scp_args(
            &v6,
            SshOptions::default(),
            "/tmp/f",
            "/srv/f",
            TransferDirection::Upload,
            false,
        );
        assert!(
            args.contains(&"deploy@[2001:db8::1]:/srv/f".to_string()),
            "IPv6 host must be bracketed: {args:?}"
        );

        // Bare IPv6, no user.
        let anon = SshTarget {
            host: "::1".into(),
            user: None,
            port: None,
            identity_file: None,
        };
        let args = build_scp_args(
            &anon,
            SshOptions::default(),
            "/tmp/f",
            "/srv/f",
            TransferDirection::Download,
            false,
        );
        assert!(args.contains(&"[::1]:/srv/f".to_string()), "{args:?}");

        // A normal hostname is never bracketed.
        assert_eq!(
            SshTarget {
                host: "example.com".into(),
                user: None,
                port: None,
                identity_file: None,
            }
            .scp_destination(),
            "example.com"
        );
    }

    #[test]
    fn wrap_empty_cwd_is_passthrough() {
        assert_eq!(wrap_remote_command("", "ls -la"), "ls -la");
        assert_eq!(wrap_remote_command("   ", "ls"), "ls");
    }

    #[test]
    fn wrap_quotes_cwd_and_chains() {
        assert_eq!(
            wrap_remote_command("/srv/app", "ls -la"),
            "cd '/srv/app' && ls -la"
        );
        // A single quote in the path can't break out of the literal.
        assert_eq!(
            wrap_remote_command("/srv/o'brien", "pwd"),
            "cd '/srv/o'\\''brien' && pwd"
        );
    }

    #[test]
    fn probe_output_parses_prefixed_lines_amid_banner() {
        let out = "Welcome to Ubuntu\nLUXSSH_CWD=/home/deploy\nLUXSSH_SYS=Linux 6.1.0 x86_64\nLUXSSH_USR=deploy\n";
        let info = parse_probe_output(out);
        assert_eq!(info.cwd.as_deref(), Some("/home/deploy"));
        assert_eq!(info.system.as_deref(), Some("Linux 6.1.0 x86_64"));
        assert_eq!(info.user.as_deref(), Some("deploy"));
    }

    #[test]
    fn probe_output_tolerates_missing_fields() {
        let info = parse_probe_output("LUXSSH_CWD=\nrandom noise\n");
        assert_eq!(info, crate::model::ProbeInfo::default());
    }

    #[test]
    fn leading_dash_host_is_shielded_by_separator() {
        // A destination beginning with `-` must sit AFTER `--`, so ssh can never
        // mistake it for an option (e.g. `-oProxyCommand=...` injection).
        let target = SshTarget {
            host: "-oProxyCommand=evil".into(),
            user: None,
            port: None,
            identity_file: None,
        };
        let args = build_ssh_args(&target, SshOptions::default(), Some("id"));
        let dashdash = args.iter().position(|a| a == "--").unwrap();
        let dest = args
            .iter()
            .position(|a| a == "-oProxyCommand=evil")
            .unwrap();
        assert!(dashdash < dest, "leading-dash destination must follow --");
        // The remote command is the final arg and there is no second `--`.
        assert_eq!(args.last().unwrap(), "id");
        assert_eq!(args.iter().filter(|a| *a == "--").count(), 1);
    }
}
