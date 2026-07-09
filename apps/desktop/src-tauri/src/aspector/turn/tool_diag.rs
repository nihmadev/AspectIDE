use aspect_ai_core::*;

use crate::aspector::session::store;

pub async fn execute_diagnostics_context(
    state: &tauri::State<'_, crate::SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let file = json_str_opt(&args, "file").map(std::path::PathBuf::from);
    let allow_all = args.get("allowAll").and_then(serde_json::Value::as_bool).unwrap_or(false);
    let result: serde_json::Value = crate::aspector::tools::executors::ai_diagnostics_context(
        state.clone(), file, allow_all,
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_read_lints(
    state: &tauri::State<'_, crate::SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let file = json_str_opt(&args, "file").map(std::path::PathBuf::from);
    let result: serde_json::Value = crate::aspector::tools::executors::ai_lint_context(state.clone(), file).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_test_health(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
    is_automatic: bool,
) -> Result<String, String> {
    let args = tc.args.clone();
    use super::approval::require_tool_approval;
    let effective = if is_automatic { "full-access" } else { "always" };
    require_tool_approval(
        app, turn_id, tc, effective, interactive,
        "TestHealth", "Test health", "runs test runner",
        "execute", &[], "test", false,
    ).await?;
    let _todo = store::current_todo(&input_session_id());
    let plan: Option<crate::aspector::plan::Plan> = None;
    let result: serde_json::Value = crate::aspector::tools::executors::ai_test_health(
        app.clone(), state.clone(), args, plan, _todo.as_deref(),
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

fn input_session_id() -> String {
    String::new()
}

pub async fn execute_failure_analyzer(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    _turn_id: &str,
    _interactive: bool,
    tc: &ParsedToolCall,
    _is_automatic: bool,
) -> Result<String, String> {
    let args = tc.args.clone();
    let result: serde_json::Value = crate::aspector::tools::executors::ai_failure_analyzer(
        app.clone(), state.clone(), args,
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_impact_analysis(
    state: &tauri::State<'_, crate::SharedState>,
    _input: &TurnInput,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let file_path = json_str(&args, "file");
    let result: serde_json::Value = crate::aspector::tools::executors::ai_impact_analysis(
        state.clone(), std::path::PathBuf::from(file_path),
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_review_diff(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
) -> Result<String, String> {
    crate::aspector::tools::executors::ai_review_diff(app.clone(), state.clone()).await
}

pub async fn execute_terminal_context(
    _input: &TurnInput,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let _max_lines = json_usize(&args, "maxLines", 200);
    let result = store::terminal_context("", _max_lines);
    Ok(result)
}

pub async fn execute_terminal_write(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
    is_automatic: bool,
) -> Result<String, String> {
    let args = tc.args.clone();
    let text = json_str(&args, "text");
    use super::approval::require_tool_approval;
    let effective = if is_automatic { "full-access" } else { "always" };
    require_tool_approval(
        app, turn_id, tc, effective, interactive,
        "TerminalWrite", "Write to terminal",
        &text.chars().take(200).collect::<String>(),
        "execute", &[], "terminal", false,
    ).await?;
    let result: serde_json::Value = crate::aspector::tools::executors::ai_terminal_write(
        state.clone(), text.to_string(),
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}
