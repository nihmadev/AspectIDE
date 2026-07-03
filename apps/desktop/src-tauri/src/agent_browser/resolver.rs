//! Binary resolution and shared configuration: locates the agent-browser CLI
//! from trusted sources only (bundled > env var > managed > PATH), centralises
//! the console-window-suppressing spawn helper, and holds the tuning constants.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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
    Managed,
    Path,
}

/// App-data directory, published once at startup (see `set_app_data_dir` in
/// `lib.rs` setup). Lets the resolver find the managed install
/// (`<app_data>/agent-browser`) and the managed Node runtime without threading
/// an `AppHandle` through every synchronous call site.
static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_app_data_dir(dir: PathBuf) {
    let _ = APP_DATA_DIR.set(dir);
}

/// `<app_data>/agent-browser` — the npm prefix the managed install targets on
/// machines without a dev checkout. `npm install --prefix` puts the CLI shim in
/// `node_modules/.bin` underneath.
pub fn managed_install_dir() -> Option<PathBuf> {
    APP_DATA_DIR.get().map(|dir| dir.join("agent-browser"))
}

/// Managed Node runtime dirs (`<app_data>/runtime/node[/bin]`). The agent-browser
/// CLI is a Node package — its launcher needs `node` resolvable. On machines where
/// Node was auto-provisioned (no system Node), every CLI spawn must see these on
/// PATH or the shim dies with "'node' is not recognized".
fn managed_node_dirs() -> Vec<PathBuf> {
    let Some(app_data) = APP_DATA_DIR.get() else {
        return Vec::new();
    };
    let node = app_data.join("runtime").join("node");
    [node.clone(), node.join("bin")]
        .into_iter()
        .filter(|dir| dir.is_dir())
        .collect()
}

/// PATH value with the managed Node runtime prepended (None when no managed
/// runtime exists — the inherited PATH is already correct then).
fn managed_node_path_env() -> Option<String> {
    let dirs = managed_node_dirs();
    if dirs.is_empty() {
        return None;
    }
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let mut parts = dirs;
    parts.extend(std::env::split_paths(&existing));
    std::env::join_paths(parts)
        .ok()
        .map(|joined| joined.to_string_lossy().to_string())
}

/// Spawns the agent-browser CLI without a visible console window on Windows.
/// Centralizes the `creation_flags` call so no spawn site can forget it, and
/// prepends the managed Node runtime to PATH so the CLI's Node launcher works
/// on machines whose only Node is the one Lux provisioned.
pub fn agent_browser_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    // `mut` is only exercised by the Windows `creation_flags` call below.
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(program);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    if let Some(path) = managed_node_path_env() {
        command.env("PATH", path);
    }
    command
}

/// Resolve the agent-browser binary path from trusted sources only.
/// Order: bundled > env var > managed > PATH. Rejects per-request overrides.
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

    // 3. Managed install (<app_data>/agent-browser) — where "Install now" lands
    //    on machines without a dev checkout.
    if let Some(path) = managed_binary() {
        return Ok((path, BinarySource::Managed));
    }

    // 4. PATH resolution.
    if let Ok(path) = which::which("agent-browser") {
        return Ok((path, BinarySource::Path));
    }

    Err(
        "agent-browser CLI is not installed. Use Settings -> Browser automation -> Install now \
         (Lux sets up Node.js automatically if needed)."
            .to_string(),
    )
}

const fn bin_shim_name() -> &'static str {
    if cfg!(windows) {
        "agent-browser.cmd"
    } else {
        "agent-browser"
    }
}

fn bundled_binary() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let desktop_dir = manifest_dir.parent()?;
    let candidate = desktop_dir
        .join("node_modules")
        .join(".bin")
        .join(bin_shim_name());
    candidate.exists().then_some(candidate)
}

fn managed_binary() -> Option<PathBuf> {
    let candidate = managed_install_dir()?
        .join("node_modules")
        .join(".bin")
        .join(bin_shim_name());
    candidate.exists().then_some(candidate)
}

pub const fn binary_source_label(source: BinarySource) -> &'static str {
    match source {
        BinarySource::Bundled => "bundled",
        BinarySource::EnvVar => "env",
        BinarySource::Managed => "managed",
        BinarySource::Path => "path",
    }
}

// ── Install location & package-manager resolution ──

/// The dev-checkout install target (`apps/desktop`). Only meaningful when the
/// app runs from a source checkout — `CARGO_MANIFEST_DIR` is a compile-time
/// path that does not exist on end-user machines, so callers must verify
/// `package.json` is actually present before installing into it.
pub fn desktop_package_dir() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = manifest_dir.parent().map(Path::to_path_buf)?;
    dir.join("package.json").is_file().then_some(dir)
}
