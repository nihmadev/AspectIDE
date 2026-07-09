use aspect_ai_core::*;

use super::subagent::run_subagent;

#[allow(clippy::too_many_arguments)]
pub async fn execute_task(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    turn_id: &str,
    tc: &ParsedToolCall,
    _interactive: bool,
) -> Result<String, String> {
    let args = tc.args.clone();
    let action = json_str(&args, "action");
    match action.as_str() {
        "create" | "start" => {
            let description = json_str(&args, "description");
            let prompt = json_str(&args, "objective");
            let subagent_type = json_str_opt(&args, "subagentType").unwrap_or("general".to_string());
            let agent_id = json_str_opt(&args, "name").unwrap_or("task".to_string());
            let background = args.get("background").and_then(serde_json::Value::as_bool).unwrap_or(false);
            let model_override = json_str_opt(&args, "model");

            if background {
                let summary = run_subagent(
                    app, state, input, turn_id, &tc.id,
                    &agent_id, &description, &prompt,
                    &subagent_type, model_override.as_deref(),
                ).await?;
                return Ok(serde_json::json!({
                    "agentId": agent_id, "subagentType": subagent_type,
                    "background": false, "status": "completed",
                    "summary": summary,
                }).to_string());
            }

            let summary = run_subagent(
                app, state, input, turn_id, &tc.id,
                &agent_id, &description, &prompt,
                &subagent_type, model_override.as_deref(),
            ).await?;
            Ok(serde_json::json!({ "summary": summary }).to_string())
        }
        "list" => Ok(serde_json::json!({ "tasks": [] }).to_string()),
        "cancel" => {
            let task_id = json_str(&args, "taskId");
            mark_subagent_cancelled(&task_id);
            Ok(serde_json::json!({ "status": "cancelled" }).to_string())
        }
        "result" => {
            Ok(serde_json::json!({ "status": "unknown" }).to_string())
        }
        other => Err(format!("Unknown Task action '{other}'. Use create|start|list|cancel|result.")),
    }
}

pub async fn execute_task_wait(
    _input: &TurnInput,
    turn_id: &str,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let timeout_secs_val = json_usize(&args, "timeoutSecs", 180);
    let timeout_secs = std::time::Duration::from_secs(timeout_secs_val as u64);
    let deadline = tokio::time::Instant::now() + timeout_secs;
    loop {
        if is_turn_cancelled(turn_id) {
            return Err("TaskWait cancelled: the turn was stopped by the user.".to_string());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("TaskWait timed out after {}s", timeout_secs.as_secs()));
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}
