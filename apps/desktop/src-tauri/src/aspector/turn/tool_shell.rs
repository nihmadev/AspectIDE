use aspect_ai_core::*;

use super::approval::require_tool_approval;

pub async fn execute_shell(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
    is_automatic: bool,
) -> Result<String, String> {
    let args = tc.args.clone();
    let command = json_str(&args, "command");
    let workdir = json_str_opt(&args, "workdir").unwrap_or_default();
    let effective = if is_automatic { "full-access" } else { input.tool_approval_mode.as_str() };
    require_tool_approval(
        app, turn_id, tc, effective, interactive,
        "Shell", &format!("Shell command: {command}"),
        &command.chars().take(400).collect::<String>(),
        "execute", &input.tool_permission_rules,
        &format!("shell:{}", command.chars().take(100).collect::<String>()), false,
    ).await?;
    let result = crate::aspector::tools::executors::ai_shell(
        app.clone(), state.clone(), command.to_string(),
        if workdir.is_empty() { None } else { Some(std::path::PathBuf::from(&workdir)) },
        None, None,
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_shell_output(
    _app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    _input: &TurnInput,
    _turn_id: &str,
    tc: &ParsedToolCall,
    _interactive: bool,
) -> Result<String, String> {
    let args = tc.args.clone();
    let cmd_id = json_str(&args, "cmdId");
    let stream_flag = args.get("stream").is_some();
    let (_lock, output_lines) = crate::aspector::tools::executors::shell_output(
        state.clone(), cmd_id.to_string(),
    ).await;
    if output_lines.is_empty() {
        return Ok("(empty)".to_string());
    }
    if stream_flag {
        let page_size = json_usize(&args, "pageSize", 50);
        let page = json_usize(&args, "page", 0);
        let total = output_lines.len();
        let start = page.saturating_mul(page_size);
        let chunk: Vec<&str> = output_lines.iter().map(|s| s.as_str()).skip(start).take(page_size).collect();
        let has_more = start + page_size < total;
        Ok(serde_json::json!({
            "lines": chunk, "total": total, "page": page, "hasMore": has_more,
        }).to_string())
    } else {
        Ok(output_lines.join("\n").to_string())
    }
}
