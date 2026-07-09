use std::path::PathBuf;
use tauri::State;
use uuid::Uuid;

use crate::SharedState;

// Re-exports from aspect_agent_tools (preserved from original executors.rs)
pub use aspect_agent_tools::types::AiFilePatchOperation;

// Internal modules (private)
mod common;
mod file_write;
mod file_replace;
mod file_patch;
mod file_delete;
mod shell;
mod symbol;
mod read;

// Re-export public functions from sub-modules
pub use file_write::ai_file_write;
pub use file_replace::ai_file_str_replace;
pub use file_patch::ai_file_patch;
pub use file_delete::ai_file_delete;
pub use shell::{ai_shell, ai_shell_classify};
pub use symbol::ai_symbol_context;
pub use read::{ai_read_file, ai_glob};

// ── Redirects to context modules ──

pub use crate::aspector::context::related::ai_related_files;
pub use crate::aspector::context::workspace::{ai_repo_map, ai_workspace_index};

pub async fn ai_grep(
    state: State<'_, SharedState>,
    pattern: String,
    include: Option<String>,
_path: Option<PathBuf>,
    max_results: usize,
) -> Result<serde_json::Value, String> {
    use aspect_core::SearchOptions;
    let root = crate::workspace_root(&state)?;
    let is_narrow = include.is_some();
    let include_globs = include.map(|i| vec![i]).unwrap_or_default();
    let exclude_globs = if is_narrow {
        Vec::new()
    } else {
        vec![
            "node_modules/**".to_string(),
            "target/**".to_string(),
            "dist/**".to_string(),
            "build/**".to_string(),
            ".git/**".to_string(),
            "vendor/**".to_string(),
            "venv/**".to_string(),
            ".venv/**".to_string(),
        ]
    };
    let options = SearchOptions {
        case_sensitive: false,
        whole_word: false,
        use_regex: true,
        include_hidden: false,
        include_globs,
        exclude_globs,
        max_results,
    };
    let query = pattern;
    let result = tokio::task::spawn_blocking(move || aspect_search::query(root, query, &options))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "results": result.hits }))
}

pub async fn ai_git_context(state: State<'_, SharedState>) -> Result<serde_json::Value, String> {
    use crate::services::git::{git_status, git_diff, git_branches};
    let status = git_status(state.clone()).await?;
    let diff = git_diff(state.clone()).await?;
    let branches = git_branches(state).await?;
    Ok(serde_json::json!({
        "status": status,
        "diff": diff,
        "branches": branches,
    }))
}

pub async fn ai_diagnostics_context(
    state: State<'_, SharedState>,
    _file: Option<PathBuf>,
    _allow_all: bool,
) -> Result<serde_json::Value, String> {
    use crate::services::lsp::diagnostics_snapshot;
    let snapshot: Vec<aspect_core::WorkspaceDiagnostic> = diagnostics_snapshot(state)?;
    Ok(serde_json::json!({ "diagnostics": snapshot }))
}

pub async fn ai_lint_context(
    state: State<'_, SharedState>,
    _file: Option<PathBuf>,
) -> Result<serde_json::Value, String> {
    use crate::services::lsp::diagnostics_snapshot;
    let snapshot: Vec<aspect_core::WorkspaceDiagnostic> = diagnostics_snapshot(state)?;
    Ok(serde_json::json!({ "lints": snapshot }))
}

pub async fn ai_test_health(
    app: tauri::AppHandle,
    _state: State<'_, SharedState>,
    _args: serde_json::Value,
    _plan: Option<crate::aspector::plan::Plan>,
    _todo: Option<&str>,
) -> Result<serde_json::Value, String> {
    let _ = &app;
    Ok(serde_json::json!({ "testHealth": "not implemented", "status": "unknown" }))
}

pub async fn ai_failure_analyzer(
    app: tauri::AppHandle,
    _state: State<'_, SharedState>,
    _args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let _ = &app;
    Ok(serde_json::json!({ "analysis": "not implemented" }))
}

pub async fn ai_impact_analysis(
    _state: State<'_, SharedState>,
    _file: PathBuf,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "impact": "not implemented" }))
}

pub async fn ai_review_diff(
    _app: tauri::AppHandle,
    _state: State<'_, SharedState>,
) -> Result<String, String> {
    Ok("Diff review not implemented".to_string())
}

pub async fn ai_terminal_write(
    state: State<'_, SharedState>,
    text: String,
) -> Result<serde_json::Value, String> {
    use crate::services::terminal::terminal_write;
    let session_id = Uuid::nil();
    terminal_write(state, session_id, text)?;
    Ok(serde_json::json!({ "written": true }))
}

pub async fn ai_tool_call(
    _app: tauri::AppHandle,
    _input: &aspect_ai_core::TurnInput,
    _turn_id: &str,
    _call_id: &str,
    _callback: &str,
) -> Result<String, String> {
    Err("ai_tool_call not implemented".to_string())
}

pub fn ai_active_context(
    state: State<'_, SharedState>,
    _session_id: &str,
    _path: Option<PathBuf>,
    _max: usize,
) -> Result<String, String> {
    let _ = &state;
    Ok(serde_json::json!({ "files": [] }).to_string())
}

pub async fn shell_output(
    _state: State<'_, SharedState>,
    _cmd_id: String,
) -> (String, Vec<String>) {
    (String::new(), Vec::new())
}
