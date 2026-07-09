use std::pin::Pin;

use aspect_ai_core::*;

use super::commands::emit_turn_event;
use super::exec::execute_tool;

#[allow(dead_code)]
/// Boxed, explicitly-`Send` wrapper around [`run_subagent`]. Background Task
/// spawning creates a recursive async chain, and rustc cannot infer `Send`
/// through that cycle of opaque futures.
#[allow(clippy::too_many_arguments)]
pub fn run_subagent_boxed<'a>(
    app: &'a tauri::AppHandle,
    state: &'a tauri::State<'a, crate::SharedState>,
    parent: &'a TurnInput,
    parent_turn_id: &'a str,
    call_id: &'a str,
    agent_id: &'a str,
    description: &'a str,
    prompt: &'a str,
    subagent_type: &'a str,
    model_override: Option<&'a str>,
) -> Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>> {
    Box::pin(run_subagent(
        app, state, parent, parent_turn_id, call_id, agent_id,
        description, prompt, subagent_type, model_override,
    ))
}

/// Run an isolated subagent turn (Task tool). The subagent gets its own model↔tool
/// loop with a capped round limit and read-only-leaning tools, then returns a concise
/// summary to the parent. Shares the session's A2A blackboard for coordination.
#[allow(clippy::too_many_arguments)]
pub async fn run_subagent(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    parent: &TurnInput,
    parent_turn_id: &str,
    call_id: &str,
    agent_id: &str,
    description: &str,
    prompt: &str,
    subagent_type: &str,
    model_override: Option<&str>,
) -> Result<String, String> {
    let max_rounds: usize = if parent.agent_mode == "automatic" { 24 } else { 16 };
    let read_only = matches!(subagent_type, "codeReviewer" | "explorer");
    let subagent_model: &str = model_override
        .map(str::trim).filter(|m| !m.is_empty())
        .unwrap_or(parent.model.as_str());

    let instructions = format!(
        "You are Aspect subagent '{agent_id}' ({subagent_type}). Task: {description}\n\
         Work in an isolated context: the parent agent sees ONLY your final message, so make \
         it a complete, self-contained report (findings, file paths, evidence, next steps). \
         Shared board: AgentMessage action=read shows what the main agent and sibling \
         subagents posted (filter with topic/author/sinceMs); AgentMessage action=post \
         publishes discoveries other agents need under a clear topic — you post as '{agent_id}', \
         and the parent is told which topics you posted to. Do not spawn further subagents."
    );
    let mut prompt_input = parent.prompt_input.clone();
    prompt_input.agent_instructions = instructions.clone();
    prompt_input.agent_name = format!("subagent:{subagent_type}");
    if read_only {
        prompt_input.agent_mode = "ask".to_string();
    }
    let system = crate::aspector::context::prompt::build_system_prompt(&prompt_input);

    let mut messages: Vec<serde_json::Value> = vec![
        build_system_message(&system, parent.anthropic_cache),
        serde_json::json!({ "role": "user", "content": prompt }),
    ];
    let mut tools = crate::aspector::tools::definitions::runtime_tool_definitions(
        if read_only { "ask" } else { &parent.agent_mode },
        parent.agent_browser_enabled,
    );
    const MAIN_AGENT_ONLY_TOOLS: [&str; 5] =
        ["Task", "TaskWait", "Goal", "TodoWrite", "PresentPlan"];
    tools.retain(|t| {
        let name = t.get("function").and_then(|f| f.get("name"))
            .or_else(|| t.get("name")).and_then(|n| n.as_str()).unwrap_or_default();
        !MAIN_AGENT_ONLY_TOOLS.contains(&name)
    });
    let subagent_allowed: std::collections::HashSet<String> = tools.iter()
        .filter_map(|t| t.get("function").and_then(|f| f.get("name"))
            .or_else(|| t.get("name")).and_then(|n| n.as_str()).map(str::to_string))
        .collect();

    let emit_progress = |stage: &str, content: String, tool: String| {
        let _ = emit_turn_event(app, &TurnEvent::SubagentProgress {
            turn_id: parent_turn_id.to_string(),
            call_id: call_id.to_string(),
            agent_id: agent_id.to_string(),
            stage: stage.to_string(),
            content, tool,
        });
    };

    let mut final_content = String::new();
    for _round in 0..max_rounds {
        if is_turn_cancelled(parent_turn_id) || is_subagent_cancelled(call_id) {
            clear_subagent_cancelled(call_id);
            let summary = if final_content.is_empty() { "Subagent cancelled.".to_string() } else { clamp_subagent_summary(final_content) };
            emit_progress("cancelled", summary.clone(), String::new());
            return Ok(summary);
        }
        let mut payload = serde_json::json!({
            "model": subagent_model, "messages": messages, "stream": true,
            "stream_options": { "include_usage": true }, "tools": tools, "tool_choice": "auto",
        });
        crate::aspector::transport::merge_reasoning(&mut payload, parent.reasoning.as_ref());
        crate::aspector::transport::apply_temperature(&mut payload, parent.reasoning.as_ref(), 0.2);
        let request = crate::aspector::transport::AiChatCompletionRequest::with_protocol(
            parent.base_url.clone(), parent.api_key.clone(), payload,
            parent.prompt_input.provider_protocol.clone(),
        );
        let cancel_turn = parent_turn_id.to_string();
        let cancel_call = call_id.to_string();
        let mut round_text = String::new();
        let mut last_progress = std::time::Instant::now();
        let mut emitted_len = 0usize;
        let response = crate::aspector::transport::completion_streaming(
            request,
            |content, _reasoning| {
                if content.is_empty() { return; }
                round_text.push_str(content);
                if last_progress.elapsed().as_millis() >= 300 && round_text.len() > emitted_len {
                    emitted_len = round_text.len();
                    last_progress = std::time::Instant::now();
                    emit_progress("text", round_text.clone(), String::new());
                }
            },
            move || is_turn_cancelled(&cancel_turn) || is_subagent_cancelled(&cancel_call),
            |_notice| {},
            |_tool_name| {},
        ).await.inspect_err(|_| { clear_subagent_cancelled(call_id); })?;
        if is_subagent_cancelled(call_id) {
            clear_subagent_cancelled(call_id);
            let summary = if final_content.is_empty() { "Subagent cancelled.".to_string() } else { clamp_subagent_summary(final_content) };
            emit_progress("cancelled", summary.clone(), String::new());
            return Ok(summary);
        }
        if round_text.len() > emitted_len {
            emit_progress("text", round_text.clone(), String::new());
        }
        let assistant = parse_assistant_message(&response.body);
        if !assistant.content.is_empty() { final_content = assistant.content.clone(); }
        if assistant.tool_calls.is_empty() { break; }
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": if assistant.content.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(assistant.content.clone()) },
            "tool_calls": assistant.tool_calls.iter().map(|tc| serde_json::json!({
                "id": tc.id, "type": "function",
                "function": { "name": tc.name, "arguments": tc.args },
            })).collect::<Vec<_>>(),
        }));
        for child in &assistant.tool_calls {
            let preview: String = child.args.to_string().chars().take(160).collect();
            emit_progress("tool", preview, child.name.clone());
            let result = if matches!(child.name.as_str(), "Task" | "TaskWait") {
                Err("Nested subagents are not allowed (depth limit).".to_string())
            } else if matches!(child.name.as_str(), "Goal" | "TodoWrite" | "PresentPlan") {
                Err(format!("{} is reserved for the main agent.", child.name))
            } else {
                Box::pin(execute_tool(app, state, parent, agent_id, false, child, &subagent_allowed)).await
            };
            let content = match result {
                Ok(output) => output,
                Err(err) => serde_json::json!({ "error": err }).to_string(),
            };
            messages.push(serde_json::json!({
                "role": "tool", "tool_call_id": child.id, "content": content,
            }));
            if is_turn_cancelled(parent_turn_id) || is_subagent_cancelled(call_id) {
                clear_subagent_cancelled(call_id);
                let summary = if final_content.is_empty() { "Subagent cancelled.".to_string() } else { clamp_subagent_summary(final_content) };
                emit_progress("cancelled", summary.clone(), String::new());
                return Ok(summary);
            }
        }
    }
    clear_subagent_cancelled(call_id);
    let summary = if final_content.is_empty() { "Subagent finished without a summary.".to_string() } else { clamp_subagent_summary(final_content) };
    emit_progress("done", summary.clone(), String::new());
    Ok(summary)
}
