use std::collections::{HashMap, HashSet};

use aspect_ai_core::*;
use futures_util::StreamExt;

use super::commands::{emit_turn_event, emit_retry_event};
use super::run::emit_accumulated_usage;
use super::exec::execute_tool;

pub struct TurnLoopState {
    pub final_content: String,
    pub usage_prompt: u64,
    pub usage_completion: u64,
    pub usage_total: u64,
    pub usage_cached: u64,
    pub model_calls: u64,
    pub completed_naturally: bool,
    pub turn_output_bytes: usize,
    pub turn_tool_calls: usize,
    pub tool_budget_exceeded: bool,
    pub reasoning_fallback_used: bool,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub async fn run_turn_loop<'a>(
    app: &'a tauri::AppHandle,
    state: &'a tauri::State<'a, crate::SharedState>,
    input: &'a TurnInput,
    turn_id: &'a str,
    messages: &'a mut Vec<serde_json::Value>,
    tools: &'a [serde_json::Value],
    allowed_tool_names: &'a HashSet<String>,
    max_rounds: usize,
    mut ls: TurnLoopState,
) -> (Result<(), String>, TurnLoopState) {
    for _round in 0..max_rounds {
        if is_turn_cancelled(turn_id) {
            clear_turn_cancelled(turn_id);
            clear_injections(&input.session_id, turn_id);
            return (cancel_result(app, turn_id, &ls), ls);
        }
        let _ = emit_turn_event(
            app,
            &TurnEvent::StatusChange {
                turn_id: turn_id.to_string(),
                phase: "thinking".to_string(),
            },
        );

        let mut payload = serde_json::json!({
            "model": input.model,
            "messages": messages,
            "stream": true,
            "stream_options": { "include_usage": true },
            "tools": tools,
            "tool_choice": "auto",
        });
        crate::aspector::transport::merge_reasoning(&mut payload, input.reasoning.as_ref());
        crate::aspector::transport::apply_temperature(&mut payload, input.reasoning.as_ref(), 0.2);

        let request = crate::aspector::transport::AiChatCompletionRequest::with_protocol(
            input.base_url.clone(),
            input.api_key.clone(),
            payload,
            input.prompt_input.provider_protocol.clone(),
        );
        ls.model_calls += 1;

        let (response, error_opt, should_continue) = round_stream_call(
            app, turn_id, &input, &mut ls, request,
        ).await;

        if let Some(error) = error_opt {
            let _ = emit_turn_event(app, &TurnEvent::TurnError {
                turn_id: turn_id.to_string(),
                error,
            });
            return (Ok(()), ls);
        }
        let response = response.unwrap();

        if let Some(usage) = response.body.get("usage") {
            accumulate_usage(
                usage,
                &mut ls.usage_prompt,
                &mut ls.usage_completion,
                &mut ls.usage_total,
                &mut ls.usage_cached,
            );
        }

        let assistant = parse_assistant_message(&response.body);
        if !assistant.content.is_empty() {
            ls.final_content = assistant.content.clone();
        } else if ls.final_content.trim().is_empty() && !assistant.reasoning.trim().is_empty() {
            ls.final_content = assistant.reasoning.clone();
        }

        if is_turn_cancelled(turn_id) {
            return (cancel_result(app, turn_id, &ls), ls);
        }

        // No tool calls → end turn or inject
        if assistant.tool_calls.is_empty() {
            let injected = drain_injections(&input.session_id, turn_id);
            if injected.is_empty() {
                ls.completed_naturally = true;
                break;
            }
            if !assistant.content.is_empty() {
                let mut committed = serde_json::json!({
                    "role": "assistant",
                    "content": assistant.content.clone(),
                });
                if let Some(anthropic_content) = response.body.get("anthropic_content") {
                    committed["anthropic_content"] = anthropic_content.clone();
                }
                messages.push(committed);
            }
            for text in injected {
                let _ = emit_turn_event(app, &TurnEvent::UserMessageInjected {
                    turn_id: turn_id.to_string(),
                    text: text.clone(),
                });
                if !text.trim().is_empty() {
                    messages.push(serde_json::json!({ "role": "user", "content": text }));
                }
            }
            continue;
        }

        let mut assistant_message = serde_json::json!({
            "role": "assistant",
            "content": if assistant.content.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(assistant.content.clone()) },
            "tool_calls": assistant.tool_calls.iter().map(|tc| serde_json::json!({
                "id": tc.id,
                "type": "function",
                "function": { "name": tc.name, "arguments": tc.args },
            })).collect::<Vec<_>>(),
        });
        if let Some(anthropic_content) = response.body.get("anthropic_content") {
            assistant_message["anthropic_content"] = anthropic_content.clone();
        }
        messages.push(assistant_message);

        let _ = emit_turn_event(app, &TurnEvent::StatusChange {
            turn_id: turn_id.to_string(),
            phase: "running-tools".to_string(),
        });

        let batch_first_reads = build_batch_first_reads(state, input, &assistant);
        let parallel_ids = build_parallel_ids(&ls, &assistant);

        let mut parallel_task_results = run_parallel_tasks(
            app, state, input, turn_id, allowed_tool_names,
            &assistant, &parallel_ids,
        ).await;

        let tool_result = execute_tool_batch(
            app, state, input, turn_id,
            messages, allowed_tool_names, &assistant,
            &mut parallel_task_results, &batch_first_reads,
            &mut ls,
        ).await;

        match tool_result {
            ToolBatchResult::Cancelled => {
                return (cancel_result(app, turn_id, &ls), ls);
            }
            ToolBatchResult::BudgetExceeded => {
                ls.tool_budget_exceeded = true;
                break;
            }
            ToolBatchResult::Continue => {}
        }

        if ls.tool_budget_exceeded {
            break;
        }

        for injected in drain_injections(&input.session_id, turn_id) {
            let _ = emit_turn_event(app, &TurnEvent::UserMessageInjected {
                turn_id: turn_id.to_string(),
                text: injected.clone(),
            });
            if !injected.trim().is_empty() {
                messages.push(serde_json::json!({ "role": "user", "content": injected }));
            }
        }
    }

    (Ok(()), ls)
}
