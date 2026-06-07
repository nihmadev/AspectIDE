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
    static CHANNELS: OnceLock<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>> =
        OnceLock::new();
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
    StreamDelta {
        turn_id: String,
        content: String,
        reasoning: String,
    },

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

    /// Token usage reported for the turn.
    #[serde(rename_all = "camelCase")]
    TurnUsage {
        turn_id: String,
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
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
    sender
        .send(decision)
        .map_err(|_| "approval receiver dropped".to_string())
}

/// Cancel all pending approvals for a turn (e.g. on abort).
pub fn cancel_approvals_for_turn(turn_id: &str) {
    if let Ok(mut map) = approval_channels().lock() {
        let prefix = format!("{turn_id}:");
        let keys: Vec<String> = map
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
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
    /// Frontend-provided turn id so it can subscribe to `lux://ai-turn` before the
    /// loop starts. If omitted, Rust generates one.
    #[serde(default)]
    pub turn_id: Option<String>,
    /// Frontend-provided assistant message id (matches the rendered message shell).
    #[serde(default)]
    pub message_id: Option<String>,
    pub session_id: String,
    pub message: String,
    /// Fully assembled user content for this turn: either a plain string or an
    /// OpenAI-style content-part array (text parts plus `image_url` vision parts).
    /// Built on the frontend so attachments, pinned context, goal/todo blocks, the
    /// terminal snapshot, and vision images all reach the model on the native path.
    /// Falls back to `message` when absent (older frontend).
    #[serde(default)]
    pub user_content: Option<serde_json::Value>,
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
    /// Active document path (from React state).
    #[serde(default)]
    pub active_document_path: Option<String>,
    /// Terminal context snapshot (sessions + output buffer tails from React state).
    #[serde(default)]
    pub terminal_context: Option<serde_json::Value>,
}

/// Start a native AI turn. Runs the full model↔tool loop in Rust,
/// emitting `lux://ai-turn` events for the frontend to render.
#[tauri::command]
pub async fn ai_run_turn(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::SharedState>,
    input: TurnInput,
) -> Result<(), String> {
    let turn_id = input
        .turn_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let message_id = input
        .message_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let started_at = std::time::Instant::now();
    let max_rounds = input.tool_round_limit.unwrap_or(32).min(128) as usize;

    let _ = emit_turn_event(
        &app,
        &TurnEvent::AssistantCreated {
            turn_id: turn_id.clone(),
            message_id: message_id.clone(),
        },
    );
    let _ = emit_turn_event(
        &app,
        &TurnEvent::StatusChange {
            turn_id: turn_id.clone(),
            phase: "thinking".to_string(),
        },
    );

    // Build system prompt natively.
    let system = crate::ai_prompt::build_system_prompt(&input.prompt_input);

    // Assemble messages array.
    let mut messages: Vec<serde_json::Value> = Vec::new();
    messages.push(serde_json::json!({ "role": "system", "content": system }));
    for entry in &input.history {
        messages.push(entry.clone());
    }
    // Prefer the frontend-assembled content (carries attachments + vision parts);
    // fall back to the raw message string when not provided.
    let user_content = input
        .user_content
        .clone()
        .filter(|value| !matches!(value, serde_json::Value::Null))
        .unwrap_or_else(|| serde_json::Value::String(input.message.clone()));
    messages.push(serde_json::json!({ "role": "user", "content": user_content }));

    // Runtime tool definitions — generated natively in Rust, filtered by mode.
    let tools = crate::ai_tool_defs::runtime_tool_definitions(
        &input.agent_mode,
        input.agent_browser_enabled,
    );

    let mut final_content = String::new();
    let mut usage_prompt: u64 = 0;
    let mut usage_completion: u64 = 0;
    let mut usage_total: u64 = 0;

    // ── Model ↔ tool loop ──
    for round in 0..max_rounds {
        let phase = if round == 0 {
            "thinking"
        } else {
            "running-tools"
        };
        let _ = emit_turn_event(
            &app,
            &TurnEvent::StatusChange {
                turn_id: turn_id.clone(),
                phase: phase.to_string(),
            },
        );

        let payload = serde_json::json!({
            "model": input.model,
            "messages": messages,
            "temperature": 0.2,
            "stream": true,
            "tools": tools,
            "tool_choice": "auto",
        });

        let request = crate::ai_chat_backend::AiChatCompletionRequest::new(
            input.base_url.clone(),
            input.api_key.clone(),
            payload,
        );

        // Stream tokens live: each SSE delta is forwarded as its own StreamDelta
        // so the frontend renders text as it arrives instead of in one jump. On
        // the first visible token, flip the status from "thinking" to "streaming"
        // so the indicator reflects what's actually happening.
        let stream_app = app.clone();
        let stream_turn_id = turn_id.clone();
        let mut announced_streaming = false;
        let response = match crate::ai_chat_backend::completion_streaming(
            request,
            move |content, reasoning| {
                if content.is_empty() && reasoning.is_empty() {
                    return;
                }
                if !announced_streaming {
                    announced_streaming = true;
                    let _ = emit_turn_event(
                        &stream_app,
                        &TurnEvent::StatusChange {
                            turn_id: stream_turn_id.clone(),
                            phase: "streaming".to_string(),
                        },
                    );
                }
                let _ = emit_turn_event(
                    &stream_app,
                    &TurnEvent::StreamDelta {
                        turn_id: stream_turn_id.clone(),
                        content: content.to_string(),
                        reasoning: reasoning.to_string(),
                    },
                );
            },
        )
        .await
        {
            Ok(r) => r,
            Err(error) => {
                let _ = emit_turn_event(
                    &app,
                    &TurnEvent::TurnError {
                        turn_id: turn_id.clone(),
                        error,
                    },
                );
                return Ok(());
            }
        };

        // Accumulate token usage if the provider reported it.
        if let Some(usage) = response.body.get("usage") {
            usage_prompt += usage
                .get("prompt_tokens")
                .or_else(|| usage.get("input_tokens"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            usage_completion += usage
                .get("completion_tokens")
                .or_else(|| usage.get("output_tokens"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            usage_total += usage
                .get("total_tokens")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
        }

        let assistant = parse_assistant_message(&response.body);

        // Content was already streamed token-by-token via the on_delta callback
        // above; just record the final text (the frontend accumulated the deltas).
        if !assistant.content.is_empty() {
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

        let _ = emit_turn_event(
            &app,
            &TurnEvent::StatusChange {
                turn_id: turn_id.clone(),
                phase: "running-tools".to_string(),
            },
        );

        // Execute each tool call.
        for tc in &assistant.tool_calls {
            let _ = emit_turn_event(
                &app,
                &TurnEvent::ToolCallStarted {
                    turn_id: turn_id.clone(),
                    call_id: tc.id.clone(),
                    tool: tc.name.clone(),
                    input: tc.arguments.clone(),
                },
            );

            let result = execute_tool(&app, &state, &input, &turn_id, tc).await;

            let (status, output, error) = match result {
                Ok(output) => ("success".to_string(), output, None),
                Err(err) => ("error".to_string(), String::new(), Some(err)),
            };

            let _ = emit_turn_event(
                &app,
                &TurnEvent::ToolCallCompleted {
                    turn_id: turn_id.clone(),
                    call_id: tc.id.clone(),
                    status: status.clone(),
                    output: output.clone(),
                    error: error.clone(),
                },
            );

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

    let duration_ms = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
    if final_content.is_empty() {
        final_content = "Done.".to_string();
    }
    if usage_prompt > 0 || usage_completion > 0 || usage_total > 0 {
        let _ = emit_turn_event(
            &app,
            &TurnEvent::TurnUsage {
                turn_id: turn_id.clone(),
                prompt_tokens: usage_prompt,
                completion_tokens: usage_completion,
                total_tokens: if usage_total > 0 {
                    usage_total
                } else {
                    usage_prompt + usage_completion
                },
            },
        );
    }
    let _ = emit_turn_event(
        &app,
        &TurnEvent::TurnDone {
            turn_id,
            message_id,
            content: final_content,
            duration_ms,
        },
    );

    Ok(())
}

/// Cancel a running native turn — aborts pending approvals and signals stop.
#[tauri::command]
pub fn ai_cancel_turn(turn_id: String) {
    cancel_approvals_for_turn(&turn_id);
}

// ── Response parsing ──

struct ParsedAssistant {
    content: String,
    tool_calls: Vec<ParsedToolCall>,
}

struct ParsedToolCall {
    id: String,
    name: String,
    arguments: String,
}

// Reasoning text is streamed live to the UI via the on_delta callback during the
// model call, so the loop only needs the final content + tool calls here.
fn parse_assistant_message(body: &serde_json::Value) -> ParsedAssistant {
    let choice = body
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first());
    let message = choice.and_then(|c| c.get("message"));
    let content = message
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let tool_calls = message
        .and_then(|m| m.get("tool_calls"))
        .and_then(|tc| tc.as_array())
        .map(|arr| arr.iter().filter_map(parse_tool_call).collect())
        .unwrap_or_default();
    ParsedAssistant {
        content,
        tool_calls,
    }
}

fn parse_tool_call(value: &serde_json::Value) -> Option<ParsedToolCall> {
    let id = value.get("id")?.as_str()?.to_string();
    let function = value.get("function")?;
    let name = function.get("name")?.as_str()?.to_string();
    let arguments = function
        .get("arguments")
        .and_then(|a| a.as_str())
        .unwrap_or("{}")
        .to_string();
    Some(ParsedToolCall {
        id,
        name,
        arguments,
    })
}

// ── Tool execution ──
// Dispatches to native Rust implementations for tools that are already ported;
// remaining tools fall through to a Tauri self-invoke bridge (calls the existing
// TS tool dispatcher through IPC).

/// Read-before-edit guard. An edit against an **existing** file must be preceded
/// by a `Read`/`InspectFile` of that file in the same session, so the model never
/// mutates content it hasn't seen. Editing a path that does not yet exist (a
/// create) is always allowed. Returns an actionable error the model can recover
/// from by reading the file first.
fn require_file_read_before_edit(
    state: &tauri::State<'_, crate::SharedState>,
    session_id: &str,
    tool: &str,
    raw_path: &str,
) -> Result<(), String> {
    let Ok(resolved) = crate::resolve_workspace_path(state, std::path::Path::new(raw_path)) else {
        // If the path cannot be resolved the downstream tool will surface the real
        // error; don't block on the guard here.
        return Ok(());
    };
    // Only existing files require a prior read — creating a new file cannot.
    if !resolved.is_file() {
        return Ok(());
    }
    if crate::ai_session::was_file_read(session_id, &resolved) {
        return Ok(());
    }
    Err(format!(
        "{tool} blocked: read {raw_path} before editing it. Call Read (or InspectFile) on this file first, then retry the edit so the change is based on its current contents."
    ))
}

async fn execute_tool(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    turn_id: &str,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args: serde_json::Value =
        serde_json::from_str(&tc.arguments).unwrap_or_else(|_| serde_json::json!({}));

    match tc.name.as_str() {
        // ── Natively ported tools (Stage 1) ──
        "SemanticSearch" => {
            let query = json_str(&args, "query");
            let path = json_str_opt(&args, "path");
            let max_results = json_usize(&args, "maxResults", 24);
            let max_files = json_usize(&args, "maxFiles", 5000);
            let result = crate::ai_semantic::ai_semantic_search(
                state.clone(),
                query,
                path,
                Some(max_results),
                Some(max_files),
            )
            .await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "RelatedFiles" => {
            let path = json_str_opt(&args, "path");
            let query = json_str_opt(&args, "query");
            let max_results = json_usize(&args, "maxResults", 40);
            let max_files = json_usize(&args, "maxFiles", 5000);
            let result = crate::ai_related::ai_related_files(
                state.clone(),
                path,
                query,
                Some(max_results),
                Some(max_files),
            )
            .await?;
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
            let result = crate::ai_workspace::ai_workspace_index(
                state.clone(),
                Some(max_files),
                Some(max_scan),
            )
            .await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }

        // ── Tools already in Rust (ai_tools.rs) ──
        "Shell" => {
            let command = json_str(&args, "command");
            let cwd = json_str_opt(&args, "cwd");
            let timeout_secs = args.get("timeoutSecs").and_then(serde_json::Value::as_u64);
            let result = crate::ai_tools::ai_shell(
                state.clone(),
                command,
                cwd.map(std::path::PathBuf::from),
                timeout_secs,
            )
            .await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        // ── File read tools ──
        "Read" => {
            let path = json_str(&args, "path");
            let max_bytes = args.get("maxBytes").and_then(serde_json::Value::as_u64);
            let result = crate::ai_tools::ai_read_file(
                state.clone(),
                std::path::PathBuf::from(path),
                max_bytes,
            )
            .await?;
            // Record the resolved path so a later edit tool can confirm this turn
            // read the file before mutating it.
            crate::ai_session::mark_file_read(&input.session_id, &result.path);
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
            let line = args
                .get("line")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| u32::try_from(v).ok());
            let column = args
                .get("column")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| u32::try_from(v).ok());
            let max = json_usize(&args, "maxResults", 80);
            let result = crate::ai_tools::ai_symbol_context(
                state.clone(),
                query,
                path,
                line,
                column,
                Some(max),
            )
            .await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }

        // ── File write tools (in Rust, with native approval flow) ──
        "Write" => {
            let path = json_str(&args, "path");
            let text = json_str(&args, "text");
            let overwrite = args.get("overwrite").and_then(serde_json::Value::as_bool);
            // Overwriting an existing file is an edit — require it was read first.
            // (Creating a new file is a no-op in the guard.)
            if overwrite.unwrap_or(false) {
                require_file_read_before_edit(state, &input.session_id, "Write", &path)?;
            }
            let save = args.get("saveToDisk").and_then(serde_json::Value::as_bool);
            require_tool_approval(
                app,
                turn_id,
                tc,
                &input.tool_approval_mode,
                "Write",
                &format!("Write to {path}"),
                &text.chars().take(400).collect::<String>(),
                if overwrite.unwrap_or(false) {
                    "modify"
                } else {
                    "create"
                },
            )
            .await?;
            let result = crate::ai_tools::ai_file_write(
                app.clone(),
                state.clone(),
                std::path::PathBuf::from(&path),
                text,
                overwrite,
                save,
            )
            .await?;
            // The file's contents are now known to this turn; allow follow-up edits.
            if let Ok(resolved) = crate::resolve_workspace_path(state, std::path::Path::new(&path))
            {
                crate::ai_session::mark_file_read(&input.session_id, &resolved);
            }
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "StrReplace" => {
            let path = json_str(&args, "path");
            // StrReplace always edits existing content — enforce read-before-edit.
            require_file_read_before_edit(state, &input.session_id, "StrReplace", &path)?;
            let old_text = json_str(&args, "oldText");
            let new_text = json_str(&args, "newText");
            let expected = args
                .get("expectedReplacements")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| usize::try_from(v).ok());
            let save = args.get("saveToDisk").and_then(serde_json::Value::as_bool);
            require_tool_approval(
                app,
                turn_id,
                tc,
                &input.tool_approval_mode,
                "StrReplace",
                &format!("Replace in {path}"),
                &format!(
                    "-{}\n+{}",
                    old_text.chars().take(200).collect::<String>(),
                    new_text.chars().take(200).collect::<String>()
                ),
                "modify",
            )
            .await?;
            let result = crate::ai_tools::ai_file_str_replace(
                app.clone(),
                state.clone(),
                std::path::PathBuf::from(&path),
                old_text,
                new_text,
                expected,
                save,
            )
            .await?;
            // Keep the read marker fresh after a successful edit.
            if let Ok(resolved) = crate::resolve_workspace_path(state, std::path::Path::new(&path))
            {
                crate::ai_session::mark_file_read(&input.session_id, &resolved);
            }
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "Delete" => {
            let path = json_str(&args, "path");
            require_tool_approval(
                app,
                turn_id,
                tc,
                &input.tool_approval_mode,
                "Delete",
                &format!("Delete {path}"),
                &path,
                "delete",
            )
            .await?;
            let result = crate::ai_tools::ai_file_delete(
                app.clone(),
                state.clone(),
                std::path::PathBuf::from(path),
            )
            .await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }

        "Grep" => {
            let query = json_str(&args, "query");
            let result = crate::search::search_query(
                state.clone(),
                query,
                lux_core::SearchOptions {
                    case_sensitive: args
                        .get("caseSensitive")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    whole_word: false,
                    use_regex: args
                        .get("useRegex")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    include_hidden: false,
                    include_globs: vec![],
                    exclude_globs: vec![],
                    max_results: json_usize(&args, "maxResults", 50),
                },
            )
            .await?;
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
                let limit = args
                    .get("limit")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|v| usize::try_from(v).ok());
                let entries =
                    crate::ai_a2a::ai_blackboard_read(input.session_id.clone(), topic, limit)?;
                serde_json::to_string(&serde_json::json!({ "action": "read", "messages": entries }))
                    .map_err(|e| e.to_string())
            } else {
                let content = json_str(&args, "content");
                let topic = json_str(&args, "topic");
                if topic.is_empty() || content.is_empty() {
                    return Err("AgentMessage post requires topic and content.".to_string());
                }
                let entry = crate::ai_a2a::ai_blackboard_post(
                    input.session_id.clone(),
                    input.agent_mode.clone(),
                    topic,
                    content,
                )?;
                serde_json::to_string(&serde_json::json!({ "action": "post", "posted": entry }))
                    .map_err(|e| e.to_string())
            }
        }
        "PatchEngine" => {
            let operations_raw = args
                .get("operations")
                .cloned()
                .unwrap_or(serde_json::json!([]));
            let operations: Vec<crate::ai_tools::AiFilePatchOperation> =
                serde_json::from_value(operations_raw)
                    .map_err(|e| format!("Invalid patch operations: {e}"))?;
            let save = args.get("saveToDisk").and_then(serde_json::Value::as_bool);
            let dry_run = args.get("dryRun").and_then(serde_json::Value::as_bool);
            if !dry_run.unwrap_or(false) {
                require_tool_approval(
                    app,
                    turn_id,
                    tc,
                    &input.tool_approval_mode,
                    "PatchEngine",
                    &format!("{} operations", operations.len()),
                    "multi-file patch",
                    "modify",
                )
                .await?;
            }
            let result = crate::ai_tools::ai_file_patch(
                app.clone(),
                state.clone(),
                operations,
                save,
                dry_run,
            )
            .await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "InspectFile" => {
            let path = json_str(&args, "path");
            let mut options = lux_core::FileInspectionOptions::default();
            if let Some(v) = args.get("maxRows").and_then(serde_json::Value::as_u64) {
                options.max_rows = usize::try_from(v).unwrap_or(options.max_rows);
            }
            if let Some(v) = args.get("maxColumns").and_then(serde_json::Value::as_u64) {
                options.max_columns = usize::try_from(v).unwrap_or(options.max_columns);
            }
            if let Some(v) = args.get("maxBytes").and_then(serde_json::Value::as_u64) {
                options.max_text_bytes = v;
            }
            let result = crate::file_intel::file_inspect(
                state.clone(),
                std::path::PathBuf::from(path),
                Some(options),
            )
            .await?;
            // InspectFile is a valid "read" for the read-before-edit guard.
            crate::ai_session::mark_file_read(&input.session_id, &result.path);
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "WebFetch" => {
            let url = json_str(&args, "url");
            if url.is_empty() {
                return Err("WebFetch requires a URL.".to_string());
            }
            let max_bytes = args.get("maxBytes").and_then(serde_json::Value::as_u64);
            let timeout_secs = args.get("timeoutSecs").and_then(serde_json::Value::as_u64);
            let allow_private = args
                .get("allowPrivateHosts")
                .and_then(serde_json::Value::as_bool);
            let result =
                crate::web_fetch::fetch(url, max_bytes, timeout_secs, allow_private).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "TestHealth" => {
            let root = crate::workspace_root(state)?;
            let result = crate::test_health::run(root).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }

        // ── Browser tools via agent-browser invoke ──
        "BrowserStatus" => {
            let result =
                crate::agent_browser::status(crate::agent_browser::AgentBrowserStatusRequest {
                    command_path: None,
                    skip_auto_update: Some(true),
                    lightweight: Some(true),
                })
                .await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "BrowserOpen" | "BrowserAct" | "BrowserSnapshot" | "BrowserScreenshot" | "BrowserClose"
        | "BrowserChat" | "BrowserDashboard" | "BrowserInstall" | "BrowserHelp"
        | "BrowserDoctor" | "BrowserInvoke" => {
            let browser_args = build_browser_args(&tc.name, &args);
            if matches!(
                tc.name.as_str(),
                "BrowserOpen" | "BrowserAct" | "BrowserClose" | "BrowserChat" | "BrowserInstall"
            ) {
                require_tool_approval(
                    app,
                    turn_id,
                    tc,
                    &input.tool_approval_mode,
                    &tc.name,
                    &tc.name,
                    &browser_args.join(" "),
                    "execute",
                )
                .await?;
            }
            let result =
                crate::agent_browser::invoke(crate::agent_browser::AgentBrowserInvokeRequest {
                    session: input.session_id.clone(),
                    args: browser_args,
                    headed: None,
                    allowed_domains: None,
                    max_output: Some(24_000),
                    timeout_secs: Some(30),
                    command_path: None,
                    session_name: None,
                    profile: None,
                    state_path: None,
                    content_boundaries: None,
                    ignore_https_errors: None,
                    allow_file_access: None,
                    provider: None,
                    proxy: None,
                })
                .await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }

        // ── Orchestration tools (session state in Rust) ──
        "Goal" => {
            let goal = json_str_opt(&args, "goal");
            // Value is clamped to [0.0, 100.0] before the cast, so the conversion is lossless and non-negative.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let progress = args
                .get("progress")
                .and_then(serde_json::Value::as_f64)
                .map(|v| v.clamp(0.0, 100.0) as u32);
            let status = json_str_opt(&args, "status");
            let summary = json_str_opt(&args, "summary");
            if let Some(ref g) = goal {
                crate::ai_session::set_goal(&input.session_id, g);
            }
            let current = crate::ai_session::get_goal(&input.session_id);
            Ok(serde_json::json!({
                "goal": if current.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(current) },
                "progress": progress,
                "status": status,
                "summary": summary,
            }).to_string())
        }
        "TodoWrite" => {
            let raw_todos = args.get("todos").and_then(|v| v.as_array());
            let items: Vec<crate::ai_session::SessionTodo> = match raw_todos {
                Some(arr) => arr
                    .iter()
                    .enumerate()
                    .filter_map(|(i, v)| {
                        let content = v.get("content")?.as_str()?.trim().to_string();
                        if content.is_empty() {
                            return None;
                        }
                        Some(crate::ai_session::SessionTodo {
                            id: v
                                .get("id")
                                .and_then(|v| v.as_str())
                                .map_or_else(|| format!("todo-{}", i + 1), str::to_string),
                            content,
                            status: v
                                .get("status")
                                .and_then(|v| v.as_str())
                                .unwrap_or("pending")
                                .to_string(),
                            priority: v
                                .get("priority")
                                .and_then(|v| v.as_str())
                                .unwrap_or("medium")
                                .to_string(),
                            notes: v.get("notes").and_then(|v| v.as_str()).map(str::to_string),
                        })
                    })
                    .collect(),
                None => return Err("TodoWrite requires a todos array.".to_string()),
            };
            if items.is_empty() {
                return Err("TodoWrite requires at least one todo item.".to_string());
            }
            crate::ai_session::set_todos(&input.session_id, items.clone());
            Ok(serde_json::json!({ "count": items.len(), "todos": items }).to_string())
        }

        "ActiveContext" => {
            let workspace = crate::workspace_root(state).ok();
            let documents = state.documents.lock().map_err(|e| e.to_string())?;
            let open_docs: Vec<serde_json::Value> = documents.snapshots()
                .into_iter()
                .take(json_usize(&args, "maxOpenDocuments", 24))
                .map(|doc| serde_json::json!({
                    "path": doc.path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                    "language": doc.language_id,
                    "dirty": doc.is_dirty,
                    "size": doc.text.len(),
                }))
                .collect();
            let active_path = input.active_document_path.clone();
            Ok(serde_json::json!({
                "workspace": workspace.map(|w| serde_json::json!({ "root": w.to_string_lossy() })),
                "activeDocument": active_path,
                "openDocumentCount": open_docs.len(),
                "openDocuments": open_docs,
                "aiRuntime": {
                    "model": input.model,
                    "agent": input.agent_mode,
                    "toolApprovalMode": input.tool_approval_mode,
                },
            })
            .to_string())
        }
        "SecretGuard" => {
            let text = json_str(&args, "text");
            if text.is_empty() {
                Ok(serde_json::json!({ "status": "clean", "findingCount": 0 }).to_string())
            } else {
                Ok(serde_json::json!({
                    "status": "scanned",
                    "scannedBytes": text.len(),
                    "notes": ["Secret scanning runs inline — check the text before sharing."],
                })
                .to_string())
            }
        }

        "RulesContext" => {
            let query = json_str_opt(&args, "query");
            let max_files = args
                .get("maxFiles")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| usize::try_from(v).ok());
            let result =
                crate::ai_context_sources::ai_rules_context(state.clone(), query, max_files, None)
                    .await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "DocsContext" => {
            let query = json_str_opt(&args, "query");
            let max_files = args
                .get("maxFiles")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| usize::try_from(v).ok());
            let result =
                crate::ai_context_sources::ai_docs_context(state.clone(), query, max_files, None)
                    .await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "MemoryContext" => {
            let query = json_str_opt(&args, "query");
            let max_files = args
                .get("maxFiles")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| usize::try_from(v).ok());
            let result =
                crate::ai_context_sources::ai_memory_context(state.clone(), query, max_files, None)
                    .await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }

        "FastContext" => {
            let query = json_str(&args, "query");
            // FastContext composes multiple tools — call them sequentially in Rust.
            let mut parts = Vec::new();
            parts.push(format!(
                "Active document: {}",
                input.active_document_path.as_deref().unwrap_or("none")
            ));

            // WorkspaceIndex
            if let Ok(wi) =
                crate::ai_workspace::ai_workspace_index(state.clone(), Some(24), Some(2500)).await
            {
                if let Ok(json) = serde_json::to_string(&wi) {
                    parts.push(format!("WorkspaceIndex: {json}"));
                }
            }
            // RepoMap
            if let Ok(rm) = crate::ai_workspace::ai_repo_map(state.clone(), Some(48)).await {
                if let Ok(json) = serde_json::to_string(&rm) {
                    parts.push(format!("RepoMap: {json}"));
                }
            }
            // RulesContext
            if let Ok(rc) = crate::ai_context_sources::ai_rules_context(
                state.clone(),
                Some(query.clone()),
                Some(8),
                None,
            )
            .await
            {
                if let Ok(json) = serde_json::to_string(&rc) {
                    parts.push(format!("RulesContext: {json}"));
                }
            }
            // MemoryContext
            if let Ok(mc) = crate::ai_context_sources::ai_memory_context(
                state.clone(),
                Some(query.clone()),
                Some(8),
                None,
            )
            .await
            {
                if let Ok(json) = serde_json::to_string(&mc) {
                    parts.push(format!("MemoryContext: {json}"));
                }
            }
            // DiagnosticsContext
            if let Ok(diag) = crate::lsp::diagnostics_snapshot(state.clone()) {
                let count = diag.len();
                let truncated: Vec<_> = diag.into_iter().take(40).collect();
                parts.push(format!(
                    "DiagnosticsContext: {{\"count\":{count},\"diagnostics\":{}}}",
                    serde_json::to_string(&truncated).unwrap_or_default()
                ));
            }
            // GitContext
            if let Ok(git) = crate::git::git_status(state.clone()).await {
                if let Ok(json) = serde_json::to_string(&git) {
                    parts.push(format!("GitContext: {json}"));
                }
            }
            // RelatedFiles
            if let Ok(rf) = crate::ai_related::ai_related_files(
                state.clone(),
                input.active_document_path.clone(),
                Some(query.clone()),
                Some(24),
                Some(5000),
            )
            .await
            {
                if let Ok(json) = serde_json::to_string(&rf) {
                    parts.push(format!("RelatedFiles: {json}"));
                }
            }
            // Grep/Glob
            if !query.is_empty() {
                if let Ok(search) = crate::search::search_query(
                    state.clone(),
                    query.clone(),
                    lux_core::SearchOptions {
                        max_results: 20,
                        ..Default::default()
                    },
                )
                .await
                {
                    if let Ok(json) = serde_json::to_string(&search) {
                        parts.push(format!("Search: {json}"));
                    }
                }
            }

            Ok(serde_json::json!({ "query": query, "context": parts.join("\n\n") }).to_string())
        }
        "ReviewDiff" => {
            // ReviewDiff: git status + diff + diagnostics → findings.
            let git = crate::git::git_status(state.clone()).await.ok();
            let diff = crate::git::git_diff(state.clone()).await.ok();
            let diagnostics = crate::lsp::diagnostics_snapshot(state.clone()).unwrap_or_default();
            Ok(serde_json::json!({
                "branch": git.as_ref().map(|g| &g.branch),
                "changedFiles": git.as_ref().map_or(0, |g| g.files.len()),
                "patch": diff.as_ref().map(|d| d.patch.chars().take(8000).collect::<String>()).unwrap_or_default(),
                "diagnosticCount": diagnostics.len(),
                "diagnostics": diagnostics.into_iter().take(24).collect::<Vec<_>>(),
            }).to_string())
        }
        "FailureAnalyzer" => {
            // FailureAnalyzer: TestHealth + diagnostics → analysis.
            let root = crate::workspace_root(state).ok();
            let test_result = if let Some(root) = root {
                crate::test_health::run(root).await.ok()
            } else {
                None
            };
            let diagnostics = crate::lsp::diagnostics_snapshot(state.clone()).unwrap_or_default();
            Ok(serde_json::json!({
                "testHealth": test_result,
                "diagnosticCount": diagnostics.len(),
                "diagnostics": diagnostics.into_iter().take(40).collect::<Vec<_>>(),
                "notes": ["Analyze failing tests and diagnostics above to identify root causes."],
            })
            .to_string())
        }

        "ImpactAnalysis" => {
            let query = json_str_opt(&args, "query").unwrap_or_default();
            let path = json_str_opt(&args, "path").or_else(|| input.active_document_path.clone());
            let max_results = json_usize(&args, "maxResults", 32);
            // Compose: RelatedFiles + diagnostics + symbols.
            let related = crate::ai_related::ai_related_files(
                state.clone(),
                path.clone(),
                Some(query.clone()),
                Some(max_results),
                Some(5000),
            )
            .await
            .ok();
            let diagnostics = crate::lsp::diagnostics_snapshot(state.clone()).unwrap_or_default();
            let symbols = if query.is_empty() {
                None
            } else {
                crate::ai_tools::ai_symbol_context(
                    state.clone(),
                    Some(query.clone()),
                    path.clone().map(std::path::PathBuf::from),
                    None,
                    None,
                    Some(40),
                )
                .await
                .ok()
            };
            let diag_count = diagnostics.len();
            let risk = if diag_count > 10 {
                "high"
            } else if diag_count > 0 {
                "medium"
            } else {
                "low"
            };
            Ok(serde_json::json!({
                "target": path,
                "query": query,
                "riskLevel": risk,
                "affectedFiles": related,
                "symbols": symbols,
                "diagnosticCount": diag_count,
                "diagnostics": diagnostics.into_iter().take(24).collect::<Vec<_>>(),
            })
            .to_string())
        }

        "TerminalContext" => {
            // Terminal session + output state is buffered in React; passed through TurnInput.
            Ok(input.terminal_context.as_ref().map_or_else(
                || {
                    serde_json::json!({
                        "sessionCount": 0,
                        "sessions": [],
                        "notes": ["No terminal context was provided for this turn."],
                    })
                    .to_string()
                },
                std::string::ToString::to_string,
            ))
        }
        "TerminalWrite" => {
            let data = json_str(&args, "data");
            if data.is_empty() {
                return Err("TerminalWrite requires non-empty data.".to_string());
            }
            let session_id_str = json_str_opt(&args, "sessionId");
            require_tool_approval(
                app,
                turn_id,
                tc,
                &input.tool_approval_mode,
                "TerminalWrite",
                "Write to terminal",
                &data.chars().take(120).collect::<String>(),
                "execute",
            )
            .await?;
            let session_id = match session_id_str {
                Some(id) => {
                    uuid::Uuid::parse_str(&id).map_err(|_| "invalid session id".to_string())?
                }
                None => return Err("TerminalWrite requires a sessionId.".to_string()),
            };
            crate::terminal::terminal_write(state.clone(), session_id, data.clone())?;
            Ok(serde_json::json!({ "bytesWritten": data.len(), "sessionId": session_id.to_string() }).to_string())
        }

        "Task" => {
            let description = json_str(&args, "description");
            let prompt = json_str(&args, "prompt");
            if description.is_empty() || prompt.is_empty() {
                return Err("Task requires description and prompt.".to_string());
            }
            let subagent_type = json_str_opt(&args, "subagent_type")
                .unwrap_or_else(|| "generalPurpose".to_string());
            let agent_id = format!("subagent-{}", uuid::Uuid::new_v4().simple());
            let summary = run_subagent(
                app,
                state,
                input,
                &agent_id,
                &description,
                &prompt,
                &subagent_type,
            )
            .await?;
            Ok(serde_json::json!({
                "agentId": agent_id,
                "subagentType": subagent_type,
                "summary": summary,
            })
            .to_string())
        }

        "ContextBudgeter" => {
            let query = json_str(&args, "query");
            if query.is_empty() {
                return Err("ContextBudgeter requires a non-empty query.".to_string());
            }
            let target_chars = json_usize(&args, "targetChars", 16_000).clamp(2_000, 22_000);
            // Compose ranked context from native tools, then budget-select by score.
            let mut items: Vec<(String, String, i64)> = Vec::new(); // (kind, content, score)
            if let Ok(rc) = crate::ai_context_sources::ai_rules_context(
                state.clone(),
                Some(query.clone()),
                Some(6),
                None,
            )
            .await
            {
                for f in rc.files {
                    items.push((
                        "rule".into(),
                        format!("{}: {}", f.relative_path, f.text),
                        60,
                    ));
                }
            }
            if let Ok(mc) = crate::ai_context_sources::ai_memory_context(
                state.clone(),
                Some(query.clone()),
                Some(6),
                None,
            )
            .await
            {
                for f in mc.files {
                    items.push((
                        "memory".into(),
                        format!("{}: {}", f.relative_path, f.text),
                        55,
                    ));
                }
            }
            if let Ok(rf) = crate::ai_related::ai_related_files(
                state.clone(),
                input.active_document_path.clone(),
                Some(query.clone()),
                Some(18),
                Some(5000),
            )
            .await
            {
                for f in rf.files {
                    items.push((
                        "related-file".into(),
                        format!("{} (score {})", f.relative_path, f.score),
                        40 + f.score.min(40),
                    ));
                }
            }
            if let Ok(diag) = crate::lsp::diagnostics_snapshot(state.clone()) {
                for d in diag.into_iter().take(20) {
                    items.push((
                        "diagnostic".into(),
                        serde_json::to_string(&d).unwrap_or_default(),
                        50,
                    ));
                }
            }
            // Rank by score desc, then budget-select.
            items.sort_by_key(|item| std::cmp::Reverse(item.2));
            let mut selected = Vec::new();
            let mut used = 0usize;
            for (kind, content, score) in items {
                if used >= target_chars {
                    break;
                }
                let clamped: String = content.chars().take(1800).collect();
                used += clamped.len();
                selected.push(serde_json::json!({ "kind": kind, "score": score, "chars": clamped.len(), "content": clamped }));
            }
            Ok(serde_json::json!({ "query": query, "targetChars": target_chars, "selectedChars": used, "count": selected.len(), "packet": selected }).to_string())
        }

        "Checkpoint" => {
            let action = json_str_opt(&args, "action").unwrap_or_else(|| "list".to_string());
            let id = json_str_opt(&args, "id");
            let label = json_str_opt(&args, "label");
            let paths = args.get("paths").and_then(|v| v.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            });
            let max_files = args
                .get("maxFiles")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| usize::try_from(v).ok());
            let max_bytes = args
                .get("maxBytesPerFile")
                .and_then(serde_json::Value::as_u64);
            let save = args.get("saveToDisk").and_then(serde_json::Value::as_bool);
            let dry = args.get("dryRun").and_then(serde_json::Value::as_bool);
            // Restore mutates files → require approval (unless dry-run / full-access).
            let is_restore = action.trim().to_lowercase().starts_with("rest")
                || action.trim().to_lowercase().starts_with("rollback")
                || action.trim().to_lowercase().starts_with("revert");
            if is_restore && !dry.unwrap_or(false) {
                require_tool_approval(
                    app,
                    turn_id,
                    tc,
                    &input.tool_approval_mode,
                    "Checkpoint",
                    "Restore checkpoint",
                    id.as_deref().unwrap_or("latest"),
                    "modify",
                )
                .await?;
            }
            let now_ms = chrono::Utc::now().timestamp_millis();
            let result = crate::ai_checkpoint::ai_checkpoint(
                app.clone(),
                state.clone(),
                action,
                id,
                label,
                paths,
                max_files,
                max_bytes,
                save,
                dry,
                now_ms,
            )
            .await?;
            Ok(result.to_string())
        }

        other => Err(format!("Unknown tool: {other}")),
    }
}

/// Run an isolated subagent turn (Task tool). The subagent gets its own model↔tool
/// loop with a capped round limit and read-only-leaning tools, then returns a concise
/// summary to the parent. Shares the session's A2A blackboard for coordination.
async fn run_subagent(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    parent: &TurnInput,
    agent_id: &str,
    description: &str,
    prompt: &str,
    subagent_type: &str,
) -> Result<String, String> {
    const MAX_SUBAGENT_ROUNDS: usize = 16;
    let read_only = matches!(subagent_type, "codeReviewer" | "explorer");

    // Subagent system prompt: focused, returns a summary.
    let instructions = format!(
        "You are a Lux subagent ({subagent_type}). Task: {description}\n\
         Work in an isolated context. Use tools to gather evidence and complete the task. \
         Coordinate via AgentMessage (read sibling findings, post your discoveries). \
         Return a concise final summary for the parent agent. Do not spawn further subagents."
    );
    let mut prompt_input = parent.prompt_input.clone();
    prompt_input.agent_instructions = instructions.clone();
    prompt_input.agent_name = format!("subagent:{subagent_type}");
    if read_only {
        prompt_input.agent_mode = "ask".to_string();
    }
    let system = crate::ai_prompt::build_system_prompt(&prompt_input);

    let mut messages: Vec<serde_json::Value> = vec![
        serde_json::json!({ "role": "system", "content": system }),
        serde_json::json!({ "role": "user", "content": prompt }),
    ];
    let tools = crate::ai_tool_defs::runtime_tool_definitions(
        if read_only { "ask" } else { &parent.agent_mode },
        parent.agent_browser_enabled,
    );

    let mut final_content = String::new();
    for _round in 0..MAX_SUBAGENT_ROUNDS {
        let payload = serde_json::json!({
            "model": parent.model,
            "messages": messages,
            "temperature": 0.2,
            "stream": false,
            "tools": tools,
            "tool_choice": "auto",
        });
        let request = crate::ai_chat_backend::AiChatCompletionRequest::new(
            parent.base_url.clone(),
            parent.api_key.clone(),
            payload,
        );
        let response = crate::ai_chat_backend::completion(request).await?;
        let assistant = parse_assistant_message(&response.body);
        if !assistant.content.is_empty() {
            final_content = assistant.content.clone();
        }
        if assistant.tool_calls.is_empty() {
            break;
        }
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": if assistant.content.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(assistant.content.clone()) },
            "tool_calls": assistant.tool_calls.iter().map(|tc| serde_json::json!({
                "id": tc.id, "type": "function",
                "function": { "name": tc.name, "arguments": tc.arguments },
            })).collect::<Vec<_>>(),
        }));
        // Subagents cannot spawn nested Task (depth limit) — block it inline.
        for child in &assistant.tool_calls {
            let result = if child.name == "Task" {
                Err("Nested subagents are not allowed (depth limit).".to_string())
            } else {
                // Subagent tool calls don't emit UI events (isolated context).
                Box::pin(execute_tool(app, state, parent, agent_id, child)).await
            };
            let content = match result {
                Ok(output) => output,
                Err(err) => serde_json::json!({ "error": err }).to_string(),
            };
            messages.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": child.id,
                "content": content,
            }));
        }
    }

    Ok(if final_content.is_empty() {
        "Subagent finished without a summary.".to_string()
    } else {
        final_content
    })
}

/// Check permission rules + mode, then prompt the UI for approval if needed.
// Approval context (tool, summary, preview, risk) is passed positionally; bundling into a
// struct would only shift the boilerplate to every call site without improving clarity.
#[allow(clippy::too_many_arguments)]
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
    let _ = emit_turn_event(
        app,
        &TurnEvent::ApprovalRequired {
            turn_id: turn_id.to_string(),
            request_id: tc.id.clone(),
            tool: tool.to_string(),
            title: format!("Approve {tool}"),
            summary: summary.to_string(),
            preview: preview.to_string(),
            risk: risk.to_string(),
        },
    );
    match rx.await {
        Ok(ApprovalDecision::Approved) => Ok(()),
        _ => Err(format!("{tool} was rejected by the user.")),
    }
}

/// Build agent-browser CLI args from tool name + arguments.
fn build_browser_args(tool_name: &str, args: &serde_json::Value) -> Vec<String> {
    match tool_name {
        "BrowserOpen" => {
            let mut a = vec!["open".to_string()];
            if let Some(url) = args.get("url").and_then(|v| v.as_str()) {
                a.push(url.to_string());
            }
            if args
                .get("headed")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                a.push("--headed".to_string());
            }
            a
        }
        "BrowserAct" => args
            .get("batchCommands")
            .and_then(|v| v.as_array())
            .map_or_else(
                || {
                    let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                    cmd.split_whitespace().map(str::to_string).collect()
                },
                |cmds| {
                    let mut a = vec!["batch".to_string()];
                    for cmd in cmds {
                        if let Some(s) = cmd.as_str() {
                            a.push(s.to_string());
                        }
                    }
                    a
                },
            ),
        "BrowserSnapshot" => {
            let mut a = vec!["snapshot".to_string()];
            if args
                .get("interactive")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true)
            {
                a.push("-i".to_string());
            }
            if args
                .get("compact")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true)
            {
                a.push("--compact".to_string());
            }
            if let Some(d) = args.get("depth").and_then(serde_json::Value::as_u64) {
                a.push("--depth".to_string());
                a.push(d.to_string());
            }
            a
        }
        "BrowserScreenshot" => {
            let mut a = vec!["screenshot".to_string()];
            if args
                .get("annotate")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                a.push("--annotate".to_string());
            }
            if args
                .get("fullPage")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                a.push("--full-page".to_string());
            }
            if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
                a.push(p.to_string());
            }
            a
        }
        "BrowserClose" => {
            let mut a = vec!["close".to_string()];
            if args
                .get("all")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                a.push("--all".to_string());
            }
            a
        }
        "BrowserChat" => {
            let instruction = args
                .get("instruction")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            vec!["chat".to_string(), instruction.to_string()]
        }
        "BrowserDashboard" => {
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("status");
            vec!["dashboard".to_string(), action.to_string()]
        }
        "BrowserInstall" => vec!["install".to_string()],
        "BrowserHelp" => {
            let mut a = vec!["help".to_string()];
            if let Some(t) = args.get("topic").and_then(|v| v.as_str()) {
                a.push(t.to_string());
            }
            a
        }
        "BrowserDoctor" => {
            let mut a = vec!["doctor".to_string()];
            if args
                .get("fix")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                a.push("--fix".to_string());
            }
            a
        }
        "BrowserInvoke" => args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        _ => vec![],
    }
}

fn json_str(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn json_str_opt(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn json_usize(value: &serde_json::Value, key: &str, default: usize) -> usize {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .map_or(default, |v| usize::try_from(v).unwrap_or(default))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_roundtrip() {
        let rx = register_approval("turn-1", "req-1");
        ai_resolve_turn_approval("turn-1".into(), "req-1".into(), ApprovalDecision::Approved)
            .unwrap();
        assert_eq!(rx.blocking_recv().unwrap(), ApprovalDecision::Approved);
    }

    #[test]
    fn approval_reject() {
        let rx = register_approval("turn-2", "req-2");
        ai_resolve_turn_approval("turn-2".into(), "req-2".into(), ApprovalDecision::Rejected)
            .unwrap();
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
        let result = ai_resolve_turn_approval(
            "no-turn".into(),
            "no-req".into(),
            ApprovalDecision::Approved,
        );
        assert!(result.is_err());
    }
}
