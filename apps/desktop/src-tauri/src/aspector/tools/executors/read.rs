use std::path::PathBuf;

use tauri::State;

use crate::{resolve_workspace_path, workspace_root, SharedState};
use aspect_agent_tools::types::{AiGlobResult, AiReadFileResult};

pub async fn ai_read_file(
    state: State<'_, SharedState>,
    path: PathBuf,
    max_bytes: Option<u64>,
    start_line: Option<u32>,
    max_lines: Option<u32>,
) -> Result<AiReadFileResult, String> {
    let path = resolve_workspace_path(&state, &path)?;
    aspect_agent_tools::file_read::ai_read_file(&path, max_bytes, start_line, max_lines).await
}

pub async fn ai_glob(
    state: State<'_, SharedState>,
    pattern: String,
    max_results: Option<usize>,
) -> Result<AiGlobResult, String> {
    let root = workspace_root(&state)?;
    aspect_agent_tools::glob_utils::ai_glob(&root, &pattern, max_results).await
}
