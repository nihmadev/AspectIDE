use aspect_ai_core::*;

use super::commands::{emit_retry_event, emit_turn_event};
use super::exec::execute_tool;
use super::helpers::is_empty_message_content;
use super::run_rec::run_recovery_synthesis;

#[allow(clippy::too_many_lines)]
pub async fn ai_run_turn_inner(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    input: TurnInput,
) -> Result<(), String> {
    let turn_id = input.turn_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let message_id = input.message_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let started_at = std::time::Instant::now();
    let _live_turn = LiveTurnGuard::register(&input.session_id, &turn_id);
    let max_rounds = input.tool_round_limit.unwrap_or(32).clamp(1, 128) as usize;

    let _ = emit_turn_event(app, &TurnEvent::AssistantCreated {
        turn_id: turn_id.clone(),
        message_id: message_id.clone(),
    });
    let _ = emit_turn_event(app, &TurnEvent::StatusChange {
        turn_id: turn_id.clone(),
        phase: "thinking".to_string(),
    });

    let system = crate::aspector::context::prompt::build_system_prompt(&input.prompt_input);
    let mut messages: Vec<serde_json::Value> = Vec::new();
    messages.push(build_system_message(&system, input.anthropic_cache));
    for entry in &input.history {
        messages.push(entry.clone());
    }
    let user_content = input.user_content.clone()
        .filter(|v| !matches!(v, serde_json::Value::Null))
        .unwrap_or_else(|| serde_json::Value::String(input.message.clone()));
    if !is_empty_message_content(&user_content) {
        messages.push(serde_json::json!({ "role": "user", "content": user_content }));
    }

    let mut tools = crate::aspector::tools::definitions::runtime_tool_definitions(
        &input.agent_mode, input.agent_browser_enabled,
    );
    crate::aspector::tools::definitions::annotate_task_model_options(&mut tools, &input.available_model_ids);
    if matches!(input.agent_mode.as_str(), "agent" | "automatic") {
        tools.extend(crate::network::mcp::agent_tool_definitions().await);
    }
    let allowed_tool_names = tool_names_from_defs(&tools);

    let mut completed_naturally = false;
    let mut turn_output_bytes: usize = 0;
    let mut turn_tool_calls: usize = 0;
    let mut tool_budget_exceeded = false;
    let mut usage_prompt: u64 = 0;
    let mut usage_completion: u64 = 0;
    let mut usage_total: u64 = 0;
    let mut usage_cached: u64 = 0;
    let mut model_calls: u64 = 0;
    let mut final_content = String::new();

    crate::aspector::session::store::clear_read_files(&input.session_id);

    for _round in 0..max_rounds {
        if is_turn_cancelled(&turn_id) {
            clear_turn_cancelled(&turn_id);
            clear_injections(&input.session_id, &turn_id);
            let _cancelled = emit_turn_event(app, &TurnEvent::TurnDone {
                turn_id: turn_id.clone(), message_id: message_id.clone(),
                content: "Turn cancelled.".to_string(),
                duration_ms: u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
            });
            return Ok(());
        }

        let _ = emit_turn_event(app, &TurnEvent::StatusChange {
            turn_id: turn_id.clone(), phase: "thinking".to_string(),
        });

        let mut payload = serde_json::json!({
            "model": input.model, "messages": messages, "stream": true,
            "stream_options": { "include_usage": true },
            "tools": tools, "tool_choice": "auto",
        });
        crate::aspector::transport::merge_reasoning(&mut payload, input.reasoning.as_ref());
        crate::aspector::transport::apply_temperature(&mut payload, input.reasoning.as_ref(), 0.2);

        let request = crate::aspector::transport::AiChatCompletionRequest::with_protocol(
            input.base_url.clone(), input.api_key.clone(), payload,
            input.prompt_input.provider_protocol.clone(),
        );
        model_calls += 1;

        let turn_id_clone = turn_id.clone();
        let _session_id = input.session_id.clone();
        let response = crate::aspector::transport::completion_streaming(
            request,
            |content, reasoning| {
                if !content.is_empty() || !reasoning.is_empty() {
                    let _ = emit_turn_event(&app, &TurnEvent::StreamDelta {
                        turn_id: turn_id_clone.clone(),
                        content: content.to_string(),
                        reasoning: reasoning.to_string(),
                    });
                }
            },
            || is_turn_cancelled(&turn_id_clone),
            |notice| emit_retry_event(app, &turn_id, &notice),
            |tool_name| {
                let _ = emit_turn_event(&app, &TurnEvent::ToolCallStarted {
                    turn_id: turn_id_clone.clone(),
                    call_id: String::new(),
                    tool: tool_name.to_string(),
                    input: String::new(),
                });
            },
        ).await;

        match response {
            Ok(resp) => {
                if let Some(usage) = resp.body.get("usage") {
                    accumulate_usage(usage, &mut usage_prompt, &mut usage_completion, &mut usage_total, &mut usage_cached);
                }
                let assistant = parse_assistant_message(&resp.body);
                if !assistant.content.is_empty() {
                    final_content = assistant.content.clone();
                } else if final_content.trim().is_empty() && !assistant.reasoning.trim().is_empty() {
                    final_content = assistant.reasoning.clone();
                }
                if is_turn_cancelled(&turn_id) {
                    return Ok(());
                }
                if assistant.tool_calls.is_empty() {
                    let injected = drain_injections(&input.session_id, &turn_id);
                    if injected.is_empty() {
                        completed_naturally = true;
                        break;
                    }
                    if !assistant.content.is_empty() {
                        messages.push(serde_json::json!({
                            "role": "assistant", "content": assistant.content.clone(),
                        }));
                    }
                    for text in injected {
                        let _ = emit_turn_event(app, &TurnEvent::UserMessageInjected {
                            turn_id: turn_id.clone(), text: text.clone(),
                        });
                        if !text.trim().is_empty() {
                            messages.push(serde_json::json!({ "role": "user", "content": text }));
                        }
                    }
                    continue;
                }
                messages.push(serde_json::json!({
                    "role": "assistant",
                    "content": if assistant.content.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(assistant.content.clone()) },
                    "tool_calls": assistant.tool_calls.iter().map(|tc| serde_json::json!({
                        "id": tc.id, "type": "function",
                        "function": { "name": tc.name, "arguments": tc.args },
                    })).collect::<Vec<_>>(),
                }));
                if let Some(anthropic_content) = resp.body.get("anthropic_content") {
                    if let Some(last) = messages.last_mut() {
                        last["anthropic_content"] = anthropic_content.clone();
                    }
                }
                let _ = emit_turn_event(app, &TurnEvent::StatusChange {
                    turn_id: turn_id.clone(), phase: "running-tools".to_string(),
                });
                // Execute each tool call
                for tc in &assistant.tool_calls {
                    if turn_tool_calls >= 200 {
                        tool_budget_exceeded = true;
                        break;
                    }
                    if is_turn_cancelled(&turn_id) {
                        break;
                    }
                    let result = execute_tool(app, state, &input, &turn_id, true, tc, &allowed_tool_names).await;
                    let (status, output, error) = match result {
                        Ok(output) => ("success".to_string(), output, None),
                        Err(err) => ("error".to_string(), String::new(), Some(err)),
                    };
                    let _ = emit_turn_event(app, &TurnEvent::ToolCallCompleted {
                        turn_id: turn_id.clone(),
                        call_id: tc.id.clone(),
                        status: status.clone(),
                        output: output.clone(),
                        error: error.clone(),
                    });
                    const TOOL_OUTPUT_CHAR_LIMIT: usize = 32_000;
                    let content_for_messages = if error.is_some() {
                        serde_json::json!({ "error": error.clone().unwrap_or_default() }).to_string()
                    } else if output.chars().count() > TOOL_OUTPUT_CHAR_LIMIT {
                        let truncated: String = output.chars().take(TOOL_OUTPUT_CHAR_LIMIT).collect();
                        format!("{truncated}\n\n[Tool output truncated: {} chars total, showing first {TOOL_OUTPUT_CHAR_LIMIT}.]",
                            output.chars().count())
                    } else {
                        output
                    };
                    let byte_len = content_for_messages.len();
                    messages.push(serde_json::json!({
                        "role": "tool", "tool_call_id": tc.id,
                        "content": content_for_messages,
                    }));
                    turn_output_bytes += byte_len;
                    turn_tool_calls += 1;
                    if turn_tool_calls >= 200 || turn_output_bytes > 600_000 {
                        tool_budget_exceeded = true;
                        break;
                    }
                }
                if tool_budget_exceeded || is_turn_cancelled(&turn_id) {
                    break;
                }
                for injected in drain_injections(&input.session_id, &turn_id) {
                    let _ = emit_turn_event(app, &TurnEvent::UserMessageInjected {
                        turn_id: turn_id.clone(), text: injected.clone(),
                    });
                    if !injected.trim().is_empty() {
                        messages.push(serde_json::json!({ "role": "user", "content": injected }));
                    }
                }
            }
            Err(e) => {
                let _ = emit_turn_event(app, &TurnEvent::TurnError {
                    turn_id: turn_id.clone(), error: e,
                });
                return Ok(());
            }
        }
    }

    if is_turn_cancelled(&turn_id) {
        clear_turn_cancelled(&turn_id);
        clear_injections(&input.session_id, &turn_id);
        return Ok(());
    }

    clear_turn_cancelled(&turn_id);
    clear_injections(&input.session_id, &turn_id);
    emit_accumulated_usage(app, &turn_id, usage_prompt, usage_completion, usage_total, usage_cached, model_calls);

    let recovery = if !completed_naturally || tool_budget_exceeded {
        run_recovery_synthesis(app, state, &input, &turn_id, &mut messages, &tools, &mut final_content, usage_prompt, usage_completion, usage_total, usage_cached, model_calls).await
    } else {
        None
    };

    let duration_ms = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
    let content = recovery.unwrap_or_else(|| {
        if final_content.trim().is_empty() {
            "The turn produced no answer. Press **Retry** or rephrase your request.".to_string()
        } else {
            final_content.clone()
        }
    });

    emit_accumulated_usage(app, &turn_id, usage_prompt, usage_completion, usage_total, usage_cached, model_calls);
    clear_turn_cancelled(&turn_id);
    clear_injections(&input.session_id, &turn_id);
    let _ = emit_turn_event(app, &TurnEvent::TurnDone {
        turn_id, message_id, content, duration_ms,
    });
    Ok(())
}

pub fn emit_accumulated_usage(
    app: &tauri::AppHandle,
    turn_id: &str,
    usage_prompt: u64,
    usage_completion: u64,
    usage_total: u64,
    usage_cached: u64,
    model_calls: u64,
) {
    if usage_prompt == 0 && usage_completion == 0 && usage_total == 0 && model_calls == 0 {
        return;
    }
    let _ = emit_turn_event(app, &TurnEvent::TurnUsage {
        turn_id: turn_id.to_string(),
        prompt_tokens: usage_prompt,
        completion_tokens: usage_completion,
        total_tokens: if usage_total > 0 { usage_total } else { usage_prompt + usage_completion },
        cached_prompt_tokens: usage_cached,
        model_calls,
    });
}
