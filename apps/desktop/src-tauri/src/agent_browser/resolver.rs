//! Binary resolution and shared configuration: locates the agent-browser CLI
//! from trusted sources only (bundled > env var > PATH), centralises the
//! console-window-suppressing spawn helper, and holds the tuning constants.

use std::path::{Path, PathBuf};

use tokio::process::Command;

/// Windows `CREATE_NO_WINDOW` — prevents a console window from flashing when the
/// agent-browser CLI is spawned from the GUI app.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub const DEFAULT_TIMEOUT_SECS: u64 = 90;
pub const MAX_TIMEOUT_SECS: u64 = 180;
pub const DEFAULT_MAX_OUTPUT: usize = 50_000;
pub const MAX_OUTPUT_CAP: usize = 120_000;
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const INSTALL_TIMEOUT_SECS: u64 = 600;
pub const READ_VERSION_TIMEOUT_SECS: u64 = 15;

// ── Binary source provenance ──
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinarySource {
    Bundled,
    EnvVar,
    Path,
}

/// Spawns the agent-browser CLI without a visible console window on Windows.
/// Centralizes the `creation_flags` call so no spawn site can forget it.
pub fn agent_browser_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    // `mut` is only exercised by the Windows `creation_flags` call below.
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(program);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// Resolve the agent-browser binary path from trusted sources only.
/// Order: bundled > env var > PATH. Rejects per-request overrides.
pub fn resolve_binary() -> Result<PathBuf, String> {
    resolve_binary_with_source().map(|(path, _)| path)
}

pub fn resolve_binary_with_source() -> Result<(PathBuf, BinarySource), String> {
    // 1. Bundled binary (node_modules/.bin/agent-browser) — highest precedence.
    if let Some(path) = bundled_binary() {
        return Ok((path, BinarySource::Bundled));
    }

    // 2. Environment variables — user-configured at the OS/process level.
    if let Ok(path) =
        std::env::var("AGENT_BROWSER_PATH").or_else(|_| std::env::var("LUX_AGENT_BROWSER_COMMAND"))
    {
        let candidate = PathBuf::from(path.trim());
        if candidate.exists() {
            return Ok((candidate, BinarySource::EnvVar));
        }
    }

    // 3. PATH resolution.
    if let Ok(path) = which::which("agent-browser") {
        return Ok((path, BinarySource::Path));
    }

    Err(
        "agent-browser CLI is not installed. Use Settings -> Browser automation -> Install now, \
         or run `pnpm add agent-browser` in apps/desktop."
            .to_string(),
    )
}

fn bundled_binary() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let desktop_dir = manifest_dir.parent()?;
    let bin_name = if cfg!(windows) {
        "agent-browser.cmd"
    } else {
        "agent-browser"
    };
    let candidate = desktop_dir.join("node_modules").join(".bin").join(bin_name);
    candidate.exists().then_some(candidate)
}

pub fn binary_source_label(source: BinarySource) -> &'static str {
    match source {
        BinarySource::Bundled => "bundled",
        BinarySource::EnvVar => "env",
        BinarySource::Path => "path",
    }
}

// ── Install location & package-manager resolution ──

pub fn desktop_package_dir() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().map(Path::to_path_buf)
}

pub fn resolve_package_manager() -> Result<(PathBuf, Vec<String>), String> {
    if let Ok(path) = which::which("pnpm") {
        return Ok((
            path,
            vec!["add".to_string(), "agent-browser@latest".to_string()],
        ));
    }
    let npm = resolve_npm()?;
    Ok((
        npm,
        vec!["install".to_string(), "agent-browser@latest".to_string()],
    ))
}

fn resolve_npm() -> Result<PathBuf, String> {
    if cfg!(windows) {
        if let Ok(path) = which::which("npm.cmd") {
            return Ok(path);
        }
    }
    which::which("npm").map_err(|_| {
        "npm was not found on PATH. Install Node.js 24+ before installing agent-browser."
            .to_string()
    })
}
