use aspect_ai_core::*;

use super::commands::emit_turn_event;

use crate::aspector::session::store;

pub fn execute_goal(
    input: &TurnInput,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let user_goal = json_str_opt(&args, "goal").unwrap_or("".to_string());
    let goal = store::get_goal(&input.session_id);
    let result = serde_json::json!({
        "goal": goal,
        "userGoal": user_goal,
        "agentMode": input.agent_mode,
        "browserEnabled": input.agent_browser_enabled,
        "workspaceRoot": input.prompt_input.workspace_root,
    });
    Ok(result.to_string())
}

pub fn execute_todo_write(
    input: &TurnInput,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let action = json_str_opt(&args, "action").unwrap_or("add".to_string());
    let content = json_str_opt(&args, "content").unwrap_or("".to_string());
    let items = args.get("items").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let items_str: Vec<String> = items.iter().filter_map(|v| v.as_str().map(String::from)).collect();
    store::update_todo(&input.session_id, &action, &content, &items_str);
    let todo = store::current_todo(&input.session_id);
    Ok(serde_json::json!({"todo": todo}).to_string())
}

pub async fn execute_ask_user(
    app: &tauri::AppHandle,
    input: &TurnInput,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let question_type = json_str_opt(&args, "type").unwrap_or("text".to_string());
    let question = json_str(&args, "question");
    let callback = json_str_opt(&args, "callback").map(|s| s.to_string());

    if !interactive {
        if let Some(cb) = &callback {
            let result = crate::aspector::tools::executors::ai_tool_call(
                app.clone(), input, turn_id, &tc.id, cb,
            ).await?;
            return Ok(result);
        }
        return Err("AskUser requires interactive mode to ask the user.".to_string());
    }

    let rx = register_approval(turn_id, &tc.id);
    if is_turn_cancelled(turn_id) {
        cancel_approvals_for_turn(turn_id);
        return Err("AskUser cancelled: the turn was stopped by the user.".to_string());
    }
    emit_turn_event(app, &TurnEvent::ApprovalRequired {
        turn_id: turn_id.to_string(),
        request_id: tc.id.clone(),
        tool: "AskUser".to_string(),
        title: "Question from AI".to_string(),
        summary: question.to_string(),
        preview: String::new(),
        risk: question_type.to_string(),
    }).ok();
    match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
        Ok(Ok(ApprovalDecision::Approved)) => Ok(serde_json::json!({"status": "answered"}).to_string()),
        Ok(Ok(ApprovalDecision::Rejected)) => Ok(serde_json::json!({"status": "rejected"}).to_string()),
        Ok(Err(_)) => Err("AskUser failed to receive approval.".to_string()),
        Err(_elapsed) => {
            cancel_approvals_for_turn(turn_id);
            Err("AskUser timed out waiting for user response.".to_string())
        }
    }
}

pub async fn execute_present_plan(
    app: &tauri::AppHandle,
    input: &TurnInput,
    turn_id: &str,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let plan = store::current_plan_raw(&input.session_id);
    emit_turn_event(app, &TurnEvent::PlanProposed {
        turn_id: turn_id.to_string(),
        plan_id: tc.id.clone(),
        title: "Plan".to_string(),
        summary: String::new(),
        steps: vec![],
        alternatives: vec![],
        risks: vec![],
        verification: vec![],
        quality: 0.0,
        coaching: vec![],
        auto_start: false,
    }).ok();
    Ok(serde_json::json!({"plan": plan}).to_string())
}

pub fn execute_active_context(
    state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let path = json_str_opt(&args, "path").map(std::path::PathBuf::from);
    let max = json_usize(&args, "maxResults", 80);
    crate::aspector::tools::executors::ai_active_context(state.clone(), &input.session_id, path, max)
}

pub async fn execute_agent_message(
    input: &TurnInput,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let action = json_str(&args, "action").trim().to_ascii_lowercase();
    let board_author = if interactive { "main".to_string() } else { turn_id.to_string() };
    match action.as_str() {
        "read" | "get" | "" => {
            let topic = json_str_opt(&args, "topic");
            let author = json_str_opt(&args, "author");
            let since_ms = args.get("sinceMs").and_then(serde_json::Value::as_i64);
            let limit = args.get("limit").and_then(serde_json::Value::as_u64).and_then(|v| usize::try_from(v).ok());
            let entries = crate::aspector::session::a2a::ai_blackboard_read(
                input.session_id.clone(), topic, limit, author, since_ms,
            )?;
            serde_json::to_string(&serde_json::json!({
                "action": "read", "you": board_author, "messages": entries,
            })).map_err(|e| e.to_string())
        }
        "post" | "write" | "send" => {
            let content = json_str(&args, "content");
            let topic = json_str(&args, "topic");
            if topic.is_empty() || content.is_empty() {
                return Err("AgentMessage post requires topic and content.".to_string());
            }
            let entry = crate::aspector::session::a2a::ai_blackboard_post(
                input.session_id.clone(), board_author, topic, content,
            )?;
            serde_json::to_string(&serde_json::json!({ "action": "post", "posted": entry }))
                .map_err(|e| e.to_string())
        }
        other => Err(format!("AgentMessage: unknown action '{other}' (use 'read' or 'post').")),
    }
}

pub fn execute_secret_guard(
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let action = json_str(&args, "action").to_lowercase();
    if action == "check" {
        let text = json_str(&args, "text");
        let (findings, _redacted) = aspect_ai_core::secrets::scan_secrets(&text, 50, true);
        Ok(serde_json::json!({ "secrets": findings }).to_string())
    } else {
        Err(format!("Unknown SecretGuard action: {action}. Use 'check'."))
    }
}
