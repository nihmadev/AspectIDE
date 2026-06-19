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

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

fn approval_channels() -> &'static Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>> {
    static CHANNELS: OnceLock<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>> =
        OnceLock::new();
    CHANNELS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Channels the `AskUser` tool suspends on while waiting for the UI to deliver a
/// human answer. Same oneshot pattern as approvals, but the payload is the chosen
/// answer text (free-form, possibly multi-select joined) rather than a yes/no.
fn question_channels() -> &'static Mutex<HashMap<String, oneshot::Sender<QuestionAnswer>>> {
    static CHANNELS: OnceLock<Mutex<HashMap<String, oneshot::Sender<QuestionAnswer>>>> =
        OnceLock::new();
    CHANNELS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registry of turn ids the UI has asked to cancel. The model↔tool loop checks
/// this at every round boundary and after each tool call so a Stop actually
/// halts streaming + side-effecting tools instead of letting the turn run on.
fn cancelled_turns() -> &'static Mutex<HashSet<String>> {
    static CANCELLED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CANCELLED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Mark a turn cancelled so the loop (and any running subagent) stops ASAP.
fn mark_turn_cancelled(turn_id: &str) {
    if let Ok(mut set) = cancelled_turns().lock() {
        // A running loop consumes a genuine cancel within milliseconds; only
        // stale/late/never-run ids accumulate. Bound the set so a stream of
        // spurious late cancels can't leak unbounded for the process lifetime.
        if set.len() >= 128 {
            set.clear();
        }
        set.insert(turn_id.to_string());
    }
}

/// True if the turn has been cancelled. Subagents share the parent turn id, so
/// this also halts an in-flight Task tool.
fn is_turn_cancelled(turn_id: &str) -> bool {
    cancelled_turns()
        .lock()
        .is_ok_and(|set| set.contains(turn_id))
}

/// Drop the cancellation flag for a finished turn so the set never grows
/// unbounded (also lets a future turn reusing the id start clean).
fn clear_turn_cancelled(turn_id: &str) {
    if let Ok(mut set) = cancelled_turns().lock() {
        set.remove(turn_id);
    }
}

/// Build the system message. When `anthropic_cache` is set, the content is the
/// structured `[{type:text, cache_control:{type:ephemeral}}]` form Anthropic needs
/// to cache the prompt; otherwise a plain string (which other providers cache
/// automatically and won't reject for an unknown field).
fn build_system_message(system: &str, anthropic_cache: bool) -> serde_json::Value {
    if anthropic_cache {
        serde_json::json!({
            "role": "system",
            "content": [{
                "type": "text",
                "text": system,
                "cache_control": { "type": "ephemeral" },
            }],
        })
    } else {
        serde_json::json!({ "role": "system", "content": system })
    }
}

/// Extract cache-read prompt tokens from a usage object across provider shapes:
/// OpenAI/OpenRouter `prompt_tokens_details.cached_tokens`, Anthropic
/// `cache_read_input_tokens`, or a top-level `cached_tokens`.
fn parse_cached_prompt_tokens(usage: &serde_json::Value) -> u64 {
    let direct = usage
        .get("cache_read_input_tokens")
        .or_else(|| usage.get("cached_tokens"))
        .and_then(serde_json::Value::as_u64);
    if let Some(value) = direct {
        return value;
    }
    usage
        .get("prompt_tokens_details")
        .or_else(|| usage.get("input_tokens_details"))
        .and_then(|details| details.get("cached_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
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

    /// The agent asked the user a question (`AskUser` tool). UI renders an
    /// interactive card and replies via `ai_resolve_turn_question`. In Automatic
    /// mode this event never fires — the model self-answers inline instead.
    #[serde(rename_all = "camelCase")]
    QuestionRequired {
        turn_id: String,
        request_id: String,
        /// The question text.
        question: String,
        /// Optional clarifying detail shown under the question.
        detail: String,
        /// Suggested answers the user can pick (0..=10). May be empty (free-form only).
        options: Vec<QuestionOption>,
        /// True → the user may pick more than one option.
        multi_select: bool,
        /// True → a free-form "write your own answer" field is offered.
        allow_custom: bool,
        /// Optional self-contained HTML5 document to render as a sandboxed preview.
        html_preview: String,
    },

    /// The agent proposed a structured plan (`PresentPlan` tool). UI renders an
    /// expandable plan card with a "Start" button that hands the plan to Agent
    /// mode. In Automatic mode the plan is shown but execution auto-starts.
    #[serde(rename_all = "camelCase")]
    PlanProposed {
        turn_id: String,
        plan_id: String,
        title: String,
        summary: String,
        steps: Vec<PlanStep>,
        /// True when the turn-loop will proceed to execute without waiting (Automatic).
        auto_start: bool,
    },

    /// Token usage reported for the turn.
    #[serde(rename_all = "camelCase")]
    TurnUsage {
        turn_id: String,
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
        cached_prompt_tokens: u64,
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

    /// A transient provider failure is being retried automatically (connection
    /// phase only — no tokens have streamed yet). The UI surfaces a live
    /// "retrying (reason) — attempt n/m" notice instead of leaving the user
    /// staring at a frozen turn.
    #[serde(rename_all = "camelCase")]
    TurnRetry {
        turn_id: String,
        attempt: u32,
        max_attempts: u32,
        reason: String,
        detail: String,
        delay_ms: u64,
    },
}

// ── Interactive question / plan payloads ──

/// One suggested answer to an `AskUser` question.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    /// Short label shown on the option chip.
    pub label: String,
    /// Optional one-line explanation of the trade-off.
    #[serde(default)]
    pub description: String,
}

/// One step of a `PresentPlan` proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub title: String,
    #[serde(default)]
    pub detail: String,
    /// Optional file path this step primarily touches (drives the rail link).
    #[serde(default)]
    pub file: String,
}

// ── Approval types (UI → Rust) ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalDecision {
    Approved,
    Rejected,
}

/// Answer delivered from the UI for a pending `AskUser` question.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswer {
    /// The final answer text the model receives (selected labels and/or custom text).
    pub answer: String,
    /// True when the user dismissed the question without answering.
    #[serde(default)]
    pub cancelled: bool,
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

/// Register a pending question and return a receiver the `AskUser` tool awaits.
pub fn register_question(turn_id: &str, request_id: &str) -> oneshot::Receiver<QuestionAnswer> {
    let (tx, rx) = oneshot::channel();
    let key = format!("{turn_id}:{request_id}");
    if let Ok(mut map) = question_channels().lock() {
        map.insert(key, tx);
    }
    rx
}

/// Resolve a pending question from the UI side (delivers the human answer).
#[tauri::command]
pub fn ai_resolve_turn_question(
    turn_id: String,
    request_id: String,
    answer: QuestionAnswer,
) -> Result<(), String> {
    let key = format!("{turn_id}:{request_id}");
    let sender = question_channels()
        .lock()
        .map_err(|_| "question lock poisoned".to_string())?
        .remove(&key)
        .ok_or_else(|| format!("no pending question for {key}"))?;
    sender
        .send(answer)
        .map_err(|_| "question receiver dropped".to_string())
}

/// Cancel all pending questions for a turn (e.g. on abort): unblock the tool
/// loop with a cancelled answer so it stops cleanly instead of hanging.
pub fn cancel_questions_for_turn(turn_id: &str) {
    if let Ok(mut map) = question_channels().lock() {
        let prefix = format!("{turn_id}:");
        let keys: Vec<String> = map
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        for key in keys {
            if let Some(sender) = map.remove(&key) {
                let _ = sender.send(QuestionAnswer {
                    answer: String::new(),
                    cancelled: true,
                });
            }
        }
    }
}

/// Emit a turn event to the frontend.
pub fn emit_turn_event(app: &tauri::AppHandle, event: &TurnEvent) -> Result<(), String> {
    use tauri::Emitter;
    app.emit("lux://ai-turn", event).map_err(|e| e.to_string())
}

/// Map a backend `RetryNotice` onto a `TurnRetry` event for the active turn.
fn emit_retry_event(
    app: &tauri::AppHandle,
    turn_id: &str,
    notice: &crate::ai_chat_backend::RetryNotice,
) {
    let _ = emit_turn_event(
        app,
        &TurnEvent::TurnRetry {
            turn_id: turn_id.to_string(),
            attempt: notice.attempt,
            max_attempts: notice.max_attempts,
            reason: notice.reason.clone(),
            detail: notice.detail.clone(),
            delay_ms: notice.delay_ms,
        },
    );
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
    /// Provider reasoning payload (e.g. `{"reasoning_effort":"high","reasoning":{"effort":"high"}}`),
    /// computed on the frontend per provider/model. Empty object when the model has no
    /// effort levels. Its keys are merged into every outgoing request payload so the
    /// native turn-loop honors the selected reasoning effort like the TS path does.
    #[serde(default)]
    pub reasoning: Option<serde_json::Value>,
    /// True for Claude-family models: tag the system message with an Anthropic
    /// `cache_control` breakpoint so the (stable) system prompt is cached and
    /// re-read cheaply each turn. Parity with the TS applyPromptCacheBreakpoints.
    #[serde(default)]
    pub anthropic_cache: bool,
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
    // Clamp to [1, 128]: a limit of 0 would skip the loop entirely and emit a
    // fake "Done." success with no model call, so guarantee at least one round.
    let max_rounds = input.tool_round_limit.unwrap_or(32).clamp(1, 128) as usize;

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

    // Assemble messages array. For Claude-family models, tag the (stable) system
    // prompt with an Anthropic `cache_control` breakpoint so it is cached and
    // re-read cheaply each turn; other providers get a plain string they cache
    // automatically (and which avoids them rejecting the unknown field).
    let mut messages: Vec<serde_json::Value> = Vec::new();
    messages.push(build_system_message(&system, input.anthropic_cache));
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
    let mut usage_cached: u64 = 0;

    // ── Model ↔ tool loop ──
    for _round in 0..max_rounds {
        // Honor a Stop pressed between rounds: abort before another model call.
        if is_turn_cancelled(&turn_id) {
            clear_turn_cancelled(&turn_id);
            let _ = emit_turn_event(
                &app,
                &TurnEvent::TurnError {
                    turn_id: turn_id.clone(),
                    error: "cancelled".to_string(),
                },
            );
            return Ok(());
        }
        // Every round starts in "thinking": the frontend uses this as the round
        // boundary to open fresh ordered reasoning/text segments (so round 2+
        // after tools appends in order instead of overwriting earlier blocks).
        let _ = emit_turn_event(
            &app,
            &TurnEvent::StatusChange {
                turn_id: turn_id.clone(),
                phase: "thinking".to_string(),
            },
        );

        let mut payload = serde_json::json!({
            "model": input.model,
            "messages": messages,
            "temperature": 0.2,
            "stream": true,
            // OpenAI-compatible providers only emit the final usage chunk when
            // include_usage is set; without it TurnUsage would never fire.
            "stream_options": { "include_usage": true },
            "tools": tools,
            "tool_choice": "auto",
        });
        // Honor the user's selected reasoning effort (parity with the TS turn path).
        crate::ai_chat_backend::merge_reasoning(&mut payload, input.reasoning.as_ref());

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
        let cancel_turn_id = turn_id.clone();
        let retry_app = app.clone();
        let retry_turn_id = turn_id.clone();
        let mut announced_streaming = false;
        let response = match crate::ai_chat_backend::completion_streaming(
            request,
            move |content, reasoning| {
                // A Stop pressed mid-stream stops tokens reaching the UI here; the
                // should_cancel hook below also drops the in-flight socket, and the
                // post-stream cancellation check then finalizes the turn.
                if is_turn_cancelled(&stream_turn_id) {
                    return;
                }
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
            // Polled once per SSE chunk: a Stop drops the in-flight stream
            // immediately instead of waiting for the model's full generation.
            move || is_turn_cancelled(&cancel_turn_id),
            // Surface each automatic transient retry so the user sees a live
            // "retrying (reason) — attempt n/m" notice instead of a frozen turn.
            move |notice| emit_retry_event(&retry_app, &retry_turn_id, &notice),
        )
        .await
        {
            Ok(r) => r,
            Err(error) => {
                // Drop any cancellation flag for this id so a Stop racing the
                // model-call failure doesn't leak a stale entry (consistent with
                // the clear-on-finish path below).
                clear_turn_cancelled(&turn_id);
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
            usage_cached += parse_cached_prompt_tokens(usage);
        }

        let assistant = parse_assistant_message(&response.body);

        // Content was already streamed token-by-token via the on_delta callback
        // above; just record the final text (the frontend accumulated the deltas).
        if !assistant.content.is_empty() {
            final_content = assistant.content.clone();
        }

        // A Stop pressed while the model was streaming sets the flag but cannot
        // interrupt the in-flight stream. Check it the moment the stream returns,
        // BEFORE the tool-less `break` (which would otherwise finish as TurnDone
        // and report success) and BEFORE executing the first — possibly
        // destructive — tool call below.
        if is_turn_cancelled(&turn_id) {
            clear_turn_cancelled(&turn_id);
            let _ = emit_turn_event(
                &app,
                &TurnEvent::TurnError {
                    turn_id: turn_id.clone(),
                    error: "cancelled".to_string(),
                },
            );
            return Ok(());
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

            let result = execute_tool(&app, &state, &input, &turn_id, true, tc).await;

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

            // A Stop pressed during tool execution: stop before the next tool /
            // round so we don't keep running side-effecting tools post-abort.
            if is_turn_cancelled(&turn_id) {
                clear_turn_cancelled(&turn_id);
                let _ = emit_turn_event(
                    &app,
                    &TurnEvent::TurnError {
                        turn_id: turn_id.clone(),
                        error: "cancelled".to_string(),
                    },
                );
                return Ok(());
            }
        }
    }

    // The model ended the turn with no answer text — it may have only run tools, hit
    // the round limit, or returned an empty completion. Give it exactly one tool-free
    // turn (tool_choice "none" forces prose) to produce its final response, streamed
    // live so it renders as the answer instead of a bare "Done.". A normal turn (with
    // text) skips this entirely and pays nothing; it never loops.
    if final_content.trim().is_empty() && !is_turn_cancelled(&turn_id) {
        let _ = emit_turn_event(
            &app,
            &TurnEvent::StatusChange {
                turn_id: turn_id.clone(),
                phase: "thinking".to_string(),
            },
        );
        let mut payload = serde_json::json!({
            "model": input.model,
            "messages": messages,
            "temperature": 0.2,
            "stream": true,
            "stream_options": { "include_usage": true },
            "tools": tools,
            "tool_choice": "none",
        });
        crate::ai_chat_backend::merge_reasoning(&mut payload, input.reasoning.as_ref());
        let request = crate::ai_chat_backend::AiChatCompletionRequest::new(
            input.base_url.clone(),
            input.api_key.clone(),
            payload,
        );
        let stream_app = app.clone();
        let stream_turn_id = turn_id.clone();
        let cancel_turn_id = turn_id.clone();
        let retry_app = app.clone();
        let retry_turn_id = turn_id.clone();
        let mut announced_streaming = false;
        if let Ok(response) = crate::ai_chat_backend::completion_streaming(
            request,
            move |content, reasoning| {
                if is_turn_cancelled(&stream_turn_id) {
                    return;
                }
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
            move || is_turn_cancelled(&cancel_turn_id),
            move |notice| emit_retry_event(&retry_app, &retry_turn_id, &notice),
        )
        .await
        {
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
                usage_cached += parse_cached_prompt_tokens(usage);
            }
            let parsed = parse_assistant_message(&response.body);
            if !parsed.content.trim().is_empty() {
                final_content = parsed.content;
            }
        }
    }

    let duration_ms = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
    if final_content.trim().is_empty() {
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
                cached_prompt_tokens: usage_cached,
            },
        );
    }
    // Turn finished normally — drop any stale cancellation flag for this id.
    clear_turn_cancelled(&turn_id);
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

/// Cancel a running native turn — signals stop and aborts pending approvals.
#[tauri::command]
pub fn ai_cancel_turn(turn_id: String) {
    // Flag the turn first so the loop sees the cancellation even if the abort
    // lands between rounds (no pending approval to reject).
    mark_turn_cancelled(&turn_id);
    cancel_approvals_for_turn(&turn_id);
    cancel_questions_for_turn(&turn_id);
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
    interactive: bool,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args: serde_json::Value =
        serde_json::from_str(&tc.arguments).unwrap_or_else(|_| serde_json::json!({}));

    // Automatic mode is full autonomy: every side-effecting tool runs without a
    // human approval prompt (catastrophic-shell and path guards still apply, and
    // deny permission rules are enforced separately). Treating it as full-access
    // here means Write/StrReplace/PatchEngine/Delete/Shell/Browser/Checkpoint never
    // suspend the loop waiting for the user. Other modes keep the user's setting.
    let is_automatic = input.agent_mode == "automatic";
    let effective_approval_mode: &str = if is_automatic {
        "full-access"
    } else {
        input.tool_approval_mode.as_str()
    };

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
            // Automatic mode always persists to disk: staging an edit off-disk would leave
            // work the autonomous agent can never come back to apply. Honor the model arg otherwise.
            let save = if is_automatic {
                Some(true)
            } else {
                args.get("saveToDisk").and_then(serde_json::Value::as_bool)
            };
            require_tool_approval(
                app,
                turn_id,
                tc,
                effective_approval_mode,
                interactive,
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
            // Automatic mode always persists to disk: staging an edit off-disk would leave
            // work the autonomous agent can never come back to apply. Honor the model arg otherwise.
            let save = if is_automatic {
                Some(true)
            } else {
                args.get("saveToDisk").and_then(serde_json::Value::as_bool)
            };
            require_tool_approval(
                app,
                turn_id,
                tc,
                effective_approval_mode,
                interactive,
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
                effective_approval_mode,
                interactive,
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
            // Read-before-edit guard: PatchEngine mutates existing files just like
            // Write/StrReplace, so enforce the same invariant here instead of
            // letting the model clobber content it never read. Inspect the raw
            // operations (the typed struct's fields are private to ai_tools) and
            // guard every action that touches an EXISTING file; pure "create" ops
            // are exempt — the guard already no-ops on non-existent paths.
            let mut guarded_paths: Vec<String> = Vec::new();
            if let Some(ops) = operations_raw.as_array() {
                for op in ops {
                    let action = op
                        .get("action")
                        .or_else(|| op.get("kind"))
                        .or_else(|| op.get("operation"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase();
                    let mutates_existing = matches!(
                        action.as_str(),
                        "write"
                            | "rewrite"
                            | "replacefile"
                            | "replace_file"
                            | "strreplace"
                            | "str_replace"
                            | "replace"
                            | "delete"
                            | "remove"
                    );
                    if !mutates_existing {
                        continue;
                    }
                    if let Some(path) = op.get("path").and_then(|v| v.as_str()) {
                        require_file_read_before_edit(
                            state,
                            &input.session_id,
                            "PatchEngine",
                            path,
                        )?;
                        guarded_paths.push(path.to_string());
                    }
                }
            }
            let operations: Vec<crate::ai_tools::AiFilePatchOperation> =
                serde_json::from_value(operations_raw)
                    .map_err(|e| format!("Invalid patch operations: {e}"))?;
            // Automatic mode always persists to disk: staging an edit off-disk would leave
            // work the autonomous agent can never come back to apply. Honor the model arg otherwise.
            let save = if is_automatic {
                Some(true)
            } else {
                args.get("saveToDisk").and_then(serde_json::Value::as_bool)
            };
            let dry_run = args.get("dryRun").and_then(serde_json::Value::as_bool);
            if !dry_run.unwrap_or(false) {
                require_tool_approval(
                    app,
                    turn_id,
                    tc,
                    effective_approval_mode,
                    interactive,
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
            // After a real (non-dry-run) patch the touched files' contents are now
            // known to this turn; refresh their read markers like StrReplace does.
            if !dry_run.unwrap_or(false) {
                for path in &guarded_paths {
                    if let Ok(resolved) =
                        crate::resolve_workspace_path(state, std::path::Path::new(path))
                    {
                        crate::ai_session::mark_file_read(&input.session_id, &resolved);
                    }
                }
            }
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
        "WebResearch" => {
            let query = json_str(&args, "query");
            if query.trim().is_empty() {
                return Err("WebResearch requires a query.".to_string());
            }
            let focus = match json_str_opt(&args, "focus")
                .map(|value| value.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("academic") => lux_research::FocusMode::Academic,
                Some("news") => lux_research::FocusMode::News,
                Some("social") => lux_research::FocusMode::Social,
                Some("video") => lux_research::FocusMode::Video,
                Some("code") => lux_research::FocusMode::Code,
                _ => lux_research::FocusMode::Web,
            };
            let max_sources = args
                .get("maxSources")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| usize::try_from(v).ok());
            let options = lux_research::ResearchOptions {
                focus,
                ..lux_research::ResearchOptions::default()
            };
            let options = lux_research::ResearchOptions {
                max_sources: max_sources.unwrap_or(options.max_sources),
                ..options
            };
            let result = crate::research::web_research(state.clone(), query, Some(options)).await?;
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
                    effective_approval_mode,
                    interactive,
                    &tc.name,
                    &tc.name,
                    &browser_args.join(" "),
                    "execute",
                )
                .await?;
            }
            // Per-tool timeout: first Chromium boot (especially --headed) and live
            // navigation/automation routinely exceed 30s. Matching the TS path's
            // generous budgets stops BrowserOpen from spuriously "failing" while the
            // browser actually opens (the bug: 30s here vs ~35s real first-launch).
            let timeout_secs = match tc.name.as_str() {
                "BrowserInstall" => 600,
                "BrowserOpen" | "BrowserAct" | "BrowserChat" => 120,
                "BrowserScreenshot" | "BrowserDoctor" => 90,
                _ => 60,
            };
            let result =
                crate::agent_browser::invoke(crate::agent_browser::AgentBrowserInvokeRequest {
                    session: input.session_id.clone(),
                    args: browser_args,
                    headed: None,
                    allowed_domains: None,
                    max_output: Some(24_000),
                    timeout_secs: Some(timeout_secs),
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

        // Ask the user a question with optional suggested answers, an HTML5 preview,
        // and a free-form fallback. Suspends the loop until the UI replies — except
        // in Automatic mode, where the model is told to self-decide (full autonomy).
        "AskUser" => {
            let question = json_str(&args, "question");
            if question.trim().is_empty() {
                return Err("AskUser requires a non-empty question.".to_string());
            }
            let detail = json_str_opt(&args, "detail").unwrap_or_default();
            let html_preview = json_str_opt(&args, "htmlPreview").unwrap_or_default();
            let multi_select = args
                .get("multiSelect")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            // Custom answers are allowed by default; the model opts out only for
            // strict pick-from-list questions.
            let allow_custom = args
                .get("allowCustom")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            let options: Vec<QuestionOption> = args
                .get("options")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| {
                            // Accept both bare strings and { label, description } objects.
                            if let Some(label) = v.as_str() {
                                let label = label.trim();
                                if label.is_empty() {
                                    return None;
                                }
                                return Some(QuestionOption {
                                    label: label.to_string(),
                                    description: String::new(),
                                });
                            }
                            let label = v.get("label")?.as_str()?.trim().to_string();
                            if label.is_empty() {
                                return None;
                            }
                            Some(QuestionOption {
                                label,
                                description: v
                                    .get("description")
                                    .and_then(|d| d.as_str())
                                    .unwrap_or("")
                                    .trim()
                                    .to_string(),
                            })
                        })
                        // The model decides how many options (the contract caps at 10).
                        .take(10)
                        .collect()
                })
                .unwrap_or_default();

            // Automatic mode = full autonomy: never block on a human. Tell the model
            // to choose the best answer itself and keep going. Non-interactive
            // subagents likewise have no UI, so they self-decide too.
            if input.agent_mode == "automatic" || !interactive {
                let rendered = if options.is_empty() {
                    "Automatic mode: no user is available to answer. Decide the best course yourself using the evidence, state the assumption briefly, and continue.".to_string()
                } else {
                    let list = options
                        .iter()
                        .map(|o| {
                            if o.description.is_empty() {
                                format!("- {}", o.label)
                            } else {
                                format!("- {} — {}", o.label, o.description)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!("Automatic mode: no user is available to answer. Pick the single best option for this repository, state the choice as an assumption, and continue.\nOptions:\n{list}")
                };
                return Ok(
                    serde_json::json!({ "autoAnswered": true, "answer": rendered }).to_string(),
                );
            }

            // Interactive: emit the question and suspend on the oneshot channel.
            let rx = register_question(turn_id, &tc.id);
            let _ = emit_turn_event(
                app,
                &TurnEvent::StatusChange {
                    turn_id: turn_id.to_string(),
                    phase: "waiting-approval".to_string(),
                },
            );
            let _ = emit_turn_event(
                app,
                &TurnEvent::QuestionRequired {
                    turn_id: turn_id.to_string(),
                    request_id: tc.id.clone(),
                    question,
                    detail,
                    options,
                    multi_select,
                    allow_custom,
                    html_preview,
                },
            );
            match rx.await {
                Ok(answer) if !answer.cancelled && !answer.answer.trim().is_empty() => {
                    Ok(serde_json::json!({ "answer": answer.answer }).to_string())
                }
                Ok(_) => Ok(serde_json::json!({
                    "answer": "",
                    "dismissed": true,
                    "note": "User dismissed the question without answering. Proceed with your best judgment or ask again only if truly blocked."
                })
                .to_string()),
                Err(_) => Err("AskUser channel closed before an answer arrived.".to_string()),
            }
        }

        // Present a structured, reviewable plan. The UI renders an expandable plan
        // card; in Agent/Plan mode a "Start" button hands it to execution, in
        // Automatic mode execution auto-starts (the model proceeds immediately).
        "PresentPlan" => {
            let title = json_str_opt(&args, "title").unwrap_or_else(|| "Plan".to_string());
            let summary = json_str_opt(&args, "summary").unwrap_or_default();
            let steps: Vec<PlanStep> = args
                .get("steps")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| {
                            if let Some(title) = v.as_str() {
                                let title = title.trim();
                                if title.is_empty() {
                                    return None;
                                }
                                return Some(PlanStep {
                                    title: title.to_string(),
                                    detail: String::new(),
                                    file: String::new(),
                                });
                            }
                            let title = v.get("title")?.as_str()?.trim().to_string();
                            if title.is_empty() {
                                return None;
                            }
                            Some(PlanStep {
                                title,
                                detail: v
                                    .get("detail")
                                    .and_then(|d| d.as_str())
                                    .unwrap_or("")
                                    .trim()
                                    .to_string(),
                                file: v
                                    .get("file")
                                    .and_then(|d| d.as_str())
                                    .unwrap_or("")
                                    .trim()
                                    .to_string(),
                            })
                        })
                        .take(40)
                        .collect()
                })
                .unwrap_or_default();
            if steps.is_empty() {
                return Err("PresentPlan requires at least one step (array of strings or { title, detail, file }).".to_string());
            }
            // Pin the plan as the session goal + task list so the rail reflects it
            // immediately, regardless of mode.
            if summary.trim().is_empty() {
                crate::ai_session::set_goal(&input.session_id, &title);
            } else {
                crate::ai_session::set_goal(&input.session_id, &summary);
            }
            let todos: Vec<crate::ai_session::SessionTodo> = steps
                .iter()
                .enumerate()
                .map(|(i, step)| crate::ai_session::SessionTodo {
                    id: format!("plan-{}", i + 1),
                    content: step.title.clone(),
                    status: if i == 0 { "in_progress" } else { "pending" }.to_string(),
                    priority: "medium".to_string(),
                    notes: if step.detail.is_empty() {
                        None
                    } else {
                        Some(step.detail.clone())
                    },
                })
                .collect();
            crate::ai_session::set_todos(&input.session_id, todos);

            let auto_start = input.agent_mode == "automatic";
            let plan_id = format!("plan-{}", tc.id);
            let _ = emit_turn_event(
                app,
                &TurnEvent::PlanProposed {
                    turn_id: turn_id.to_string(),
                    plan_id,
                    title,
                    summary,
                    steps: steps.clone(),
                    auto_start,
                },
            );
            let guidance = if auto_start {
                "Plan presented and auto-started (Automatic mode). Begin executing step 1 now; do not wait for confirmation."
            } else {
                "Plan presented to the user. Stop here and wait — the user will press Start to hand the plan to Agent mode for execution. Do not begin editing yet."
            };
            Ok(serde_json::json!({ "stepCount": steps.len(), "autoStart": auto_start, "guidance": guidance }).to_string())
        }

        "ActiveContext" => {
            let workspace = crate::workspace_root(state).ok();
            let documents = state.documents.lock().map_err(|e| e.to_string())?;
            // Capture the true open-document count BEFORE truncating to the cap,
            // so the model is told the real total (mirrors DiagnosticsContext).
            let snaps = documents.snapshots();
            let open_document_count = snaps.len();
            let open_docs: Vec<serde_json::Value> = snaps
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
                "openDocumentCount": open_document_count,
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

        "RecallMemory" => {
            let query = json_str(&args, "query");
            let category = json_str_opt(&args, "category");
            let limit = args
                .get("limit")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| usize::try_from(v).ok())
                .unwrap_or(8);
            let options = lux_memory::SearchOptions {
                category,
                limit,
                ..Default::default()
            };
            let hits =
                crate::memory::memory_search(app.clone(), state.clone(), query, Some(options))
                    .await?;
            serde_json::to_string(&serde_json::json!({ "memories": hits }))
                .map_err(|e| e.to_string())
        }
        "RememberMemory" => {
            let content = json_str(&args, "content");
            if content.trim().is_empty() {
                return Ok(serde_json::json!({ "error": "content is required" }).to_string());
            }
            let category =
                json_str_opt(&args, "category").unwrap_or_else(|| "semantic".to_string());
            let input = lux_memory::NewMemory {
                category,
                content,
                importance: args.get("importance").and_then(serde_json::Value::as_f64),
                pinned: args.get("pinned").and_then(serde_json::Value::as_bool),
                source: Some("agent".to_string()),
                ..Default::default()
            };
            let memory = crate::memory::memory_create(app.clone(), state.clone(), input).await?;
            serde_json::to_string(&serde_json::json!({
                "status": "remembered",
                "id": memory.id,
                "category": memory.category,
            }))
            .map_err(|e| e.to_string())
        }
        "ListSkills" => {
            let query = json_str_opt(&args, "query").unwrap_or_default();
            let limit = args
                .get("limit")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| usize::try_from(v).ok())
                .unwrap_or(20);
            let matched =
                crate::skills::skills_match(app.clone(), state.clone(), query, Some(limit))?;
            let catalog: Vec<_> = matched
                .iter()
                .map(|scored| {
                    serde_json::json!({
                        "slug": scored.skill.slug,
                        "name": scored.skill.name,
                        "description": scored.skill.description,
                        "scope": scored.skill.scope,
                        "tags": scored.skill.tags,
                    })
                })
                .collect();
            serde_json::to_string(&serde_json::json!({ "skills": catalog }))
                .map_err(|e| e.to_string())
        }
        "UseSkill" => {
            let slug = json_str(&args, "slug");
            match crate::skills::skills_get(app.clone(), state.clone(), slug.clone())? {
                Some(skill) => serde_json::to_string(&serde_json::json!({
                    "slug": skill.slug,
                    "name": skill.name,
                    "description": skill.description,
                    "allowedTools": skill.allowed_tools,
                    "instructions": skill.body,
                }))
                .map_err(|e| e.to_string()),
                None => Ok(
                    serde_json::json!({ "error": format!("no skill named {slug}") }).to_string(),
                ),
            }
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
            // SkillsCatalog — reusable instruction modules relevant to the task.
            if let Ok(skills) =
                crate::skills::skills_match(app.clone(), state.clone(), query.clone(), Some(12))
            {
                if !skills.is_empty() {
                    let catalog: Vec<_> = skills
                        .iter()
                        .map(|scored| {
                            serde_json::json!({
                                "slug": scored.skill.slug,
                                "name": scored.skill.name,
                                "description": scored.skill.description,
                            })
                        })
                        .collect();
                    if let Ok(json) =
                        serde_json::to_string(&serde_json::json!({ "skills": catalog }))
                    {
                        parts.push(format!("SkillsCatalog: {json}"));
                    }
                }
            }
            // MemoryRecall — salient durable memories from the structured store.
            {
                let options = lux_memory::SearchOptions {
                    limit: 6,
                    ..Default::default()
                };
                if let Ok(hits) = crate::memory::memory_search(
                    app.clone(),
                    state.clone(),
                    query.clone(),
                    Some(options),
                )
                .await
                {
                    if !hits.is_empty() {
                        let items: Vec<_> = hits
                            .iter()
                            .map(|hit| {
                                serde_json::json!({
                                    "content": hit.memory.content,
                                    "category": hit.memory.category,
                                    "importance": hit.memory.importance,
                                })
                            })
                            .collect();
                        if let Ok(json) =
                            serde_json::to_string(&serde_json::json!({ "memories": items }))
                        {
                            parts.push(format!("MemoryRecall: {json}"));
                        }
                    }
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
            // Validate the sessionId BEFORE prompting for approval, so a missing
            // or malformed id fails fast instead of wasting an approval prompt
            // that immediately errors afterwards.
            let session_id = match json_str_opt(&args, "sessionId") {
                Some(id) => {
                    uuid::Uuid::parse_str(&id).map_err(|_| "invalid session id".to_string())?
                }
                None => return Err("TerminalWrite requires a sessionId.".to_string()),
            };
            require_tool_approval(
                app,
                turn_id,
                tc,
                effective_approval_mode,
                interactive,
                "TerminalWrite",
                "Write to terminal",
                &data.chars().take(120).collect::<String>(),
                "execute",
            )
            .await?;
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
            // `turn_id` here is the real parent turn id: subagents cannot spawn
            // Task (blocked inline below), so run_subagent is only reached on the
            // interactive parent path. Thread it through so a Stop on the parent
            // halts the subagent's own model↔tool loop instead of letting it run
            // to completion.
            let summary = run_subagent(
                app,
                state,
                input,
                turn_id,
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
                // Budget by character count (matches `target_chars`), not UTF-8
                // byte length, so multibyte content isn't over-counted.
                let n = clamped.chars().count();
                used += n;
                selected.push(serde_json::json!({ "kind": kind, "score": score, "chars": n, "content": clamped }));
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
            // Automatic mode always persists to disk: staging an edit off-disk would leave
            // work the autonomous agent can never come back to apply. Honor the model arg otherwise.
            let save = if is_automatic {
                Some(true)
            } else {
                args.get("saveToDisk").and_then(serde_json::Value::as_bool)
            };
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
                    effective_approval_mode,
                    interactive,
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

        // ── Code-graph tools ──
        "CodeGraphDefinition" => {
            let symbol = json_str(&args, "symbol");
            let result = crate::code_graph::with_index(state.inner(), |index| {
                let graph = index.graph();
                match lux_codegraph::resolve_one(graph, &symbol) {
                    Some(node) => Ok(serde_json::json!({
                        "found": true,
                        "name": node.name,
                        "file": node.file,
                        "line": node.line,
                    })),
                    None => Ok(serde_json::json!({"found": false, "note": format!("No symbol matching '{symbol}' in the code graph.")})),
                }
            }).await?;
            Ok(result.to_string())
        }
        "CodeGraphCallers" => {
            let symbol = json_str(&args, "symbol");
            let result = crate::code_graph::with_index(state.inner(), |index| {
                let graph = index.graph();
                let Some(nr) = lux_codegraph::resolve_one(graph, &symbol) else {
                    return Ok(serde_json::json!({"found": false, "note": format!("Unknown symbol: {symbol}")}));
                };
                let callers: Vec<serde_json::Value> = lux_codegraph::callers(graph, nr.node)
                    .into_iter()
                    .map(|r| serde_json::json!({"name": r.name, "file": r.file, "line": r.line}))
                    .collect();
                Ok(serde_json::json!({"symbol": nr.name, "callers": callers}))
            }).await?;
            Ok(result.to_string())
        }
        "CodeGraphCallees" => {
            let symbol = json_str(&args, "symbol");
            let result = crate::code_graph::with_index(state.inner(), |index| {
                let graph = index.graph();
                let Some(nr) = lux_codegraph::resolve_one(graph, &symbol) else {
                    return Ok(serde_json::json!({"found": false, "note": format!("Unknown symbol: {symbol}")}));
                };
                let callees: Vec<serde_json::Value> = lux_codegraph::callees(graph, nr.node)
                    .into_iter()
                    .map(|r| serde_json::json!({"name": r.name, "file": r.file, "line": r.line}))
                    .collect();
                Ok(serde_json::json!({"symbol": nr.name, "callees": callees}))
            }).await?;
            Ok(result.to_string())
        }
        "CodeGraphExplain" => {
            let symbol = json_str(&args, "symbol");
            let result = crate::code_graph::with_index(state.inner(), |index| {
                let graph = index.graph();
                let Some(nr) = lux_codegraph::resolve_one(graph, &symbol) else {
                    return Ok(serde_json::json!({"found": false, "note": format!("Unknown symbol: {symbol}")}));
                };
                let Some(expl) = lux_codegraph::explain(graph, nr.node) else {
                    return Ok(serde_json::json!({"found": false}));
                };
                Ok(serde_json::json!({
                    "name": expl.node.name,
                    "kind": format!("{:?}", expl.kind).to_lowercase(),
                    "degree": expl.degree,
                    "totalConnections": expl.total_connections,
                    "connections": expl.connections.into_iter().map(|n| serde_json::json!({
                        "name": n.node.name,
                        "file": n.node.file,
                        "line": n.node.line,
                        "relation": format!("{:?}", n.relation).to_lowercase(),
                        "direction": format!("{:?}", n.direction).to_lowercase(),
                    })).collect::<Vec<_>>(),
                }))
            }).await?;
            Ok(result.to_string())
        }
        "CodeGraphOverview" => {
            let result = crate::code_graph::with_index(state.inner(), |index| {
                let graph = index.graph();
                let gods = lux_codegraph::god_nodes(graph, 10);
                let nodes = index.graph().node_count();
                let edges = index.graph().edge_count();
                let communities = lux_codegraph::detect_communities(graph);
                Ok(serde_json::json!({
                    "nodes": nodes,
                    "edges": edges,
                    "communities": communities.len(),
                    "godNodes": gods.into_iter().map(|g| serde_json::json!({
                        "name": g.name,
                        "degree": g.degree,
                    })).collect::<Vec<_>>(),
                }))
            })
            .await?;
            Ok(result.to_string())
        }

        other => Err(format!("Unknown tool: {other}")),
    }
}

/// Run an isolated subagent turn (Task tool). The subagent gets its own model↔tool
/// loop with a capped round limit and read-only-leaning tools, then returns a concise
/// summary to the parent. Shares the session's A2A blackboard for coordination.
#[allow(clippy::too_many_arguments)]
async fn run_subagent(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    parent: &TurnInput,
    parent_turn_id: &str,
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
        // Inherit the parent's Anthropic cache setting so the subagent's system
        // prompt is cached on Claude-family models too.
        build_system_message(&system, parent.anthropic_cache),
        serde_json::json!({ "role": "user", "content": prompt }),
    ];
    let tools = crate::ai_tool_defs::runtime_tool_definitions(
        if read_only { "ask" } else { &parent.agent_mode },
        parent.agent_browser_enabled,
    );

    let mut final_content = String::new();
    for _round in 0..MAX_SUBAGENT_ROUNDS {
        // A Stop on the parent turn must halt the subagent immediately instead of
        // burning up to MAX_SUBAGENT_ROUNDS model calls + running side-effecting
        // tools. The parent loop is blocked awaiting this Task, so this is the
        // only place the cancellation can be observed mid-subagent. Do NOT clear
        // the flag here — the parent's post-tool check still needs to see it to
        // abort the parent turn afterward.
        if is_turn_cancelled(parent_turn_id) {
            return Ok(if final_content.is_empty() {
                "Subagent cancelled.".to_string()
            } else {
                final_content
            });
        }
        let mut payload = serde_json::json!({
            "model": parent.model,
            "messages": messages,
            "temperature": 0.2,
            "stream": true,
            "stream_options": { "include_usage": true },
            "tools": tools,
            "tool_choice": "auto",
        });
        // Subagents inherit the parent turn's reasoning effort.
        crate::ai_chat_backend::merge_reasoning(&mut payload, parent.reasoning.as_ref());
        let request = crate::ai_chat_backend::AiChatCompletionRequest::new(
            parent.base_url.clone(),
            parent.api_key.clone(),
            payload,
        );
        // Use the streaming transport (the same one the parent turn uses). A
        // non-streaming request hangs against providers/local proxies that only
        // speak SSE — every round stalls until the request timeout, which the user
        // experiences as the whole IDE freezing while a subagent runs. Streaming
        // also lets a parent Stop abort the model call mid-flight (the `should_cancel`
        // hook) instead of only between rounds. The subagent is an isolated context,
        // so its tokens are intentionally not forwarded to the parent UI.
        let cancel_turn = parent_turn_id.to_string();
        let response = crate::ai_chat_backend::completion_streaming(
            request,
            |_content, _reasoning| {},
            move || is_turn_cancelled(&cancel_turn),
            // Subagents run in an isolated context with no UI to notify.
            |_notice| {},
        )
        .await?;
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
                Box::pin(execute_tool(app, state, parent, agent_id, false, child)).await
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
            // Stop the subagent between tools too, so a Stop mid-round doesn't
            // keep running the remaining (possibly side-effecting) tool calls.
            if is_turn_cancelled(parent_turn_id) {
                return Ok(if final_content.is_empty() {
                    "Subagent cancelled.".to_string()
                } else {
                    final_content
                });
            }
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
    interactive: bool,
    tool: &str,
    summary: &str,
    preview: &str,
    risk: &str,
) -> Result<(), String> {
    // Full-access mode → always approved.
    if approval_mode == "full-access" {
        return Ok(());
    }
    // Non-interactive callers (subagents) have no UI to approve through: the
    // approval event would be keyed by the agent id the UI filters out, so
    // awaiting it would deadlock the parent's Task call. Auto-reject instead so
    // the model adapts rather than hangs.
    if !interactive {
        return Err(format!(
            "{tool} requires approval and is unavailable to subagents."
        ));
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
