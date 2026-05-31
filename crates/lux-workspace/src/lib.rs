use std::path::{Path, PathBuf};

use chrono::Utc;
use lux_core::{AppError, AppResult, WorkspaceId, WorkspaceInfo};

pub fn open_workspace(path: impl AsRef<Path>) -> AppResult<WorkspaceInfo> {
    let root = normalize_existing_directory(path.as_ref())?;
    let name = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Workspace")
        .to_string();

    Ok(WorkspaceInfo {
        id: WorkspaceId::new(),
        name,
        root,
        opened_at: Utc::now(),
    })
}

pub fn normalize_existing_directory(path: &Path) -> AppResult<PathBuf> {
    let root = path.canonicalize()?;
    if !root.is_dir() {
        return Err(AppError::InvalidPath(format!(
            "{} is not a directory",
            root.display()
        )));
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_current_directory() {
        let workspace = open_workspace(".").expect("workspace opens");
        assert!(workspace.root.is_dir());
        assert!(!workspace.name.is_empty());
    }
}
