use std::path::PathBuf;
use tauri::State;

use aspect_ai_core::*;

use crate::SharedState;

pub async fn execute_semantic_search(
    state: &State<'_, SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let query = json_str(&args, "query");
    let max = json_usize(&args, "maxResults", 25);
    let path = json_str_opt(&args, "path");
    let max_files = args.get("maxFiles").and_then(|v| v.as_u64()).map(|n| n as usize);
    let result = crate::aspector::context::semantic::ai_semantic_search(
        state.clone(), query, path, Some(max), max_files,
    )
    .await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_related_files(
    state: &State<'_, SharedState>,
    input: &TurnInput,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let _ = input;
    let args = tc.args.clone();
    let target_path = json_str(&args, "path");
    let query = json_str_opt(&args, "query");
    let max = json_usize(&args, "maxResults", 25);
    let max_files = args.get("maxFiles").and_then(|v| v.as_u64()).map(|n| n as usize);
    let result = crate::aspector::tools::executors::ai_related_files(
        state.clone(), Some(target_path), query, Some(max), max_files,
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_repo_map(
    state: &State<'_, SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let max_files = json_usize(&args, "maxFiles", 80);
    let result = crate::aspector::tools::executors::ai_repo_map(
        state.clone(), Some(max_files),
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_workspace_index(
    state: &State<'_, SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let max = json_usize(&args, "maxResults", 60);
    let result = crate::aspector::tools::executors::ai_workspace_index(
        state.clone(), Some(max), None,
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_grep(
    state: &State<'_, SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let pattern = json_str(&args, "pattern");
    let include = json_str_opt(&args, "include");
    let path = json_str_opt(&args, "path").map(PathBuf::from);
    let max = json_usize(&args, "maxResults", 50);
    let result: serde_json::Value = crate::aspector::tools::executors::ai_grep(
        state.clone(), pattern, include, path, max,
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_git_context(
    state: &State<'_, SharedState>,
) -> Result<String, String> {
    let result: serde_json::Value = crate::aspector::tools::executors::ai_git_context(state.clone()).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}
