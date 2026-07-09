use aspect_ai_core::*;

pub fn require_browser_enabled(input: &TurnInput, _action: &str) -> Result<(), String> {
    if input.agent_browser_enabled {
        Ok(())
    } else {
        Err("Browser tools are not enabled for this session. Enable the browser agent first.".to_string())
    }
}

pub async fn browser_tool_action(input: &TurnInput, action: &str, args: serde_json::Value) -> Result<String, String> {
    let cmd_args = aspect_ai_core::browser::build_browser_args(action, &args);
    let root = std::path::Path::new(&input.prompt_input.workspace_root);
    let normalized = cmd_args.iter().map(|a| {
        aspect_ai_core::browser::normalize_screenshot_path(a, root)
    }).collect::<Vec<_>>();
    Ok(serde_json::json!({ "action": action, "args": normalized }).to_string())
}
