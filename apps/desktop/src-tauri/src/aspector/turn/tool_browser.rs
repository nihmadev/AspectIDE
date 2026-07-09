use aspect_ai_core::*;

use super::approval::require_tool_approval;

pub async fn execute_browser_tool(
    app: &tauri::AppHandle,
    _state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let action = &tc.name["Browser".len()..];
    if let Err(e) = crate::aspector::tools::browser_tool::require_browser_enabled(input, action) {
        return Err(format!("Browser tool {action}: {e}"));
    }
    require_tool_approval(
        app, turn_id, tc, "always", interactive,
        &format!("Browser{action}"), &format!("Browser {action}"),
        &serde_json::to_string(&args).unwrap_or_default().chars().take(400).collect::<String>(),
        "execute", &[], "browser", false,
    ).await?;
    crate::aspector::tools::browser_tool::browser_tool_action(input, action, args).await
}
