#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

//! `aspect-ssh` вЂ” the pure core of AspectIDE's first-class SSH integration.
//!
//! The IDE drives the system OpenSSH client (`ssh`/`scp`, present out of the box
//! on Windows 10+/macOS/Linux) rather than reimplementing the protocol, so it
//! transparently honors the user's existing `~/.ssh/config`, keys, `ssh-agent`,
//! and `known_hosts`. This crate owns the I/O-free parts so they can be fully
//! unit-tested:
//!
//! - [`build_ssh_args`] / [`build_scp_args`]: hardened, **non-interactive**
//!   argument vectors вЂ” the fix for agents that hang on host-key/password
//!   prompts when they shell out to raw `ssh`.
//! - [`parse_ssh_config`]: read-only host discovery from `~/.ssh/config`.
//! - [`wrap_remote_command`]: sticky remote working directory.
//! - [`SshRegistry`]: the in-memory table of live connection profiles.
//!
//! The desktop glue (`ssh.rs`) owns process spawning, timeouts, the safety
//! classifier, secret redaction, and the Tauri command surface.

mod args;
mod config;
mod model;

pub use args::{
    build_scp_args, build_ssh_args, parse_probe_output, posix_single_quote, probe_command,
    wrap_remote_command,
};
pub use config::parse_ssh_config;
pub use model::{
    HostKeyPolicy, ProbeInfo, SshConfigHost, SshOptions, SshSession, SshTarget, TransferDirection,
};

use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
};

use chrono::Utc;
use uuid::Uuid;

/// Thread-safe table of live SSH connection profiles, keyed by session id.
///
/// A "session" is just a stored destination plus a sticky logical working
/// directory вЂ” AspectIDE runs one short-lived `ssh` process per command, so there is no
/// long-running remote process or held credential to leak when a session is
/// dropped.
#[derive(Debug, Default)]
pub struct SshRegistry {
    sessions: Mutex<HashMap<Uuid, SshSession>>,
}

impl SshRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A poisoned registry lock must not take down SSH for the rest of the
    /// session, so recover the guard rather than propagating the panic.
    fn guard(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, SshSession>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Register a new session and return a clone of the stored record.
    pub fn insert(
        &self,
        label: String,
        target: SshTarget,
        cwd: String,
        system: Option<String>,
        remote_user: Option<String>,
    ) -> SshSession {
        let session = SshSession {
            id: Uuid::new_v4(),
            label,
            target,
            cwd,
            system,
            remote_user,
            created_at: Utc::now(),
        };
        self.guard().insert(session.id, session.clone());
        session
    }

    #[must_use]
    pub fn get(&self, id: Uuid) -> Option<SshSession> {
        self.guard().get(&id).cloned()
    }

    /// All sessions, oldest first (stable order for listing).
    #[must_use]
    pub fn list(&self) -> Vec<SshSession> {
        let mut sessions: Vec<SshSession> = self.guard().values().cloned().collect();
        sessions.sort_by_key(|session| session.created_at);
        sessions
    }

    #[must_use]
    pub fn count(&self) -> usize {
        self.guard().len()
    }

    /// Remove one session; returns whether it existed.
    pub fn remove(&self, id: Uuid) -> bool {
        self.guard().remove(&id).is_some()
    }

    /// Remove every session; returns how many were dropped.
    pub fn clear(&self) -> usize {
        let mut guard = self.guard();
        let count = guard.len();
        guard.clear();
        count
    }

    /// Update a session's sticky working directory; returns whether it existed.
    pub fn set_cwd(&self, id: Uuid, cwd: String) -> bool {
        if let Some(session) = self.guard().get_mut(&id) {
            session.cwd = cwd;
            true
        } else {
            false
        }
    }
}

