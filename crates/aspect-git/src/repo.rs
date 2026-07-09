use std::path::Path;

use aspect_core::{AppError, AppResult};

use crate::command::{git_command, run_git_capture};

pub fn repo_root(root: impl AsRef<Path>) -> AppResult<std::path::PathBuf> {
    let output =
        run_git_capture(git_command(root.as_ref()).args(["rev-parse", "--show-toplevel"]))?;
    let text = String::from_utf8_lossy(&output).trim().to_string();
    if text.is_empty() {
        return Err(AppError::Service(
            "git rev-parse --show-toplevel returned no path".to_string(),
        ));
    }
    Ok(std::path::PathBuf::from(text))
}
