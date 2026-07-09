use std::path::PathBuf;
use std::sync::OnceLock;

use tokio::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub const DEFAULT_TIMEOUT_SECS: u64 = 90;
pub const MAX_TIMEOUT_SECS: u64 = 180;
pub const DEFAULT_MAX_OUTPUT: usize = 50_000;
pub const MAX_OUTPUT_CAP: usize = 120_000;
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const INSTALL_TIMEOUT_SECS: u64 = 600;
pub const READ_VERSION_TIMEOUT_SECS: u64 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinarySource {
    Bundled,
    EnvVar,
    Managed,
    Path,
}

static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static DESKTOP_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_app_data_dir(dir: PathBuf) {
    let _ = APP_DATA_DIR.set(dir);
}

pub fn set_desktop_dir(dir: PathBuf) {
    let _ = DESKTOP_DIR.set(dir);
}

pub fn managed_install_dir() -> Option<PathBuf> {
    APP_DATA_DIR.get().map(|dir| dir.join("agent-browser"))
}

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

pub fn agent_browser_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(program);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    if let Some(path) = managed_node_path_env() {
        command.env("PATH", path);
    }
    command
}

pub fn resolve_binary() -> Result<PathBuf, String> {
    resolve_binary_with_source().map(|(path, _)| path)
}

pub fn resolve_binary_with_source() -> Result<(PathBuf, BinarySource), String> {
    if let Some(path) = bundled_binary() {
        return Ok((path, BinarySource::Bundled));
    }

    if let Ok(path) =
        std::env::var("AGENT_BROWSER_PATH").or_else(|_| std::env::var("ASPECT_AGENT_BROWSER_COMMAND"))
    {
        let candidate = PathBuf::from(path.trim());
        if candidate.exists() {
            return Ok((candidate, BinarySource::EnvVar));
        }
    }

    if let Some(path) = managed_binary() {
        return Ok((path, BinarySource::Managed));
    }

    if let Ok(path) = which::which("agent-browser") {
        return Ok((path, BinarySource::Path));
    }

    Err(
        "agent-browser CLI is not installed. Use Settings -> Browser automation -> Install now \
         (AspectIDE sets up Node.js automatically if needed)."
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
    let desktop_dir = DESKTOP_DIR.get()?;
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

pub fn desktop_package_dir() -> Option<PathBuf> {
    let dir = DESKTOP_DIR.get()?;
    dir.join("package.json").is_file().then_some(dir.clone())
}
