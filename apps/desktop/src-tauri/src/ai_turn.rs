//! Native AI turn loop — Stage 3 of the TS→Rust migration.
//!
//! Drives the model↔tool cycle entirely in Rust. Communicates with the React
//! frontend through Tauri events (Rust→UI) and a Tauri command for approval
//! responses (UI→Rust). The React side becomes a thin renderer + approval
//! responder.
//!
//! ## Event contract (`lux://ai-turn`)
//!
//! All events are emitted on the `lux://ai-turn` channel with a `TurnEvent`
//! payload. The frontend subscribes once and dispatches by `kind`.
//!
//! ## Approval flow
//!
//! When a tool requires approval, Rust emits `TurnEvent::ApprovalRequired` and
//! suspends the tool loop on a `tokio::sync::oneshot`. The frontend calls
//! `ai_resolve_turn_approval(turn_id, request_id, decision)` which sends the
//! decision through the channel, unblocking the loop.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

fn approval_channels() -> &'static Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>> {
    static CHANNELS: OnceLock<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>> = OnceLock::new();
    CHANNELS.get_or_init(|| Mutex::new(HashMap::new()))
}

// ── Event types (Rust → UI) ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum TurnEvent {
    /// New assistant message shell created (empty, will be patched).
    #[serde(rename_all = "camelCase")]
    AssistantCreated { turn_id: String, message_id: String },

    /// Streamed text/reasoning delta.
    #[serde(rename_all = "camelCase")]
    StreamDelta { turn_id: String, content: String, reasoning: String },

    /// Status phase changed (thinking, streaming, running-tools, waiting-approval).
    #[serde(rename_all = "camelCase")]
    StatusChange { turn_id: String, phase: String },

    /// Tool call started.
    #[serde(rename_all = "camelCase")]
    ToolCallStarted {
        turn_id: String,
        call_id: String,
        tool: String,
        input: String,
    },

    /// Tool call completed.
    #[serde(rename_all = "camelCase")]
    ToolCallCompleted {
        turn_id: String,
        call_id: String,
        status: String,
        output: String,
        error: Option<String>,
    },

    /// Approval required — UI must respond via `ai_resolve_turn_approval`.
    #[serde(rename_all = "camelCase")]
    ApprovalRequired {
        turn_id: String,
        request_id: String,
        tool: String,
        title: String,
        summary: String,
        preview: String,
        risk: String,
    },

    /// Turn completed successfully.
    #[serde(rename_all = "camelCase")]
    TurnDone {
        turn_id: String,
        message_id: String,
        content: String,
        duration_ms: u64,
    },

    /// Turn failed.
    #[serde(rename_all = "camelCase")]
    TurnError { turn_id: String, error: String },

    /// Turn was cancelled.
    #[serde(rename_all = "camelCase")]
    TurnCancelled { turn_id: String },
}

// ── Approval types (UI → Rust) ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalDecision {
    Approved,
    Rejected,
}

/// Register a pending approval and return a receiver the tool loop can await.
pub fn register_approval(turn_id: &str, request_id: &str) -> oneshot::Receiver<ApprovalDecision> {
    let (tx, rx) = oneshot::channel();
    let key = format!("{turn_id}:{request_id}");
    if let Ok(mut map) = approval_channels().lock() {
        map.insert(key, tx);
    }
    rx
}

/// Resolve a pending approval from the UI side.
#[tauri::command]
pub fn ai_resolve_turn_approval(
    turn_id: String,
    request_id: String,
    decision: ApprovalDecision,
) -> Result<(), String> {
    let key = format!("{turn_id}:{request_id}");
    let sender = approval_channels()
        .lock()
        .map_err(|_| "approval lock poisoned".to_string())?
        .remove(&key)
        .ok_or_else(|| format!("no pending approval for {key}"))?;
    sender.send(decision).map_err(|_| "approval receiver dropped".to_string())
}

/// Cancel all pending approvals for a turn (e.g. on abort).
pub fn cancel_approvals_for_turn(turn_id: &str) {
    if let Ok(mut map) = approval_channels().lock() {
        let prefix = format!("{turn_id}:");
        let keys: Vec<String> = map.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
        for key in keys {
            if let Some(sender) = map.remove(&key) {
                let _ = sender.send(ApprovalDecision::Rejected);
            }
        }
    }
}

/// Emit a turn event to the frontend.
pub fn emit_turn_event(app: &tauri::AppHandle, event: &TurnEvent) -> Result<(), String> {
    use tauri::Emitter;
    app.emit("lux://ai-turn", event).map_err(|e| e.to_string())
}

// ── Turn input ──

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInput {
    pub session_id: String,
    pub message: String,
    pub history: Vec<serde_json::Value>,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub agent_mode: String,
    pub tool_round_limit: Option<u32>,
    pub tool_approval_mode: String,
    pub prompt_input: crate::ai_prompt::SystemPromptInput,
    /// Whether agent-browser tools are enabled.
    #[serde(default)]
    pub agent_browser_enabled: bool,
}

/// Start a native AI turn. Runs the full model↔tool loop in Rust,
/// emitting `lux://ai-turn` events for the frontend to render.
#[tauri::command]
pub async fn ai_run_turn(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::SharedState>,
    input: TurnInput,
) -> Result<(), String> {
    let turn_id = uuid::Uuid::new_v4().to_string();
    let message_id = uuid::Uuid::new_v4().to_string();
    let started_at = std::time::Instant::now();
    let max_rounds = input.tool_round_limit.unwrap_or(32).min(128) as usize;

    let _ = emit_turn_event(&app, &TurnEvent::AssistantCreated {
        turn_id: turn_id.clone(),
        message_id: message_id.clone(),
    });
    let _ = emit_turn_event(&app, &TurnEvent::StatusChange {
        turn_id: turn_id.clone(),
        phase: "thinking".to_string(),
    });

    // Build system prompt natively.
    let system = crate::ai_prompt::build_system_prompt(&input.prompt_input);

    // Assemble messages array.
    let mut messages: Vec<serde_json::Value> = Vec::new();
    messages.push(serde_json::json!({ "role": "system", "content": system }));
    for entry in &input.history {
        messages.push(entry.clone());
    }
    messages.push(serde_json::json!({ "role": "user", "content": input.message }));

    // Runtime tool definitions — generated natively in Rust, filtered by mode.
    let tools = crate::ai_tool_defs::runtime_tool_definitions(
        &input.agent_mode,
        input.agent_browser_enabled,
    );

    let mut final_content = String::new();

    // ── Model ↔ tool loop ──
    for round in 0..max_rounds {
        let phase = if round == 0 { "thinking" } else { "running-tools" };
        let _ = emit_turn_event(&app, &TurnEvent::StatusChange {
            turn_id: turn_id.clone(),
            phase: phase.to_string(),
        });

        let payload = serde_json::json!({
            "model": input.model,
            "messages": messages,
            "temperature": 0.2,
            "stream": false,
            "tools": tools,
            "tool_choice": "auto",
        });

        let request = crate::ai_chat_backend::AiChatCompletionRequest::new(
            input.base_url.clone(),
            input.api_key.clone(),
            payload,
        );

        let response = match crate::ai_chat_backend::completion(request).await {
            Ok(r) => r,
            Err(error) => {
                let _ = emit_turn_event(&app, &TurnEvent::TurnError {
                    turn_id: turn_id.clone(),
                    error,
                });
                return Ok(());
            }
        };

        let assistant = parse_assistant_message(&response.body);

        // Emit any streamed content.
        if !assistant.content.is_empty() {
            let _ = emit_turn_event(&app, &TurnEvent::StreamDelta {
                turn_id: turn_id.clone(),
                content: assistant.content.clone(),
                reasoning: assistant.reasoning.clone(),
            });
            final_content = assistant.content.clone();
        }

        // No tool calls → turn is done.
        if assistant.tool_calls.is_empty() {
            break;
        }

        // Append assistant message with tool_calls to conversation.
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": if assistant.content.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(assistant.content.clone()) },
            "tool_calls": assistant.tool_calls.iter().map(|tc| serde_json::json!({
                "id": tc.id,
                "type": "function",
                "function": { "name": tc.name, "arguments": tc.arguments },
            })).collect::<Vec<_>>(),
        }));

        let _ = emit_turn_event(&app, &TurnEvent::StatusChange {
            turn_id: turn_id.clone(),
            phase: "running-tools".to_string(),
        });

        // Execute each tool call.
        for tc in &assistant.tool_calls {
            let _ = emit_turn_event(&app, &TurnEvent::ToolCallStarted {
                turn_id: turn_id.clone(),
                call_id: tc.id.clone(),
                tool: tc.name.clone(),
                input: tc.arguments.clone(),
            });

            let result = execute_tool(&app, &state, &input, &turn_id, tc).await;

            let (status, output, error) = match result {
                Ok(output) => ("success".to_string(), output, None),
                Err(err) => ("error".to_string(), String::new(), Some(err)),
            };

            let _ = emit_turn_event(&app, &TurnEvent::ToolCallCompleted {
                turn_id: turn_id.clone(),
                call_id: tc.id.clone(),
                status: status.clone(),
                output: output.clone(),
                error: error.clone(),
            });

            // Append tool result to conversation.
            messages.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": if error.is_some() {
                    serde_json::json!({ "error": error.unwrap_or_default() }).to_string()
                } else {
                    output
                },
            }));
        }
    }

    let duration_ms = started_at.elapsed().as_millis() as u64;
    if final_content.is_empty() {
        final_content = "Done.".to_string();
    }
    let _ = emit_turn_event(&app, &TurnEvent::TurnDone {
        turn_id,
        message_id,
        content: final_content,
        duration_ms,
    });

    Ok(())
}

// ── Response parsing ──

struct ParsedAssistant {
    content: String,
    reasoning: String,
    tool_calls: Vec<ParsedToolCall>,
}

struct ParsedToolCall {
    id: String,
    name: String,
    arguments: String,
}

fn parse_assistant_message(body: &serde_json::Value) -> ParsedAssistant {
    let choice = body.get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first());
    let message = choice.and_then(|c| c.get("message"));
    let content = message
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let reasoning = message
        .and_then(|m| m.get("reasoning_content").or_else(|| m.get("reasoning")))
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_string();
    let tool_calls = message
        .and_then(|m| m.get("tool_calls"))
        .and_then(|tc| tc.as_array())
        .map(|arr| arr.iter().filter_map(parse_tool_call).collect())
        .unwrap_or_default();
    ParsedAssistant { content, reasoning, tool_calls }
}

fn parse_tool_call(value: &serde_json::Value) -> Option<ParsedToolCall> {
    let id = value.get("id")?.as_str()?.to_string();
    let function = value.get("function")?;
    let name = function.get("name")?.as_str()?.to_string();
    let arguments = function.get("arguments")
        .and_then(|a| a.as_str())
        .unwrap_or("{}")
        .to_string();
    Some(ParsedToolCall { id, name, arguments })
}

// ── Tool execution ──
// Dispatches to native Rust implementations for tools that are already ported;
// remaining tools fall through to a Tauri self-invoke bridge (calls the existing
// TS tool dispatcher through IPC).

async fn execute_tool(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    turn_id: &str,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap_or(serde_json::json!({}));

    match tc.name.as_str() {
        // ── Natively ported tools (Stage 1) ──
        "SemanticSearch" => {
            let query = json_str(&args, "query");
            let path = json_str_opt(&args, "path");
            let max_results = json_usize(&args, "maxResults", 24);
            let max_files = json_usize(&args, "maxFiles", 5000);
            let result = crate::ai_semantic::ai_semantic_search(
                state.clone(), query, path, Some(max_results), Some(max_files),
            ).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "RelatedFiles" => {
            let path = json_str_opt(&args, "path");
            let query = json_str_opt(&args, "query");
            let max_results = json_usize(&args, "maxResults", 40);
            let max_files = json_usize(&args, "maxFiles", 5000);
            let result = crate::ai_related::ai_related_files(
                state.clone(), path, query, Some(max_results), Some(max_files),
            ).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "RepoMap" => {
            let max_files = json_usize(&args, "maxFiles", 80);
            let result = crate::ai_workspace::ai_repo_map(state.clone(), Some(max_files)).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "WorkspaceIndex" => {
            let max_files = json_usize(&args, "maxFiles", 60);
            let max_scan = json_usize(&args, "maxScan", 5000);
            let result = crate::ai_workspace::ai_workspace_index(state.clone(), Some(max_files), Some(max_scan)).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }

        // ── Tools already in Rust (ai_tools.rs) ──
        "Shell" => {
            let command = json_str(&args, "command");
            let cwd = json_str_opt(&args, "cwd");
            let timeout_secs = args.get("timeoutSecs").and_then(|v| v.as_u64());
            let result = crate::ai_tools::ai_shell(
                state.clone(), command, cwd.map(std::path::PathBuf::from), timeout_secs,
            ).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        // ── File read tools ──
        "Read" => {
            let path = json_str(&args, "path");
            let max_bytes = args.get("maxBytes").and_then(|v| v.as_u64());
            let result = crate::ai_tools::ai_read_file(state.clone(), std::path::PathBuf::from(path), max_bytes).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "Glob" => {
            let pattern = json_str(&args, "pattern");
            let max = json_usize(&args, "maxResults", 80);
            let result = crate::ai_tools::ai_glob(state.clone(), pattern, Some(max)).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "SymbolContext" => {
            let query = json_str_opt(&args, "query");
            let path = json_str_opt(&args, "path").map(std::path::PathBuf::from);
            let line = args.get("line").and_then(|v| v.as_u64()).map(|v| v as u32);
            let column = args.get("column").and_then(|v| v.as_u64()).map(|v| v as u32);
            let max = json_usize(&args, "maxResults", 80);
            let result = crate::ai_tools::ai_symbol_context(state.clone(), query, path, line, column, Some(max)).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }

        // ── File write tools (in Rust, with native approval flow) ──
        "Write" => {
            let path = json_str(&args, "path");
            let text = json_str(&args, "text");
            let overwrite = args.get("overwrite").and_then(|v| v.as_bool());
            let save = args.get("saveToDisk").and_then(|v| v.as_bool());
            require_tool_approval(app, turn_id, tc, &input.tool_approval_mode, "Write", &format!("Write to {path}"), &text.chars().take(400).collect::<String>(), if overwrite.unwrap_or(false) { "modify" } else { "create" }).await?;
            let result = crate::ai_tools::ai_file_write(app.clone(), state.clone(), std::path::PathBuf::from(path), text, overwrite, save).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "StrReplace" => {
            let path = json_str(&args, "path");
            let old_text = json_str(&args, "oldText");
            let new_text = json_str(&args, "newText");
            let expected = args.get("expectedReplacements").and_then(|v| v.as_u64()).map(|v| v as usize);
            let save = args.get("saveToDisk").and_then(|v| v.as_bool());
            require_tool_approval(app, turn_id, tc, &input.tool_approval_mode, "StrReplace", &format!("Replace in {path}"), &format!("-{}\n+{}", old_text.chars().take(200).collect::<String>(), new_text.chars().take(200).collect::<String>()), "modify").await?;
            let result = crate::ai_tools::ai_file_str_replace(app.clone(), state.clone(), std::path::PathBuf::from(path), old_text, new_text, expected, save).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "Delete" => {
            let path = json_str(&args, "path");
            require_tool_approval(app, turn_id, tc, &input.tool_approval_mode, "Delete", &format!("Delete {path}"), &path, "delete").await?;
            let result = crate::ai_tools::ai_file_delete(app.clone(), state.clone(), std::path::PathBuf::from(path)).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }

        "Grep" => {
            let query = json_str(&args, "query");
            let result = crate::search::search_query(
                state.clone(),
                query,
                lux_core::SearchOptions {
                    case_sensitive: args.get("caseSensitive").and_then(|v| v.as_bool()).unwrap_or(false),
                    whole_word: false,
                    use_regex: args.get("useRegex").and_then(|v| v.as_bool()).unwrap_or(false),
                    include_hidden: false,
                    include_globs: vec![],
                    exclude_globs: vec![],
                    max_results: json_usize(&args, "maxResults", 50),
                },
            ).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "GitContext" => {
            let status = crate::git::git_status(state.clone()).await?;
            serde_json::to_string(&status).map_err(|e| e.to_string())
        }
        "DiagnosticsContext" | "ReadLints" => {
            let max = json_usize(&args, "maxResults", 80);
            let diagnostics = crate::lsp::diagnostics_snapshot(state.clone())?;
            let count = diagnostics.len();
            let truncated: Vec<_> = diagnostics.into_iter().take(max).collect();
            Ok(serde_json::json!({ "count": count, "diagnostics": truncated }).to_string())
        }
        "AgentMessage" => {
            let action = json_str(&args, "action");
            if action == "read" {
                let topic = json_str_opt(&args, "topic");
                let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);
                let entries = crate::ai_a2a::ai_blackboard_read(input.session_id.clone(), topic, limit)?;
                serde_json::to_string(&serde_json::json!({ "action": "read", "messages": entries })).map_err(|e| e.to_string())
            } else {
                let content = json_str(&args, "content");
                let topic = json_str(&args, "topic");
                if topic.is_empty() || content.is_empty() {
                    return Err("AgentMessage post requires topic and content.".to_string());
                }
                let entry = crate::ai_a2a::ai_blackboard_post(input.session_id.clone(), input.agent_mode.clone(), topic, content)?;
                serde_json::to_string(&serde_json::json!({ "action": "post", "posted": entry })).map_err(|e| e.to_string())
            }
        }
        // ── Remaining tools: not yet ported → error with hint ──
        other => {
            Err(format!(
                "Tool '{other}' is not yet available in the native Rust turn loop. \
                 Use the standard TS-based AI chat for full tool coverage while the \
                 migration completes."
            ))
        }
    }
}

/// Check permission rules + mode, then prompt the UI for approval if needed.
async fn require_tool_approval(
    app: &tauri::AppHandle,
    turn_id: &str,
    tc: &ParsedToolCall,
    approval_mode: &str,
    tool: &str,
    summary: &str,
    preview: &str,
    risk: &str,
) -> Result<(), String> {
    // Full-access mode → always approved.
    if approval_mode == "full-access" {
        return Ok(());
    }
    // Emit approval request and wait for decision from UI.
    let rx = register_approval(turn_id, &tc.id);
    let _ = emit_turn_event(app, &TurnEvent::ApprovalRequired {
        turn_id: turn_id.to_string(),
        request_id: tc.id.clone(),
        tool: tool.to_string(),
        title: format!("Approve {tool}"),
        summary: summary.to_string(),
        preview: preview.to_string(),
        risk: risk.to_string(),
    });
    match rx.await {
        Ok(ApprovalDecision::Approved) => Ok(()),
        _ => Err(format!("{tool} was rejected by the user.")),
    }
}

fn json_str(value: &serde_json::Value, key: &str) -> String {
    value.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn json_str_opt(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string)
}

fn json_usize(value: &serde_json::Value, key: &str, default: usize) -> usize {
    value.get(key).and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_roundtrip() {
        let rx = register_approval("turn-1", "req-1");
        ai_resolve_turn_approval("turn-1".into(), "req-1".into(), ApprovalDecision::Approved).unwrap();
        assert_eq!(rx.blocking_recv().unwrap(), ApprovalDecision::Approved);
    }

    #[test]
    fn approval_reject() {
        let rx = register_approval("turn-2", "req-2");
        ai_resolve_turn_approval("turn-2".into(), "req-2".into(), ApprovalDecision::Rejected).unwrap();
        assert_eq!(rx.blocking_recv().unwrap(), ApprovalDecision::Rejected);
    }

    #[test]
    fn cancel_approvals_resolves_rejected() {
        let rx = register_approval("turn-3", "req-3");
        cancel_approvals_for_turn("turn-3");
        assert_eq!(rx.blocking_recv().unwrap(), ApprovalDecision::Rejected);
    }

    #[test]
    fn missing_approval_returns_error() {
        let result = ai_resolve_turn_approval("no-turn".into(), "no-req".into(), ApprovalDecision::Approved);
        assert!(result.is_err());
    }
}
