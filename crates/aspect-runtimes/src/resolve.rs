use std::path::{Path, PathBuf};

use crate::runtime::runtime_bin_dirs;

/// On Windows, the order in which executable extensions are tried.
#[cfg(windows)]
const WINDOWS_EXE_EXTENSIONS: &[&str] = &[".com", ".exe", ".bat", ".cmd"];

/// Resolve a command on the system PATH, honoring Windows executable extensions.
pub fn resolve_on_path(command: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        for ext in WINDOWS_EXE_EXTENSIONS {
            if let Ok(path) = which::which(format!("{command}{ext}")) {
                return Some(path);
            }
        }
        which::which(command).ok()
    }
    #[cfg(not(windows))]
    {
        which::which(command).ok()
    }
}

/// Resolve `command` inside a specific directory (managed bin dir), applying
/// Windows executable extensions in native-first order.
pub fn resolve_in_dir(dir: &Path, command: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        for ext in WINDOWS_EXE_EXTENSIONS {
            let candidate = dir.join(format!("{command}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let direct = dir.join(command);
    if direct.is_file() {
        return Some(direct);
    }
    None
}

/// Resolve a host tool (npm/go/python/rustup), preferring a managed runtime
/// (`<app_data>/runtime`) over the system PATH.
pub fn resolve_tool(data_dir: &Path, tool: &str) -> Option<PathBuf> {
    for dir in runtime_bin_dirs(data_dir) {
        if let Some(path) = resolve_in_dir(&dir, tool) {
            return Some(path);
        }
    }
    resolve_on_path(tool)
}
