//! Data types for the SSH engine: connection targets, hardening options, parsed
//! `~/.ssh/config` hosts, command results, and live session records.
//!
//! Everything here is pure data — no I/O. The desktop glue owns the actual
//! process spawning and fills these in.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How OpenSSH verifies the remote host key.
///
/// Lux deliberately never exposes the unsafe `StrictHostKeyChecking=no`/`off`
/// (which silently trusts any key and so invites MITM): the choice is only
/// between trust-on-first-use and fully strict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostKeyPolicy {
    /// Accept a brand-new host's key automatically (recorded to `known_hosts`),
    /// but refuse to connect if a *known* host's key has changed. Sensible default
    /// for an IDE: no interactive prompt, yet still catches key swaps.
    #[default]
    AcceptNew,
    /// Refuse any host whose key is not already trusted in `known_hosts`.
    Strict,
}

impl HostKeyPolicy {
    /// The value for OpenSSH's `StrictHostKeyChecking` option.
    #[must_use]
    pub const fn ssh_option_value(self) -> &'static str {
        match self {
            Self::AcceptNew => "accept-new",
            Self::Strict => "yes",
        }
    }
}

/// A resolved SSH destination.
///
/// `host` may be a plain hostname/IP or an alias defined in the user's
/// `~/.ssh/config`; OpenSSH resolves the alias itself, so any explicitly-supplied
/// `user`/`port`/`identity_file` here simply override it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTarget {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
}

impl SshTarget {
    /// The `[user@]host` destination string passed to `ssh`.
    #[must_use]
    pub fn destination(&self) -> String {
        match &self.user {
            Some(user) if !user.is_empty() => format!("{user}@{}", self.host),
            _ => self.host.clone(),
        }
    }

    /// The destination for `scp`'s `host:path` operand. `scp` splits on the FIRST
    /// colon, so a bare IPv6 literal (`2001:db8::1`) would be mis-parsed; bracket
    /// the host (`[2001:db8::1]`) per scp(1) when it looks like an unbracketed IPv6
    /// literal. Hostnames, aliases, and IPv4 pass through unchanged.
    #[must_use]
    pub fn scp_destination(&self) -> String {
        let host = if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        match &self.user {
            Some(user) if !user.is_empty() => format!("{user}@{host}"),
            _ => host,
        }
    }
}

/// Tunables applied to every spawned `ssh`/`scp` invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SshOptions {
    /// `ConnectTimeout` (seconds) — how long the TCP/auth handshake may take
    /// before OpenSSH gives up. Bounds a dead host to a fast, clean failure.
    pub connect_timeout_secs: u16,
    pub host_key_policy: HostKeyPolicy,
}

impl Default for SshOptions {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 12,
            host_key_policy: HostKeyPolicy::AcceptNew,
        }
    }
}

/// Direction of an `scp` file transfer relative to the local workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferDirection {
    /// Local → remote.
    Upload,
    /// Remote → local.
    Download,
}

/// A host entry distilled from `~/.ssh/config`, for discovery in the UI/agent.
/// Carries only non-secret routing fields (never key material or passphrases).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfigHost {
    /// The `Host` alias as written in the config (first non-wildcard pattern).
    pub alias: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
}

/// Parsed output of the connection probe (server identity + starting directory).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProbeInfo {
    /// The remote starting directory (the login/home dir), if reported.
    pub cwd: Option<String>,
    /// `uname -srm` of the remote host, if reported.
    pub system: Option<String>,
    /// The remote login user, if reported.
    pub user: Option<String>,
}

/// A live, registered SSH connection profile.
///
/// Lux drives one short-lived, non-interactive `ssh` invocation per command, so a
/// "session" is the stored destination + a sticky logical working directory —
/// there is no long-running remote process to leak, and no password is ever held.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshSession {
    pub id: Uuid,
    pub label: String,
    pub target: SshTarget,
    /// Sticky remote working directory prepended (`cd`) to each command. Empty
    /// means "run in the login default directory".
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_user: Option<String>,
    pub created_at: DateTime<Utc>,
}
