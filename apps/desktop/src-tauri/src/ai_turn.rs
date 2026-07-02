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

use std::collections::{HashMap, HashSet, VecDeque};
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
fn cancelled_turns() -> &'static Mutex<CancelRegistry> {
    static CANCELLED: OnceLock<Mutex<CancelRegistry>> = OnceLock::new();
    CANCELLED.get_or_init(|| Mutex::new(CancelRegistry::default()))
}

/// Bounded set of cancelled turn ids with FIFO eviction. The previous design
/// `clear()`-ed the *whole* set at a cap, which could wipe the flag of a turn
/// that was just cancelled but hadn't yet observed it — un-cancelling a live
/// turn (M4). Evicting only the OLDEST ids keeps the bound while guaranteeing a
/// freshly-inserted cancel is never dropped.
#[derive(Default)]
struct CancelRegistry {
    ids: HashSet<String>,
    order: VecDeque<String>,
}

/// Mark a turn cancelled so the loop (and any running subagent) stops ASAP.
fn mark_turn_cancelled(turn_id: &str) {
    /// Far above any realistic count of concurrently-tracked turns; only
    /// never-consumed ids accumulate, and the oldest are the safe ones to drop.
    const CAP: usize = 256;
    if let Ok(mut reg) = cancelled_turns().lock() {
        if reg.ids.insert(turn_id.to_string()) {
            reg.order.push_back(turn_id.to_string());
        }
        while reg.order.len() > CAP {
            if let Some(oldest) = reg.order.pop_front() {
                reg.ids.remove(&oldest);
            }
        }
    }
}

/// True if the turn has been cancelled. Subagents share the parent turn id, so
/// this also halts an in-flight Task tool.
fn is_turn_cancelled(turn_id: &str) -> bool {
    cancelled_turns()
        .lock()
        .is_ok_and(|reg| reg.ids.contains(turn_id))
}

/// Drop the cancellation flag for a finished turn so the set never grows
/// unbounded (also lets a future turn reusing the id start clean).
fn clear_turn_cancelled(turn_id: &str) {
    if let Ok(mut reg) = cancelled_turns().lock() {
        if reg.ids.remove(turn_id) {
            reg.order.retain(|id| id != turn_id);
        }
    }
}

/// Messages the UI staged WHILE a turn was running, keyed by `session_id:turn_id`.
/// Keying by `turn_id` prevents a restarted or concurrent turn draining messages
/// that were staged for a different (earlier) turn (F5 — misrouting live input).
fn pending_injections() -> &'static Mutex<HashMap<String, VecDeque<String>>> {
    static PENDING: OnceLock<Mutex<HashMap<String, VecDeque<String>>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Per-turn cap: at most this many mid-turn injections are buffered. Beyond this,
/// new ones are silently dropped (a truncation notice is already emitted to the model
/// when the cap is hit). Prevents a flood of staged messages from exploding context.
const MAX_INJECTIONS_PER_TURN: usize = 16;

/// Queue a user message for injection into the running turn identified by
/// `session_id` + `turn_id`. Silently drops messages beyond the per-turn cap.
pub fn enqueue_injection(session_id: &str, turn_id: &str, text: String) {
    if text.trim().is_empty() {
        return;
    }
    let key = format!("{session_id}:{turn_id}");
    if let Ok(mut map) = pending_injections().lock() {
        let queue = map.entry(key).or_default();
        if queue.len() < MAX_INJECTIONS_PER_TURN {
            queue.push_back(text);
        }
        // Silently drop: caller already emits a truncation notice via the turn event.
    }
}

/// Take all messages staged for the given `session_id`+`turn_id`, clearing that slot.
fn drain_injections(session_id: &str, turn_id: &str) -> Vec<String> {
    let key = format!("{session_id}:{turn_id}");
    if let Ok(mut map) = pending_injections().lock() {
        if let Some(queue) = map.get_mut(&key) {
            let drained: Vec<String> = queue.drain(..).collect();
            if !drained.is_empty() {
                map.remove(&key);
            }
            return drained;
        }
    }
    Vec::new()
}

/// Drop any leftover staged messages for this turn (early exit / cancellation / error).
/// Also drops the legacy session-only slot if present for backward compat.
fn clear_injections(session_id: &str, turn_id: &str) {
    let key = format!("{session_id}:{turn_id}");
    if let Ok(mut map) = pending_injections().lock() {
        map.remove(&key);
        // Also sweep any leftover legacy session-only slots.
        map.remove(session_id);
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

/// Fold one response's `usage` object into the running per-turn totals,
/// normalizing across provider shapes (`OpenAI` `*_tokens`, Anthropic
/// `input_tokens`/`output_tokens`). `total_tokens` is DERIVED from
/// prompt+completion when the provider omits it (Anthropic does), so the reported
/// total stays consistent across providers and rounds instead of summing a field
/// only some providers send (L9).
fn accumulate_usage(
    usage: &serde_json::Value,
    prompt: &mut u64,
    completion: &mut u64,
    total: &mut u64,
    cached: &mut u64,
) {
    let round_prompt = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let round_completion = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    *prompt += round_prompt;
    *completion += round_completion;
    *total += usage
        .get("total_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(round_prompt + round_completion);
    *cached += parse_cached_prompt_tokens(usage);
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

    /// A user message staged mid-turn was folded into the running conversation at a
    /// round boundary — the UI renders it as a user bubble before the next answer.
    #[serde(rename_all = "camelCase")]
    UserMessageInjected { turn_id: String, text: String },

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
        /// Key design decisions: chosen approach + tradeoff vs the alternative(s).
        alternatives: Vec<PlanDecision>,
        /// Failure modes / hidden assumptions the plan must survive (critique phase).
        risks: Vec<String>,
        /// Checks that prove it works + rollback trigger (verification phase).
        verification: Vec<String>,
        /// Deterministic plan-quality score in `[0,1]` from the 5-phase gate.
        quality: f64,
        /// Concrete coaching nudges for whatever the gate found missing.
        coaching: Vec<String>,
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

/// A key design decision in a `PresentPlan` proposal: the chosen approach plus
/// why it wins over the alternative(s). Ported from think-mcp's `alternative` +
/// `synthesis` reasoning phases — the part of a plan that proves the model
/// weighed tradeoffs instead of charging ahead with its first idea.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDecision {
    /// The chosen approach (e.g. "Server-side cursor pagination").
    pub option: String,
    /// Why it wins / what it costs vs the alternative(s).
    #[serde(default)]
    pub tradeoff: String,
}

/// Risk markers that demand a deeper, verified plan (ported from think-mcp's
/// complexity heuristic). Their presence in the goal/steps raises the bar for how
/// many concrete steps and explicit verification the plan must carry.
const PLAN_RISK_MARKERS: &[&str] = &[
    "security",
    "secure",
    "auth",
    "password",
    "token",
    "payment",
    "billing",
    "concurren",
    "migrat",
    "schema",
    "distributed",
    "performance",
    "rollback",
    "delete",
    "destructive",
    "public api",
    "breaking",
];

/// Vague step labels that signal a phase-level plan instead of concrete edits.
const PLAN_VAGUE_LABELS: &[&str] = &[
    "set up the project",
    "set up project",
    "setup",
    "implement business logic",
    "implement logic",
    "implement the feature",
    "add documentation",
    "write docs",
    "do the rest",
    "finish up",
    "wire everything",
    "make it work",
    "clean up",
    "testing",
    "polish",
];

/// Deterministic, advisory plan-quality assessment — the think-mcp cycle gate
/// applied to a `PresentPlan`. Returns a score in `[0,1]` plus concrete coaching
/// nudges for whatever is missing. Never blocks; it only tells the model how to
/// make the plan sharper.
///
/// Scores the five reasoning phases think-mcp's `cycle.service.ts` gate demands —
/// **decompose · alternative · critique · synthesis · verification** — folded into
/// a `[0,1]` score. `decompose`/`synthesis` live in `steps`+`summary`; the other
/// three phases are first-class structured inputs (`alternatives`, `risks`,
/// `verification`) but the gate also accepts the same content expressed in prose so
/// a plan is never punished for phrasing. Alternatives + critique are only
/// *expected* on non-trivial/risky work, so simple plans stay terse.
fn assess_plan_quality(
    title: &str,
    summary: &str,
    steps: &[PlanStep],
    alternatives: &[PlanDecision],
    risks: &[String],
    verification: &[String],
) -> (f64, Vec<String>) {
    let mut coaching: Vec<String> = Vec::new();
    let haystack = {
        let mut s = format!("{title}\n{summary}");
        for step in steps {
            s.push('\n');
            s.push_str(&step.title);
            s.push('\n');
            s.push_str(&step.detail);
        }
        for alt in alternatives {
            s.push('\n');
            s.push_str(&alt.option);
            s.push('\n');
            s.push_str(&alt.tradeoff);
        }
        for risk in risks {
            s.push('\n');
            s.push_str(risk);
        }
        for check in verification {
            s.push('\n');
            s.push_str(check);
        }
        s.to_lowercase()
    };
    let risk_hits = PLAN_RISK_MARKERS
        .iter()
        .filter(|m| haystack.contains(**m))
        .count();
    // Riskier work needs more decomposition: 3 steps baseline, +1 per risk marker, capped.
    let required_steps = (3 + risk_hits).min(8);
    // Alternatives/critique only matter once the work is big or risky enough to
    // carry a real design decision — don't nag a 2-step chore for tradeoffs.
    let expects_alternatives = risk_hits >= 1 || steps.len() >= 5;
    let expects_critique = risk_hits >= 1 || steps.len() >= 4;

    let mut score = 1.0_f64;

    // 1. Decompose — enough concrete steps for the risk.
    if steps.len() < required_steps {
        score -= 0.2;
        coaching.push(format!(
            "Decompose further — {} step(s) for {}-risk work; aim for ~{}, each a concrete action on a named file/module.",
            steps.len(),
            if risk_hits > 0 { "higher" } else { "this" },
            required_steps
        ));
    }

    // 2. Concreteness — steps should reference a file or carry real detail, not vague labels.
    let vague = steps
        .iter()
        .filter(|s| {
            let t = s.title.to_lowercase();
            PLAN_VAGUE_LABELS
                .iter()
                .any(|v| t == *v || t.starts_with(v))
        })
        .count();
    let with_anchor = steps
        .iter()
        .filter(|s| !s.file.is_empty() || s.detail.chars().count() >= 24)
        .count();
    if vague > 0 {
        score -= 0.15;
        coaching.push(format!(
            "Replace {vague} vague step label(s) (e.g. 'implement logic', 'add documentation') with a specific action + its acceptance check."
        ));
    }
    if steps.len() >= 3 && with_anchor * 2 < steps.len() {
        score -= 0.1;
        coaching.push(
            "Most steps lack a file or concrete detail — name the file/module each step touches and what proves it done.".to_string(),
        );
    }

    // 3. Alternative + synthesis — name the key decision and why it wins.
    let has_decision = alternatives.iter().any(|a| !a.option.trim().is_empty())
        || [
            "instead of",
            "rather than",
            "trade-off",
            "tradeoff",
            "alternative",
            " vs ",
            "chose ",
            "chosen ",
            "decided ",
            "вместо",
            "альтернатив",
            "компромисс",
        ]
        .iter()
        .any(|kw| haystack.contains(kw));
    if expects_alternatives && !has_decision {
        score -= 0.2;
        coaching.push(
            "Name the key decision: the approach you chose and why it wins over the alternative(s) (the tradeoff). A plan that weighs options beats one that charges ahead with its first idea.".to_string(),
        );
    }

    // 4. Critique — failure modes / hidden assumptions of the riskiest step.
    let has_critique = risks.iter().any(|r| !r.trim().is_empty())
        || [
            "risk",
            "fail",
            "assumption",
            "assume",
            "edge case",
            "race",
            "breaks if",
            "could break",
            "fallback",
            "риск",
            "провал",
            "допущен",
            "сломает",
        ]
        .iter()
        .any(|kw| haystack.contains(kw));
    if expects_critique && !has_critique {
        score -= 0.2;
        coaching.push(
            "Critique the riskiest step: list its failure modes and hidden assumptions — what breaks, under what input/timing, and how you'd catch it.".to_string(),
        );
    }

    // 5. Verification — an explicit check that proves it works.
    let has_verification = verification.iter().any(|v| !v.trim().is_empty())
        || steps.iter().any(|s| {
            let t = format!("{} {}", s.title, s.detail).to_lowercase();
            [
                "test",
                "verif",
                "build",
                "typecheck",
                "lint",
                "run ",
                "check",
                "assert",
                "validate",
            ]
            .iter()
            .any(|kw| t.contains(kw))
        });
    if !has_verification {
        score -= 0.25;
        coaching.push(
            "Add explicit verification: the tests/build/checks that prove it works (plus a rollback trigger for risky changes).".to_string(),
        );
    }

    // Rollback awareness for genuinely risky work.
    if risk_hits >= 2 {
        let has_rollback = haystack.contains("rollback")
            || haystack.contains("revert")
            || haystack.contains("checkpoint")
            || haystack.contains("backup");
        if !has_rollback {
            score -= 0.1;
            coaching.push(
                "High-risk plan: name a rollback/recovery path (Checkpoint, revert, or backup) for the riskiest step.".to_string(),
            );
        }
    }

    (score.clamp(0.0, 1.0), coaching)
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
    /// User-configured deny/ask/allow permission rules (`deny:Write(*.env)`, …).
    /// The native loop's authoritative gate: a Deny is a hard block even in
    /// full-access/automatic mode, an Allow skips the prompt, an Ask always
    /// prompts. Empty when unset. (Closes C2 — previously the engine ran only on
    /// the dev-only TS path, so the shipped app enforced nothing.)
    #[serde(default)]
    pub tool_permission_rules: Vec<String>,
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

    // Runtime tool definitions — generated natively in Rust, filtered by mode, plus
    // the live tools of any connected MCP server (namespaced mcp__<server>__<tool>).
    let mut tools = crate::ai_tool_defs::runtime_tool_definitions(
        &input.agent_mode,
        input.agent_browser_enabled,
    );
    if matches!(input.agent_mode.as_str(), "agent" | "automatic") {
        tools.extend(crate::mcp::agent_tool_definitions().await);
    }

    // F4: build the authoritative allowed-tool set from the exact definitions sent in
    // the request. Any tool call whose name is not in this set is rejected before
    // dispatch — enforcing mode restrictions at the Rust boundary, not only via
    // prompt/tool-definition shaping.
    let allowed_tool_names = tool_names_from_defs(&tools);

    let mut final_content = String::new();
    let mut usage_prompt: u64 = 0;
    let mut usage_completion: u64 = 0;
    let mut usage_total: u64 = 0;
    let mut usage_cached: u64 = 0;
    // True only when the model ended the turn by answering (no tool calls). When it
    // instead exhausts `max_rounds` mid-work, `final_content` may be stale text from
    // an early round; the recovery turn below then refreshes it (L8).
    let mut completed_naturally = false;
    // F7: aggregate tool-output budget for the WHOLE turn. The per-tool clamp below
    // bounds any single result, but a flood of many tools (or many rounds) can still
    // accumulate unbounded context. Track the post-clamp bytes appended to the
    // conversation and the total tool calls; once either ceiling is crossed we stop
    // issuing more tools and fall through to the tool-free recovery synthesis, which
    // turns the work done so far into a final answer instead of looping forever.
    let mut turn_output_bytes: usize = 0;
    let mut turn_tool_calls: usize = 0;
    let mut tool_budget_exceeded = false;
    /// Hard ceiling on cumulative tool-output bytes appended across the turn. Far
    /// above a normal multi-step task; only a tool flood reaches it.
    const TURN_OUTPUT_BYTE_BUDGET: usize = 600_000;
    /// Hard ceiling on total tool calls across the turn — a backstop against a model
    /// that calls tools without converging.
    const TURN_TOOL_CALL_BUDGET: usize = 200;

    // Clear the read-before-edit set at turn start: reads from a previous turn
    // must not authorize edits in the new one (on-disk state may have changed).
    crate::ai_session::clear_read_files(&input.session_id);

    // ── Model ↔ tool loop ──
    for _round in 0..max_rounds {
        // Honor a Stop pressed between rounds: abort before another model call.
        if is_turn_cancelled(&turn_id) {
            clear_turn_cancelled(&turn_id);
            clear_injections(&input.session_id, &turn_id); // F5: clean on every exit
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
            "stream": true,
            // OpenAI-compatible providers only emit the final usage chunk when
            // include_usage is set; without it TurnUsage would never fire.
            "stream_options": { "include_usage": true },
            "tools": tools,
            "tool_choice": "auto",
        });
        // Honor the user's selected reasoning effort (parity with the TS turn path).
        crate::ai_chat_backend::merge_reasoning(&mut payload, input.reasoning.as_ref());
        // Standard models only — reasoning models reject an explicit temperature.
        crate::ai_chat_backend::apply_temperature(&mut payload, input.reasoning.as_ref(), 0.2);

        let request = crate::ai_chat_backend::AiChatCompletionRequest::with_protocol(
            input.base_url.clone(),
            input.api_key.clone(),
            payload,
            input.prompt_input.provider_protocol.clone(),
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
                clear_injections(&input.session_id, &turn_id); // F5: clean on every exit
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
            accumulate_usage(
                usage,
                &mut usage_prompt,
                &mut usage_completion,
                &mut usage_total,
                &mut usage_cached,
            );
        }

        let assistant = parse_assistant_message(&response.body);

        // Content was already streamed token-by-token via the on_delta callback
        // above; just record the final text (the frontend accumulated the deltas).
        // If the model produced ONLY reasoning (empty content — common for reasoning
        // models on a trivial prompt), fall back to the thinking text so the turn
        // shows a real answer instead of "The turn produced no answer".
        if !assistant.content.is_empty() {
            final_content = assistant.content.clone();
        } else if final_content.trim().is_empty() && !assistant.reasoning.trim().is_empty() {
            final_content = assistant.reasoning.clone();
        }

        // A Stop pressed while the model was streaming sets the flag but cannot
        // interrupt the in-flight stream. Check it the moment the stream returns,
        // BEFORE the tool-less `break` (which would otherwise finish as TurnDone
        // and report success) and BEFORE executing the first — possibly
        // destructive — tool call below.
        if is_turn_cancelled(&turn_id) {
            clear_turn_cancelled(&turn_id);
            clear_injections(&input.session_id, &turn_id); // F5: clean on every exit
            let _ = emit_turn_event(
                &app,
                &TurnEvent::TurnError {
                    turn_id: turn_id.clone(),
                    error: "cancelled".to_string(),
                },
            );
            return Ok(());
        }

        // No tool calls → the model would end the turn here. But if the user staged a
        // message mid-work, fold it in NOW and run another round so the model answers
        // it before the turn closes — otherwise a recommendation sent during the final
        // round would be silently dropped (then wiped by clear_injections on TurnDone).
        if assistant.tool_calls.is_empty() {
            let injected = drain_injections(&input.session_id, &turn_id);
            if injected.is_empty() {
                completed_naturally = true;
                break;
            }
            // Commit the assistant's just-streamed answer so the conversation stays
            // well-formed, then append the staged user message(s) and loop again.
            if !assistant.content.is_empty() {
                messages.push(serde_json::json!({
                    "role": "assistant",
                    "content": assistant.content.clone(),
                }));
            }
            for text in injected {
                let _ = emit_turn_event(
                    &app,
                    &TurnEvent::UserMessageInjected {
                        turn_id: turn_id.clone(),
                        text: text.clone(),
                    },
                );
                messages.push(serde_json::json!({ "role": "user", "content": text }));
            }
            continue;
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

        // F6: pre-scan this batch for paths that are ONLY being read for the first time
        // in the same batch as an edit. A model cannot have observed a Read result it
        // issued in the same response — so an edit whose only prior read is from this
        // same batch is based on content the model never saw.
        //
        // Algorithm: collect paths that receive a first-time Read in this batch AND
        // also receive an edit in this batch. Those paths are "batch-read-only" and
        // any edit against them in this batch is rejected with a "read first" message.
        let batch_first_reads: std::collections::HashSet<String> = {
            let mut reads_in_batch = std::collections::HashSet::new();
            for tc in &assistant.tool_calls {
                let tc_args: serde_json::Value =
                    serde_json::from_str(&tc.arguments).unwrap_or_else(|_| serde_json::json!({}));
                if matches!(tc.name.as_str(), "Read" | "InspectFile") {
                    if let Some(raw) = tc_args.get("path").and_then(|v| v.as_str()) {
                        // Only a first-time read (file not yet in session) creates the risk.
                        if let Ok(resolved) =
                            crate::resolve_workspace_path(&state, std::path::Path::new(raw))
                        {
                            if !crate::ai_session::was_file_read(&input.session_id, &resolved) {
                                reads_in_batch
                                    .insert(resolved.to_string_lossy().replace('\\', "/"));
                            }
                        }
                    }
                }
            }
            reads_in_batch
        };

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

            // F6: reject edits whose only eligible read is from THIS same batch —
            // the model generated the edit before receiving the Read result.
            let tc_args_for_guard: serde_json::Value =
                serde_json::from_str(&tc.arguments).unwrap_or_else(|_| serde_json::json!({}));
            let batch_edit_path: Option<String> =
                if matches!(tc.name.as_str(), "StrReplace" | "PatchEngine") {
                    tc_args_for_guard
                        .get("path")
                        .and_then(|v| v.as_str())
                        .and_then(|raw| {
                            crate::resolve_workspace_path(&state, std::path::Path::new(raw))
                                .ok()
                                .map(|p| p.to_string_lossy().replace('\\', "/"))
                        })
                } else if tc.name == "Write" {
                    let overwrite = tc_args_for_guard
                        .get("overwrite")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    if overwrite {
                        tc_args_for_guard
                            .get("path")
                            .and_then(|v| v.as_str())
                            .and_then(|raw| {
                                crate::resolve_workspace_path(&state, std::path::Path::new(raw))
                                    .ok()
                                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                            })
                    } else {
                        None
                    }
                } else {
                    None
                };
            if let Some(edit_target) = batch_edit_path {
                if batch_first_reads.contains(&edit_target) {
                    // The only prior read for this path was in the same batch:
                    // the model never saw the content. Surface a recoverable error.
                    let result_early: Result<String, String> = Err(format!(
                        "{} blocked (F6): the Read of {} was issued in the same response as this edit. \
                         The model could not have seen the file contents. Read the file in a prior turn, \
                         then retry the edit.",
                        tc.name, edit_target
                    ));
                    let (status, output, error) = match result_early {
                        Ok(o) => ("success".to_string(), o, None),
                        Err(e) => ("error".to_string(), String::new(), Some(e)),
                    };
                    let _ = emit_turn_event(
                        &app,
                        &TurnEvent::ToolCallCompleted {
                            turn_id: turn_id.clone(),
                            call_id: tc.id.clone(),
                            status,
                            output: output.clone(),
                            error: error.clone(),
                        },
                    );
                    messages.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": tc.id,
                        "content": serde_json::json!({ "error": error.unwrap_or_default() }).to_string(),
                    }));
                    continue;
                }
            }

            // Box the per-tool future: `execute_tool` is a large state machine
            // (every tool arm) and would otherwise blow the `large_futures` budget.
            let result = Box::pin(execute_tool(
                &app,
                &state,
                &input,
                &turn_id,
                true,
                tc,
                &allowed_tool_names,
            ))
            .await;

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

            // F7: clamp tool output before appending to the conversation so unbounded
            // MCP/browser/research results cannot explode the context window. Opaque
            // outputs (those that look like raw JSON or long text) are the main risk.
            const TOOL_OUTPUT_CHAR_LIMIT: usize = 32_000;
            let content_for_messages = if error.is_some() {
                serde_json::json!({ "error": error.clone().unwrap_or_default() }).to_string()
            } else if output.chars().count() > TOOL_OUTPUT_CHAR_LIMIT {
                // Truncate and append a metadata notice so the model knows context was cut.
                let truncated: String = output.chars().take(TOOL_OUTPUT_CHAR_LIMIT).collect();
                format!(
                    "{truncated}\n\n[Tool output truncated: {} chars total, showing first {TOOL_OUTPUT_CHAR_LIMIT}. Use targeted follow-up queries to retrieve specific sections.]",
                    output.chars().count()
                )
            } else {
                output
            };
            // F7: fold the post-clamp result into the per-turn aggregate budget so a
            // flood of (individually-bounded) tool outputs still can't grow context
            // without limit.
            turn_output_bytes = turn_output_bytes.saturating_add(content_for_messages.len());
            turn_tool_calls = turn_tool_calls.saturating_add(1);
            // Append tool result to conversation.
            messages.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": content_for_messages,
            }));

            // A Stop pressed during tool execution: stop before the next tool /
            // round so we don't keep running side-effecting tools post-abort.
            if is_turn_cancelled(&turn_id) {
                clear_turn_cancelled(&turn_id);
                clear_injections(&input.session_id, &turn_id); // F5: clean on every exit
                let _ = emit_turn_event(
                    &app,
                    &TurnEvent::TurnError {
                        turn_id: turn_id.clone(),
                        error: "cancelled".to_string(),
                    },
                );
                return Ok(());
            }

            // F7: aggregate tool-output budget reached — stop running tools for the
            // rest of this turn. We break out to the tool-free recovery synthesis
            // (which sees everything done so far) rather than continuing to flood the
            // context. The model is told its work was capped via the tool result.
            if turn_output_bytes >= TURN_OUTPUT_BYTE_BUDGET
                || turn_tool_calls >= TURN_TOOL_CALL_BUDGET
            {
                tool_budget_exceeded = true;
                messages.push(serde_json::json!({
                    "role": "user",
                    "content": format!(
                        "[Tool budget reached for this turn: {turn_tool_calls} tool calls, \
                         ~{turn_output_bytes} bytes of tool output. No more tools will run this \
                         turn — synthesize a final answer from the results already gathered.]"
                    ),
                }));
                break;
            }
        }
        // F7: a budget break inside the tool loop ends the whole round loop; the
        // recovery synthesis below (forced tool_choice "none") produces the answer.
        if tool_budget_exceeded {
            break;
        }

        // Inter-round injection: fold in any messages the user staged mid-work so a
        // recommendation reaches the model at THIS gap, not after the whole turn.
        // Appended as user messages after the round's tool results so the model sees
        // them on its next call; the UI is told to render the bubbles in order.
        for injected in drain_injections(&input.session_id, &turn_id) {
            let _ = emit_turn_event(
                &app,
                &TurnEvent::UserMessageInjected {
                    turn_id: turn_id.clone(),
                    text: injected.clone(),
                },
            );
            messages.push(serde_json::json!({ "role": "user", "content": injected }));
        }
    }

    // The model ended the turn with no answer text — it may have only run tools, hit
    // the round limit, or returned an empty completion. Give it exactly one tool-free
    // turn (tool_choice "none" forces prose) to produce its final response, streamed
    // live so it renders as the answer instead of a bare "Done.". A normal turn that
    // finished by answering (`completed_naturally`) skips this entirely and pays
    // nothing; it never loops. When the round limit was hit instead, refresh even a
    // non-empty `final_content` so the answer reflects the latest work, not stale
    // text from an early round (L8).
    if (final_content.trim().is_empty() || !completed_naturally) && !is_turn_cancelled(&turn_id) {
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
            "stream": true,
            "stream_options": { "include_usage": true },
            "tools": tools,
            "tool_choice": "none",
        });
        crate::ai_chat_backend::merge_reasoning(&mut payload, input.reasoning.as_ref());
        crate::ai_chat_backend::apply_temperature(&mut payload, input.reasoning.as_ref(), 0.2);
        let request = crate::ai_chat_backend::AiChatCompletionRequest::with_protocol(
            input.base_url.clone(),
            input.api_key.clone(),
            payload,
            input.prompt_input.provider_protocol.clone(),
        );
        let stream_app = app.clone();
        let stream_turn_id = turn_id.clone();
        let cancel_turn_id = turn_id.clone();
        let retry_app = app.clone();
        let retry_turn_id = turn_id.clone();
        let mut announced_streaming = false;
        // F3: handle Err from the recovery call explicitly. A provider error or
        // cancellation here must NOT be swallowed — report TurnError instead of
        // emitting TurnDone with stale content.
        match crate::ai_chat_backend::completion_streaming(
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
            Ok(response) => {
                // Check for a post-recovery cancellation before we use the result.
                if is_turn_cancelled(&turn_id) {
                    clear_turn_cancelled(&turn_id);
                    clear_injections(&input.session_id, &turn_id);
                    let _ = emit_turn_event(
                        &app,
                        &TurnEvent::TurnError {
                            turn_id,
                            error: "cancelled".to_string(),
                        },
                    );
                    return Ok(());
                }
                if let Some(usage) = response.body.get("usage") {
                    accumulate_usage(
                        usage,
                        &mut usage_prompt,
                        &mut usage_completion,
                        &mut usage_total,
                        &mut usage_cached,
                    );
                }
                let parsed = parse_assistant_message(&response.body);
                if !parsed.content.trim().is_empty() {
                    final_content = parsed.content;
                } else if final_content.trim().is_empty() && !parsed.reasoning.trim().is_empty() {
                    // Recovery also came back reasoning-only — surface the thinking text
                    // rather than the canned placeholder.
                    final_content = parsed.reasoning;
                }
            }
            Err(error) => {
                // The final synthesis failed: emit TurnError rather than TurnDone
                // with stale content from an earlier round (F3 — correctness).
                clear_turn_cancelled(&turn_id);
                clear_injections(&input.session_id, &turn_id);
                let _ = emit_turn_event(
                    &app,
                    &TurnEvent::TurnError {
                        turn_id,
                        error: format!("Recovery synthesis failed: {error}"),
                    },
                );
                return Ok(());
            }
        }
    }

    let duration_ms = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
    if final_content.trim().is_empty() {
        final_content =
            "The turn produced no answer. Press **Retry** or rephrase your request.".to_string();
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
    // Turn finished normally — drop any stale cancellation flag for this id and
    // discard anything staged but not yet drained (it would target a dead turn;
    // the frontend re-queues it for the next turn instead).
    clear_turn_cancelled(&turn_id);
    clear_injections(&input.session_id, &turn_id);
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

/// Stage a user message for injection into a specific running turn.
/// `turn_id` scopes the injection so a restarted turn cannot drain messages
/// that belonged to an older one (F5 — misrouting live input between turns).
#[tauri::command]
pub fn ai_inject_message(session_id: String, turn_id: String, text: String) {
    enqueue_injection(&session_id, &turn_id, text);
}

// ── Response parsing ──

struct ParsedAssistant {
    content: String,
    /// The model's thinking text (`reasoning_content` / Anthropic-translated
    /// `thinking`). Kept so a reasoning-only completion — empty `content`, all
    /// thought — can fall back to it instead of finishing as "no answer".
    reasoning: String,
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
    // Reasoning models can finish a trivial prompt with empty content and only
    // thinking text; read it back (OpenAI streams it as reasoning_content) so the
    // turn can fall back to it instead of surfacing a bare "no answer".
    let reasoning = message
        .and_then(|m| m.get("reasoning_content").or_else(|| m.get("reasoning")))
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
        reasoning,
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

/// F4: collect the set of tool names from the exact definitions sent to the model.
/// `execute_tool` rejects any call whose name is not in this set, so mode
/// restrictions are enforced at the Rust boundary — a forged `Write`/`Shell` call
/// in a read-only mode (whose definitions never included those tools) is blocked
/// regardless of approval settings. Handles both the `OpenAI` `{function:{name}}`
/// shape and a bare `{name}` shape.
fn tool_names_from_defs(tools: &[serde_json::Value]) -> std::collections::HashSet<String> {
    tools
        .iter()
        .filter_map(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .or_else(|| t.get("name"))
                .and_then(|n| n.as_str())
        })
        .map(str::to_string)
        .collect()
}

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
    allowed_tool_names: &std::collections::HashSet<String>,
) -> Result<String, String> {
    // F4: hard-enforce mode allowlist — reject tool calls whose name was not in the
    // definitions sent to the model. A compromised proxy or malformed response cannot
    // route Write/Shell/McpManage into plan/ask modes by naming them in the response.
    if !allowed_tool_names.is_empty() && !allowed_tool_names.contains(&tc.name) {
        return Err(format!(
            "{} is not available in {} mode and was blocked by the tool allowlist.",
            tc.name, input.agent_mode
        ));
    }

    let args: serde_json::Value =
        serde_json::from_str(&tc.arguments).unwrap_or_else(|_| serde_json::json!({}));

    // Automatic mode is full autonomy: every side-effecting tool runs without a
    // human approval prompt (catastrophic-shell and path guards still apply).
    // `require_tool_approval` still evaluates the user's permission rules first, so
    // a `deny:` rule is a hard block even here. Treating the mode as full-access
    // means Write/StrReplace/PatchEngine/Delete/Shell/Browser/Checkpoint never
    // suspend the loop waiting for the user. Other modes keep the user's setting.
    let is_automatic = input.agent_mode == "automatic";
    let effective_approval_mode: &str = if is_automatic {
        "full-access"
    } else {
        input.tool_approval_mode.as_str()
    };

    match tc.name.as_str() {
        // ── MCP proxy: mcp__<server>__<tool> → the connected server's tool ──
        name if name.starts_with("mcp__") => {
            let rest = &name["mcp__".len()..];
            let (server, tool) = rest
                .split_once("__")
                .ok_or_else(|| format!("malformed MCP tool name: {name}"))?;
            // MCP tools are opaque third-party code (fs/shell/net): gate them like
            // any side-effecting tool (H7). Rules match against `Mcp(server/tool)`,
            // e.g. `deny:Mcp(*)` blocks all, `allow:Mcp(github/*)` trusts a server.
            let mcp_target = format!("{server}/{tool}");
            let preview = serde_json::to_string(&args).unwrap_or_default();
            require_tool_approval(
                app,
                turn_id,
                tc,
                effective_approval_mode,
                interactive,
                "Mcp",
                &format!("Call MCP tool {mcp_target}"),
                &preview.chars().take(400).collect::<String>(),
                "execute",
                &input.tool_permission_rules,
                &mcp_target,
                false,
            )
            .await?;
            crate::mcp::call_tool(server, tool, args).await
        }
        // ── MCP self-management: install / inspect / restart servers ──
        "McpManage" => {
            let action = json_str(&args, "action").to_lowercase();
            let id = json_str(&args, "id");
            // 'list' is read-only; every mutating action runs a process or writes
            // config, so gate them through the approval flow like other side effects.
            if action != "list" {
                let preview = serde_json::to_string(&args).unwrap_or_default();
                require_tool_approval(
                    app,
                    turn_id,
                    tc,
                    effective_approval_mode,
                    interactive,
                    "McpManage",
                    &format!("MCP {action} {id}"),
                    &preview.chars().take(400).collect::<String>(),
                    "execute",
                    &input.tool_permission_rules,
                    &format!("manage/{action}"),
                    false,
                )
                .await?;
            }
            match action.as_str() {
                "list" => {
                    let configs = crate::mcp::read_mcp_config(state);
                    let live = crate::mcp::all_status().await;
                    Ok(serde_json::json!({ "configured": configs, "live": live }).to_string())
                }
                "add" => {
                    let id = id.trim();
                    if id.is_empty() {
                        return Err("McpManage add requires 'id'.".to_string());
                    }
                    let command = json_str(&args, "command");
                    if command.trim().is_empty() {
                        return Err("McpManage add requires 'command'.".to_string());
                    }
                    let server_args = json_str_array(&args, "args", 64);
                    let env = args
                        .get("env")
                        .and_then(|v| v.as_object())
                        .map(|m| {
                            m.iter()
                                .filter_map(|(k, v)| {
                                    v.as_str().map(|s| (k.clone(), s.to_string()))
                                })
                                .collect::<std::collections::HashMap<String, String>>()
                        })
                        .unwrap_or_default();
                    let enabled = args
                        .get("enabled")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(true);
                    let name = json_str_opt(&args, "name").unwrap_or_else(|| id.to_string());
                    let config = crate::mcp::McpServerConfig {
                        id: id.to_string(),
                        name,
                        command,
                        args: server_args,
                        env,
                        enabled,
                    };
                    let status = crate::mcp::mcp_add(state.clone(), config).await?;
                    serde_json::to_string(&status).map_err(|e| e.to_string())
                }
                "connect" | "restart" => {
                    let configs = crate::mcp::read_mcp_config(state);
                    let config = configs
                        .into_iter()
                        .find(|c| c.id == id)
                        .ok_or_else(|| format!("MCP server '{id}' not found"))?;
                    let status = crate::mcp::connect_server(config).await?;
                    serde_json::to_string(&status).map_err(|e| e.to_string())
                }
                "disconnect" => {
                    crate::mcp::disconnect_server(&id).await;
                    Ok(serde_json::json!({ "id": id, "state": "disconnected" }).to_string())
                }
                "enable" | "disable" => {
                    let enabled = action == "enable";
                    crate::mcp::mcp_enable(state.clone(), id.clone(), enabled).await?;
                    Ok(serde_json::json!({ "id": id, "enabled": enabled }).to_string())
                }
                "remove" => {
                    crate::mcp::mcp_remove(state.clone(), id.clone()).await?;
                    Ok(serde_json::json!({ "id": id, "removed": true }).to_string())
                }
                other => Err(format!(
                    "Unknown McpManage action '{other}'. Use list|add|connect|restart|disconnect|enable|disable|remove."
                )),
            }
        }
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
            // Gate Shell like every other side-effecting tool (C1). Permission
            // rules run first; only a command classified read-only auto-approves
            // at the default tier (mirrors the TS `autoApproveOnDefault`).
            // Catastrophic commands are still refused inside `ai_shell` itself.
            let read_only = crate::ai_shell_safety::classify_shell_command(&command).read_only;
            require_tool_approval(
                app,
                turn_id,
                tc,
                effective_approval_mode,
                interactive,
                "Shell",
                &format!("Run: {}", command.chars().take(80).collect::<String>()),
                &command.chars().take(400).collect::<String>(),
                "execute",
                &input.tool_permission_rules,
                &command,
                read_only,
            )
            .await?;
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
            // Optional 1-based line window so the model can page a large file.
            let start_line = args
                .get("startLine")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| u32::try_from(v).ok());
            let max_lines = args
                .get("maxLines")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| u32::try_from(v).ok());
            let result = crate::ai_tools::ai_read_file(
                state.clone(),
                std::path::PathBuf::from(path),
                max_bytes,
                start_line,
                max_lines,
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
            // F9: resolve the path before evaluating permission rules so glob patterns
            // in deny/ask/allow rules match the canonical workspace-relative form, not
            // whatever spelling the model used (./x, mixed separators, etc.).
            let resolved_path = crate::resolve_workspace_path(state, std::path::Path::new(&path))
                .map_or_else(|_| path.clone(), |p| p.to_string_lossy().replace('\\', "/"));
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
                &input.tool_permission_rules,
                &resolved_path,
                false,
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
            // F9: resolve path for permission evaluation.
            let resolved_str_path =
                crate::resolve_workspace_path(state, std::path::Path::new(&path))
                    .map_or_else(|_| path.clone(), |p| p.to_string_lossy().replace('\\', "/"));
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
                &input.tool_permission_rules,
                &resolved_str_path, // F9: use resolved canonical path
                false,
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
            // F9: resolve path for permission evaluation.
            let resolved_delete_path =
                crate::resolve_workspace_path(state, std::path::Path::new(&path))
                    .map_or_else(|_| path.clone(), |p| p.to_string_lossy().replace('\\', "/"));
            require_tool_approval(
                app,
                turn_id,
                tc,
                effective_approval_mode,
                interactive,
                "Delete",
                &format!("Delete {path}"),
                &resolved_delete_path,
                "delete",
                &input.tool_permission_rules,
                &resolved_delete_path, // F9: use resolved canonical path
                false,
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
            // A non-repo workspace (or missing git) previously surfaced as an opaque
            // "service error:". Return an actionable, structured result instead so the
            // model knows git is simply unavailable here rather than that the tool broke.
            match crate::git::git_status(state.clone()).await {
                Ok(status) => serde_json::to_string(&status).map_err(|e| e.to_string()),
                Err(error) => Ok(serde_json::json!({
                    "isRepo": false,
                    "note": format!(
                        "Git status is unavailable for this workspace ({error}). It may not be a \
                         git repository (no .git), or git is not installed/on PATH."
                    ),
                })
                .to_string()),
            }
        }
        "DiagnosticsContext" => {
            let max = json_usize(&args, "maxResults", 80);
            let diagnostics = crate::lsp::diagnostics_snapshot(state.clone())?;
            let count = diagnostics.len();
            let truncated: Vec<_> = diagnostics.into_iter().take(max).collect();
            Ok(serde_json::json!({ "count": count, "diagnostics": truncated }).to_string())
        }
        "ReadLints" => {
            // Honor the advertised path/severity/source filters instead of returning
            // the whole snapshot. Filtering happens BEFORE the maxResults truncation,
            // and `count` reports the pre-truncation *filtered* length.
            let max = json_usize(&args, "maxResults", 80);
            // Optional path prefix: resolve the requested spelling to the canonical
            // workspace path so the prefix match works regardless of how the model
            // referenced it (./x, mixed separators, absolute vs relative).
            let path_prefix = json_str_opt(&args, "path").map(|raw| {
                crate::resolve_workspace_path(state, std::path::Path::new(&raw)).map_or_else(
                    |_| raw.replace('\\', "/"),
                    |p| p.to_string_lossy().replace('\\', "/"),
                )
            });
            let severity_filter =
                json_str_opt(&args, "severity").map(|s| s.trim().to_ascii_lowercase());
            let source_filter =
                json_str_opt(&args, "source").map(|s| s.trim().to_ascii_lowercase());

            let diagnostics = crate::lsp::diagnostics_snapshot(state.clone())?;
            let filtered: Vec<_> = diagnostics
                .into_iter()
                .filter(|d| {
                    // Path: prefix match on the canonical diagnostic path.
                    if let Some(prefix) = &path_prefix {
                        let diag_path = d.path.to_string_lossy().replace('\\', "/");
                        if !diag_path.starts_with(prefix.as_str()) {
                            return false;
                        }
                    }
                    // Severity: case-insensitive match against the enum name
                    // (Error/Warning/Information/Hint → error/warning/…).
                    if let Some(want) = &severity_filter {
                        let sev = format!("{:?}", d.severity).to_ascii_lowercase();
                        if &sev != want {
                            return false;
                        }
                    }
                    // Source: case-insensitive match against the diagnostic source.
                    if let Some(want) = &source_filter {
                        if d.source.to_ascii_lowercase() != *want {
                            return false;
                        }
                    }
                    true
                })
                .collect();
            let count = filtered.len();
            let truncated: Vec<_> = filtered.into_iter().take(max).collect();
            Ok(serde_json::json!({ "count": count, "diagnostics": truncated }).to_string())
        }
        "AgentMessage" => {
            // Normalize the action and route EXPLICITLY: read is the default, an
            // empty/omitted action reads (never writes), and an unrecognized action
            // is rejected rather than silently falling through to the write branch.
            let action = json_str(&args, "action").trim().to_ascii_lowercase();
            match action.as_str() {
                "read" | "get" | "" => {
                    let topic = json_str_opt(&args, "topic");
                    let limit = args
                        .get("limit")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|v| usize::try_from(v).ok());
                    let entries =
                        crate::ai_a2a::ai_blackboard_read(input.session_id.clone(), topic, limit)?;
                    serde_json::to_string(
                        &serde_json::json!({ "action": "read", "messages": entries }),
                    )
                    .map_err(|e| e.to_string())
                }
                "post" | "write" | "send" => {
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
                other => Err(format!(
                    "AgentMessage: unknown action '{other}' (use 'read' or 'post')."
                )),
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
                    let overwrite_flag = op
                        .get("overwrite")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    let is_create = matches!(action.as_str(), "create");
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
                            | "remove" // F8: treat create+overwrite on an existing file as an existing-file
                                       // mutation — require a prior eligible read rather than bypassing the guard.
                    ) || (is_create && overwrite_flag);
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
            // Deserialize op-by-op so a malformed entry names WHICH one (and why)
            // instead of a bare top-level "missing field `path`" that gives the model
            // no way to tell which operation to fix.
            let serde_json::Value::Array(raw_ops) = operations_raw else {
                return Err("PatchEngine `operations` must be an array.".to_string());
            };
            let mut operations: Vec<crate::ai_tools::AiFilePatchOperation> =
                Vec::with_capacity(raw_ops.len());
            for (index, raw) in raw_ops.into_iter().enumerate() {
                let op = serde_json::from_value(raw).map_err(|e| {
                    format!(
                        "PatchEngine operation[{index}] is invalid: {e}. Every operation needs at least `action` and `path`."
                    )
                })?;
                operations.push(op);
            }
            // Automatic mode always persists to disk: staging an edit off-disk would leave
            // work the autonomous agent can never come back to apply. Honor the model arg otherwise.
            let save = if is_automatic {
                Some(true)
            } else {
                args.get("saveToDisk").and_then(serde_json::Value::as_bool)
            };
            let dry_run = args.get("dryRun").and_then(serde_json::Value::as_bool);
            if !dry_run.unwrap_or(false) {
                // F9: resolve each guarded path before permission evaluation so
                // deny/ask/allow rules match the canonical form, not model spellings.
                let resolved_patch_targets: Vec<String> = guarded_paths
                    .iter()
                    .map(|p| {
                        crate::resolve_workspace_path(state, std::path::Path::new(p))
                            .map_or_else(|_| p.clone(), |r| r.to_string_lossy().replace('\\', "/"))
                    })
                    .collect();
                // Evaluate each target independently so a deny on any one blocks all.
                for resolved_target in &resolved_patch_targets {
                    require_tool_approval(
                        app,
                        turn_id,
                        tc,
                        effective_approval_mode,
                        interactive,
                        "PatchEngine",
                        &format!("{} operations", operations.len()),
                        resolved_target,
                        "modify",
                        &input.tool_permission_rules,
                        resolved_target,
                        false,
                    )
                    .await?;
                }
                // If there are no guarded paths (all creates), still require one gate.
                if resolved_patch_targets.is_empty() {
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
                        &input.tool_permission_rules,
                        "patch",
                        false,
                    )
                    .await?;
                }
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
            // `max_columns` bounds column previews for tabular/spreadsheet/notebook
            // files only; plain text and source files ignore it (text_preview clips
            // by bytes, not width). Track whether the model actually asked for it so
            // we can surface — rather than silently drop — the request below.
            let max_columns_requested = args
                .get("maxColumns")
                .and_then(serde_json::Value::as_u64)
                .is_some();
            if let Some(v) = args.get("maxColumns").and_then(serde_json::Value::as_u64) {
                options.max_columns = usize::try_from(v).unwrap_or(options.max_columns);
            }
            if let Some(v) = args.get("maxBytes").and_then(serde_json::Value::as_u64) {
                options.max_text_bytes = v;
            }
            let mut result = crate::file_intel::file_inspect(
                state.clone(),
                std::path::PathBuf::from(path),
                Some(options),
            )
            .await?;
            // Honesty: if maxColumns was requested but this resolved to a plain-text/
            // source preview, tell the model it had no effect here instead of letting
            // it assume the output was width-bounded.
            if max_columns_requested && matches!(result.preview, lux_core::FilePreview::Text { .. })
            {
                result.warnings.push(
                    "maxColumns applies only to tabular/spreadsheet/notebook previews; \
                     it was ignored for this plain-text/source file."
                        .to_string(),
                );
            }
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
            // No `allowPrivateHosts`: the model cannot disable the SSRF guard (H1).
            let result = crate::web_fetch::fetch(url, max_bytes, timeout_secs).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "WebResearch" => {
            let query = json_str(&args, "query");
            if query.trim().is_empty() {
                return Err("WebResearch requires a query.".to_string());
            }
            // An unrecognized focus previously fell through to Web silently, so the
            // model believed it ran an academic/code search when it did not. Compute
            // the focus and a note together and surface the note in the response.
            let focus_raw = json_str_opt(&args, "focus").map(|v| v.trim().to_ascii_lowercase());
            let (focus, focus_note) = match focus_raw.as_deref() {
                None | Some("web" | "") => (lux_research::FocusMode::Web, None),
                Some("academic") => (lux_research::FocusMode::Academic, None),
                Some("news") => (lux_research::FocusMode::News, None),
                Some("social") => (lux_research::FocusMode::Social, None),
                Some("video") => (lux_research::FocusMode::Video, None),
                Some("code") => (lux_research::FocusMode::Code, None),
                Some(other) => (
                    lux_research::FocusMode::Web,
                    Some(format!(
                        "Unrecognized focus '{other}'; used 'web'. Valid: web|academic|news|social|video|code."
                    )),
                ),
            };
            let max_sources = args
                .get("maxSources")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| usize::try_from(v).ok());
            // `depth`: standard (fast) vs deep (query expansion + multi-engine +
            // 1-hop crawl + more diverse sources). Unknown values → standard.
            let depth = match json_str_opt(&args, "depth")
                .map(|v| v.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("deep") => lux_research::ResearchDepth::Deep,
                _ => lux_research::ResearchDepth::Standard,
            };
            let defaults = lux_research::ResearchOptions::default();
            let options = lux_research::ResearchOptions {
                focus,
                depth,
                max_sources: max_sources.unwrap_or(defaults.max_sources),
                ..defaults
            };
            let mut result =
                crate::research::web_research(state.clone(), query, Some(options)).await?;
            if let Some(note) = focus_note {
                result.notes.insert(0, note);
            }
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }

        // ── SSH tools (system OpenSSH via lux-ssh) ──
        "SshConnect" => {
            let host = json_str(&args, "host");
            if host.trim().is_empty() {
                return Err(
                    "SshConnect requires a host (alias, hostname/IP, or user@host).".to_string(),
                );
            }
            let user = json_str_opt(&args, "user");
            let port = args
                .get("port")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| u16::try_from(value).ok());
            let identity_file = json_str_opt(&args, "identityFile");
            let label = json_str_opt(&args, "label");
            require_tool_approval(
                app,
                turn_id,
                tc,
                effective_approval_mode,
                interactive,
                "SshConnect",
                &format!("Open SSH connection to {host}"),
                &host,
                "execute",
                &input.tool_permission_rules,
                &host,
                false,
            )
            .await?;
            let result = Box::pin(crate::ssh::ssh_connect(
                state.clone(),
                host,
                user,
                port,
                identity_file,
                label,
            ))
            .await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "SshExec" => {
            let session_id = json_str(&args, "session");
            let command = json_str(&args, "command");
            if command.trim().is_empty() {
                return Err("SshExec requires a non-empty command.".to_string());
            }
            let cwd = json_str_opt(&args, "cwd");
            let timeout_secs = args.get("timeoutSecs").and_then(serde_json::Value::as_u64);
            require_tool_approval(
                app,
                turn_id,
                tc,
                effective_approval_mode,
                interactive,
                "SshExec",
                &format!(
                    "Run on SSH session: {}",
                    command.chars().take(80).collect::<String>()
                ),
                &command.chars().take(200).collect::<String>(),
                "execute",
                &input.tool_permission_rules,
                &command,
                false,
            )
            .await?;
            let result = Box::pin(crate::ssh::ssh_exec(
                state.clone(),
                session_id,
                command,
                cwd,
                timeout_secs,
            ))
            .await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "SshTransfer" => {
            let session_id = json_str(&args, "session");
            let direction_raw = json_str(&args, "direction").to_ascii_lowercase();
            let direction = match direction_raw.as_str() {
                "upload" => lux_ssh::TransferDirection::Upload,
                "download" => lux_ssh::TransferDirection::Download,
                _ => {
                    return Err(
                        "SshTransfer direction must be \"upload\" or \"download\".".to_string()
                    )
                }
            };
            let local_path = json_str(&args, "localPath");
            let remote_path = json_str(&args, "remotePath");
            let recursive = args.get("recursive").and_then(serde_json::Value::as_bool);
            require_tool_approval(
                app,
                turn_id,
                tc,
                effective_approval_mode,
                interactive,
                "SshTransfer",
                &format!("scp {direction_raw}: {local_path} <-> {remote_path}"),
                &format!("{local_path}  {remote_path}"),
                "execute",
                &input.tool_permission_rules,
                &format!("{local_path} {remote_path}"),
                false,
            )
            .await?;
            let result = Box::pin(crate::ssh::ssh_transfer(
                state.clone(),
                session_id,
                direction,
                local_path,
                remote_path,
                recursive,
            ))
            .await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "SshList" => {
            let result = Box::pin(crate::ssh::ssh_list(state.clone())).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }
        "SshDisconnect" => {
            let session_id = json_str_opt(&args, "session");
            let all = args.get("all").and_then(serde_json::Value::as_bool);
            let result = crate::ssh::ssh_disconnect(state.clone(), session_id, all)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        }

        "TestHealth" => {
            // F10: TestHealth runs project test/build commands — gate it like Shell
            // so it cannot execute project scripts from read-only modes without approval.
            let root = crate::workspace_root(state)?;
            let root_str = root.to_string_lossy().to_string();
            require_tool_approval(
                app,
                turn_id,
                tc,
                effective_approval_mode,
                interactive,
                "TestHealth",
                "Run workspace test health check",
                &root_str,
                "execute",
                &input.tool_permission_rules,
                &root_str,
                false,
            )
            .await?;
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
            // F1: Classify every browser tool by side-effect risk. Previously only
            // 5 tools were gated; BrowserInvoke (raw CLI escape hatch), BrowserDoctor
            // with --fix, and BrowserScreenshot with a write path were ungated.
            let browser_is_side_effecting = match tc.name.as_str() {
                // Always side-effecting: opens connections, mutates pages, installs.
                // BrowserInvoke is a raw CLI escape hatch — also always side-effecting.
                "BrowserOpen" | "BrowserAct" | "BrowserClose" | "BrowserChat"
                | "BrowserInstall" | "BrowserInvoke" => true,
                // BrowserDoctor with --fix runs repair commands; read-only without it.
                "BrowserDoctor" => args
                    .get("fix")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                // BrowserScreenshot with a file path writes to disk.
                "BrowserScreenshot" => args.get("path").and_then(|v| v.as_str()).is_some(),
                // BrowserSnapshot, BrowserStatus, BrowserDashboard, BrowserHelp are read-only.
                _ => false,
            };
            if browser_is_side_effecting {
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
                    &input.tool_permission_rules,
                    &browser_args.join(" "),
                    false,
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
            // Run the CLI in the workspace root so a relative/default screenshot path
            // lands in a writable place instead of the app's (often read-only) launch
            // dir — the "BrowserScreenshot: access denied" bug.
            let browser_cwd = crate::workspace_root(state)
                .ok()
                .map(|r| r.to_string_lossy().into_owned());
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
                    cwd: browser_cwd,
                })
                .await?;
            // Parity with the TS runtime (aiRuntimeBrowser.ts), which throws on
            // `!result.success`: a failed CLI run (non-zero exit with stdout, or a
            // `success:false` JSON envelope) must reach the model as an Err — not as
            // a "successful" tool result whose failure is buried in a JSON field.
            if result.success {
                serde_json::to_string(&result).map_err(|e| e.to_string())
            } else {
                let detail = result.text.trim();
                Err(if detail.is_empty() {
                    format!("{} failed (exit {:?}).", tc.name, result.exit_code)
                } else {
                    format!("{} failed (exit {:?}): {detail}", tc.name, result.exit_code)
                })
            }
        }

        // ── Orchestration tools (session state in Rust) ──
        "Goal" => {
            // Only `goal` is persisted (ai_session has no progress/status/summary
            // state). Previously the arm parsed progress/status/summary and echoed
            // them straight back, so the model believed they were recorded when
            // nothing stored them. Surface only what actually persists.
            if let Some(g) = json_str_opt(&args, "goal") {
                crate::ai_session::set_goal(&input.session_id, &g);
            }
            let current = crate::ai_session::get_goal(&input.session_id);
            Ok(serde_json::json!({
                "goal": if current.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(current)
                },
            })
            .to_string())
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
            // F2: check emit success before awaiting; a missing frontend card would
            // otherwise cause the turn to hang forever.
            let rx = register_question(turn_id, &tc.id);
            let _ = emit_turn_event(
                app,
                &TurnEvent::StatusChange {
                    turn_id: turn_id.to_string(),
                    phase: "waiting-approval".to_string(),
                },
            );
            if let Err(emit_err) = emit_turn_event(
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
            ) {
                cancel_questions_for_turn(turn_id);
                return Err(format!(
                    "AskUser question could not be delivered to the UI ({emit_err}); question skipped."
                ));
            }
            // Timeout prevents deadlock when the card is destroyed mid-turn (F2).
            const QUESTION_TIMEOUT: std::time::Duration = std::time::Duration::from_mins(5);
            match tokio::time::timeout(QUESTION_TIMEOUT, rx).await {
                Ok(Ok(answer)) if !answer.cancelled && !answer.answer.trim().is_empty() => {
                    Ok(serde_json::json!({ "answer": answer.answer }).to_string())
                }
                Ok(Ok(_)) => Ok(serde_json::json!({
                    "answer": "",
                    "dismissed": true,
                    "note": "User dismissed the question without answering. Proceed with your best judgment or ask again only if truly blocked."
                })
                .to_string()),
                Ok(Err(_)) => Err("AskUser channel closed before an answer arrived.".to_string()),
                Err(_elapsed) => {
                    cancel_questions_for_turn(turn_id);
                    Ok(serde_json::json!({
                        "answer": "",
                        "dismissed": true,
                        "note": "AskUser timed out waiting for a response. Proceed with your best judgment."
                    })
                    .to_string())
                }
            }
        }

        // Present a structured, reviewable plan. The UI renders an expandable plan
        // card; in Agent/Plan mode a "Start" button hands it to execution, in
        // Automatic mode execution auto-starts (the model proceeds immediately).
        "PresentPlan" => {
            let title = json_str_opt(&args, "title").unwrap_or_else(|| "Plan".to_string());
            let summary = json_str_opt(&args, "summary").unwrap_or_default();
            let mut steps: Vec<PlanStep> = args
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
                        .collect()
                })
                .unwrap_or_default();
            if steps.is_empty() {
                return Err("PresentPlan requires at least one step (array of strings or { title, detail, file }).".to_string());
            }
            // Inputs are capped, but the caller was never told when items were
            // dropped (stepCount reported the post-truncation length). Capture the
            // submitted count + a truncated flag, then truncate, so the response can
            // signal the loss instead of silently shrinking the pinned task list.
            const STEP_CAP: usize = 40;
            let steps_submitted = steps.len();
            let steps_truncated = steps_submitted > STEP_CAP;
            steps.truncate(STEP_CAP);
            // Structured reasoning phases (think-mcp parity): the key decision(s),
            // the failure modes, and the verification checks. Each accepts strings
            // or objects and is optional — the gate only expects them on risky work.
            let mut alternatives: Vec<PlanDecision> = args
                .get("alternatives")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| {
                            if let Some(option) = v.as_str() {
                                let option = option.trim();
                                if option.is_empty() {
                                    return None;
                                }
                                return Some(PlanDecision {
                                    option: option.to_string(),
                                    tradeoff: String::new(),
                                });
                            }
                            let option = v.get("option")?.as_str()?.trim().to_string();
                            if option.is_empty() {
                                return None;
                            }
                            Some(PlanDecision {
                                option,
                                tradeoff: v
                                    .get("tradeoff")
                                    .and_then(|d| d.as_str())
                                    .unwrap_or("")
                                    .trim()
                                    .to_string(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            const ALTERNATIVE_CAP: usize = 8;
            let alternatives_submitted = alternatives.len();
            let alternatives_truncated = alternatives_submitted > ALTERNATIVE_CAP;
            alternatives.truncate(ALTERNATIVE_CAP);
            const RISK_CAP: usize = 12;
            const VERIFICATION_CAP: usize = 12;
            let mut risks: Vec<String> = json_str_array(&args, "risks", usize::MAX);
            let risks_submitted = risks.len();
            let risks_truncated = risks_submitted > RISK_CAP;
            risks.truncate(RISK_CAP);
            let mut verification: Vec<String> = json_str_array(&args, "verification", usize::MAX);
            let verification_submitted = verification.len();
            let verification_truncated = verification_submitted > VERIFICATION_CAP;
            verification.truncate(VERIFICATION_CAP);
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

            // Deterministic plan-quality gate (ported from think-mcp's cycle gate):
            // score the proposed plan on the dimensions the prompt asks for and fold
            // coaching nudges into the tool result. Advisory, never blocking — in
            // Automatic the plan auto-starts, so a hard gate would stall execution;
            // instead the model sees concrete gaps and can self-correct in-flight.
            let (quality, coaching) = assess_plan_quality(
                &title,
                &summary,
                &steps,
                &alternatives,
                &risks,
                &verification,
            );

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
                    alternatives,
                    risks,
                    verification,
                    quality,
                    coaching: coaching.clone(),
                    auto_start,
                },
            );
            let base_guidance = if auto_start {
                "Plan presented and auto-started (Automatic mode). Begin executing step 1 now; do not wait for confirmation."
            } else {
                "Plan presented to the user. Stop here and wait — the user will press Start to hand the plan to Agent mode for execution. Do not begin editing yet."
            };
            // Prepend coaching so the model addresses gaps — on the next step in
            // Automatic, or by revising the plan before the user starts it otherwise.
            let mut guidance = if coaching.is_empty() {
                base_guidance.to_string()
            } else {
                format!(
                    "Plan quality {:.2}/1.0 — strengthen before/while executing: {}\n{base_guidance}",
                    quality,
                    coaching.join(" ")
                )
            };
            // Tell the model when its plan was clipped so it can re-submit tighter or
            // split it — the pinned task list only holds the kept steps.
            if steps_truncated
                || alternatives_truncated
                || risks_truncated
                || verification_truncated
            {
                guidance.push_str(
                    "\nNote: some inputs exceeded their caps and were truncated (the pinned \
                     task list holds only the kept items). Re-submit a tighter plan, or split \
                     it, if the dropped items matter.",
                );
            }
            Ok(serde_json::json!({
                "stepCount": steps.len(),
                "stepsSubmitted": steps_submitted,
                "stepsTruncated": steps_truncated,
                "alternativesSubmitted": alternatives_submitted,
                "alternativesTruncated": alternatives_truncated,
                "risksSubmitted": risks_submitted,
                "risksTruncated": risks_truncated,
                "verificationSubmitted": verification_submitted,
                "verificationTruncated": verification_truncated,
                "autoStart": auto_start,
                "quality": quality,
                "coaching": coaching,
                "guidance": guidance,
            })
            .to_string())
        }

        "ActiveContext" => {
            let workspace = crate::workspace_root(state).ok();
            let include_active_text = args
                .get("includeActiveText")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let documents = state.documents.lock().map_err(|e| e.to_string())?;
            // Capture the true open-document count BEFORE truncating to the cap,
            // so the model is told the real total (mirrors DiagnosticsContext).
            let snaps = documents.snapshots();
            let open_document_count = snaps.len();
            let active_path = input.active_document_path.clone();
            // Honor the advertised `includeActiveText`: surface a bounded copy of the
            // active buffer's LIVE text (editor state, so unsaved edits are reflected —
            // which a disk read via ReadFile would miss). The borrow ends once the
            // owned (String, bool) is produced, before `snaps.into_iter()` below.
            let active_text = if include_active_text {
                active_path
                    .as_deref()
                    .and_then(|active| {
                        snaps.iter().find(|doc| {
                            doc.path
                                .as_ref()
                                .is_some_and(|p| p.to_string_lossy() == active)
                        })
                    })
                    .map(|doc| {
                        const ACTIVE_TEXT_CHAR_LIMIT: usize = 20_000;
                        let text: String = doc.text.chars().take(ACTIVE_TEXT_CHAR_LIMIT).collect();
                        let truncated = text.len() < doc.text.len();
                        (text, truncated)
                    })
            } else {
                None
            };
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
            Ok(serde_json::json!({
                "workspace": workspace.map(|w| serde_json::json!({ "root": w.to_string_lossy() })),
                "activeDocument": active_path,
                "activeDocumentText": active_text.as_ref().map(|(text, _)| text.as_str()),
                "activeDocumentTextTruncated": active_text.as_ref().map(|(_, truncated)| *truncated),
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
            let max_findings = json_usize(&args, "maxFindings", 50);
            let return_redacted = args
                .get("returnRedactedText")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let (findings, redacted) = scan_secrets(&text, max_findings, return_redacted);
            let status = if findings.is_empty() {
                "clean"
            } else {
                "findings"
            };
            // `redactedText` is present only when redaction was requested AND at
            // least one token matched; otherwise the field is omitted entirely.
            let redacted_value =
                redacted.map_or(serde_json::Value::Null, serde_json::Value::String);
            let mut result = serde_json::Map::new();
            result.insert("status".to_string(), serde_json::Value::from(status));
            result.insert(
                "findingCount".to_string(),
                serde_json::Value::from(findings.len()),
            );
            result.insert(
                "findings".to_string(),
                serde_json::to_value(&findings).map_err(|e| e.to_string())?,
            );
            if !redacted_value.is_null() {
                result.insert("redactedText".to_string(), redacted_value);
            }
            Ok(serde_json::Value::Object(result).to_string())
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
                category: category.clone(),
                limit,
                ..Default::default()
            };
            let hits =
                crate::memory::memory_search(app.clone(), state.clone(), query, Some(options))
                    .await?;
            // `category` is forwarded as an exact-match SQL filter, so a typo or an
            // empty label returns [] that is indistinguishable from "nothing stored".
            // When the result is empty AND a category was supplied AND it is absent
            // from the store's actual categories, surface the real categories so the
            // model can fix its filter instead of concluding no memory is relevant.
            if hits.is_empty() {
                if let Some(requested) = &category {
                    let existing: Vec<String> =
                        crate::memory::memory_stats(app.clone(), state.clone())
                            .await
                            .map(|s| s.by_category.into_iter().map(|c| c.category).collect())
                            .unwrap_or_default();
                    if !existing.is_empty() && !existing.iter().any(|c| c == requested) {
                        return serde_json::to_string(&serde_json::json!({
                            "memories": hits,
                            "note": format!(
                                "no memories in category '{requested}'; existing categories: {}",
                                existing.join(", ")
                            ),
                        }))
                        .map_err(|e| e.to_string());
                    }
                }
            }
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
            // Dedup: the store mints a fresh uuid on every call, so re-remembering
            // the same fact silently piles up duplicate rows that later pollute
            // recall. Probe for a byte-identical (after trim) memory in the same
            // category and short-circuit with a `duplicate` signal so the model gets
            // the feedback loop it needs to stop re-saving known facts. Only exact
            // trimmed-content matches dedup; every genuinely new fact still inserts.
            let probe = lux_memory::SearchOptions {
                category: Some(category.clone()),
                limit: 1,
                touch: false,
                ..Default::default()
            };
            if let Ok(existing) = crate::memory::memory_search(
                app.clone(),
                state.clone(),
                content.clone(),
                Some(probe),
            )
            .await
            {
                if let Some(hit) = existing.into_iter().next() {
                    if hit.lexical >= 0.999 && hit.memory.content.trim() == content.trim() {
                        return serde_json::to_string(&serde_json::json!({
                            "status": "duplicate",
                            "existingId": hit.memory.id,
                            "category": hit.memory.category,
                            "importance": hit.memory.importance,
                        }))
                        .map_err(|e| e.to_string());
                    }
                }
            }
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
                // Echo the effective (store-clamped to [0,1]) importance so the model
                // sees the applied weight rather than assuming its raw value stuck.
                "importance": memory.importance,
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
            // Distinguish "no slug passed" from "unknown slug": an empty/whitespace
            // slug otherwise produced `no skill named ` (blank name), which reads like
            // a formatting bug rather than actionable guidance.
            if slug.trim().is_empty() {
                return Ok(serde_json::json!({
                    "error": "slug is required; call ListSkills to get a valid slug"
                })
                .to_string());
            }
            // A11: use the enabled-aware loader so a DISABLED skill is not returned as
            // authoritative instructions — it resolves to None here (reported as "no
            // skill named …"), while the Settings UI keeps using skills_get.
            match crate::skills::skill_for_use(app.clone(), state.clone(), slug.clone())? {
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
            // FailureAnalyzer: TestHealth (project test commands) + diagnostics → analysis.
            // F10: gate the test-health branch behind approval, matching the Shell
            // safety contract. Allow the user to opt out with `includeTestHealth:false`.
            let include_test_health = args
                .get("includeTestHealth")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            // Honor the advertised params the arm previously dropped: `log` (the raw
            // failing output the model pasted to be analyzed), `includeDiagnostics`
            // (noise suppression), and `maxFindings` (wider/narrower view).
            let provided_log = json_str_opt(&args, "log");
            let include_diagnostics = args
                .get("includeDiagnostics")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            let max_findings = json_usize(&args, "maxFindings", 40).clamp(1, 500);
            let root = crate::workspace_root(state).ok();
            let test_result = if include_test_health {
                if let Some(ref root) = root {
                    let root_str = root.to_string_lossy().to_string();
                    // Gate test-health execution through the normal approval flow.
                    if let Err(e) = require_tool_approval(
                        app,
                        turn_id,
                        tc,
                        effective_approval_mode,
                        interactive,
                        "FailureAnalyzer/TestHealth",
                        "Run workspace test health check (FailureAnalyzer)",
                        &root_str,
                        "execute",
                        &input.tool_permission_rules,
                        &root_str,
                        false,
                    )
                    .await
                    {
                        // If approval is denied / rejected, skip the test-health step
                        // but still return diagnostics — partial analysis is useful.
                        let denied_diag_count = if include_diagnostics {
                            crate::lsp::diagnostics_snapshot(state.clone())
                                .unwrap_or_default()
                                .len()
                        } else {
                            0
                        };
                        // This branch diverges (returns), so moving `provided_log`
                        // here does not conflict with the final return's move.
                        return Ok(serde_json::json!({
                            "testHealth": serde_json::Value::Null,
                            "testHealthSkipped": true,
                            "testHealthSkipReason": e,
                            "providedLog": provided_log,
                            "diagnosticCount": denied_diag_count,
                            "notes": ["TestHealth step was not approved; diagnostics only."],
                        })
                        .to_string());
                    }
                    crate::test_health::run(root.clone()).await.ok()
                } else {
                    None
                }
            } else {
                None
            };
            let diagnostics = if include_diagnostics {
                crate::lsp::diagnostics_snapshot(state.clone()).unwrap_or_default()
            } else {
                Vec::new()
            };
            let diagnostic_count = diagnostics.len();
            Ok(serde_json::json!({
                "testHealth": test_result,
                // Surface the pasted failing output back in the payload so `log` is
                // an analyzed input instead of being silently discarded.
                "providedLog": provided_log,
                "diagnosticCount": diagnostic_count,
                "diagnostics": diagnostics.into_iter().take(max_findings).collect::<Vec<_>>(),
                "notes": ["Analyze the provided failing output (providedLog), tests, and diagnostics above to identify root causes."],
            })
            .to_string())
        }

        "ImpactAnalysis" => {
            let query = json_str_opt(&args, "query").unwrap_or_default();
            let path = json_str_opt(&args, "path").or_else(|| input.active_document_path.clone());
            // ai_related_files clamps maxResults to 1..=120; surface the clip so a
            // model requesting 500 isn't misled that a small result set is the true
            // blast radius rather than a clipped one.
            const MAX_AFFECTED_FILES: usize = 120;
            let requested_max = json_usize(&args, "maxResults", 32);
            let max_results = requested_max.clamp(1, MAX_AFFECTED_FILES);
            let mut notes: Vec<String> = Vec::new();
            if requested_max > MAX_AFFECTED_FILES {
                notes.push(format!(
                    "maxResults {requested_max} exceeds the affected-files ceiling; capped at {MAX_AFFECTED_FILES} (governs affectedFiles only; symbol coverage is fixed)."
                ));
            }
            // Compose: RelatedFiles + diagnostics + symbols. Surface backend errors
            // instead of collapsing them into null — for a risk tool, "the analysis
            // failed" must be distinguishable from "nothing is affected".
            let mut had_error = false;
            let related = match crate::ai_related::ai_related_files(
                state.clone(),
                path.clone(),
                Some(query.clone()),
                Some(max_results),
                Some(5000),
            )
            .await
            {
                Ok(r) => Some(r),
                Err(e) => {
                    had_error = true;
                    notes.push(format!("related-files analysis failed: {e}"));
                    None
                }
            };
            let diagnostics = crate::lsp::diagnostics_snapshot(state.clone()).unwrap_or_default();
            let symbols = if query.is_empty() {
                None
            } else {
                match crate::ai_tools::ai_symbol_context(
                    state.clone(),
                    Some(query.clone()),
                    path.clone().map(std::path::PathBuf::from),
                    None,
                    None,
                    Some(40),
                )
                .await
                {
                    Ok(s) => Some(s),
                    Err(e) => {
                        had_error = true;
                        notes.push(format!("symbol-context analysis failed: {e}"));
                        None
                    }
                }
            };
            let diag_count = diagnostics.len();
            let risk = if diag_count > 10 {
                "high"
            } else if diag_count > 0 {
                "medium"
            } else {
                "low"
            };
            if had_error {
                notes.insert(
                    0,
                    "Some analyses errored; null affectedFiles/symbols mean the computation failed, not an empty blast radius.".to_string(),
                );
            }
            Ok(serde_json::json!({
                "target": path,
                "query": query,
                "riskLevel": risk,
                "affectedFiles": related,
                "symbols": symbols,
                "diagnosticCount": diag_count,
                "diagnostics": diagnostics.into_iter().take(24).collect::<Vec<_>>(),
                "notes": notes,
            })
            .to_string())
        }

        "TerminalContext" => {
            // Terminal session + output state is buffered in React; passed through
            // TurnInput. Honor the advertised sessionId (scope to one session) and
            // maxChars (clip each output buffer's tail) instead of echoing the whole
            // unbounded blob.
            let Some(ctx) = input.terminal_context.as_ref() else {
                return Ok(serde_json::json!({
                    "sessionCount": 0,
                    "sessions": [],
                    "notes": ["No terminal context was provided for this turn."],
                })
                .to_string());
            };
            let session_filter = json_str_opt(&args, "sessionId");
            // Default 12k, clamped to a sane window so a huge value can't defeat the clip.
            let max_chars = json_usize(&args, "maxChars", 12_000).clamp(1, 100_000);

            let active_terminal_id = ctx
                .get("activeTerminalId")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let output_buffers = ctx.get("outputBuffers").and_then(|v| v.as_object());
            let raw_sessions = ctx
                .get("sessions")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            // Clip a buffer's text to the tail (most recent output) up to max_chars.
            let clip_tail = |text: &str| -> String {
                let count = text.chars().count();
                if count <= max_chars {
                    text.to_string()
                } else {
                    text.chars().skip(count - max_chars).collect()
                }
            };

            // Determine which session ids to report. When a filter is given, keep only
            // that one; otherwise keep every session present in the context.
            let session_ids: Vec<String> = {
                let mut ids: Vec<String> = raw_sessions
                    .iter()
                    .filter_map(|s| {
                        s.get("id")
                            .or_else(|| s.get("sessionId"))
                            .and_then(|v| v.as_str())
                            .map(str::to_string)
                    })
                    .collect();
                // Fall back to buffer keys if the sessions array carried no ids.
                if ids.is_empty() {
                    if let Some(buffers) = output_buffers {
                        ids = buffers.keys().cloned().collect();
                    }
                }
                match &session_filter {
                    Some(want) => ids.into_iter().filter(|id| id == want).collect(),
                    None => ids,
                }
            };

            let sessions: Vec<serde_json::Value> = session_ids
                .iter()
                .map(|id| {
                    let meta = raw_sessions.iter().find(|s| {
                        s.get("id")
                            .or_else(|| s.get("sessionId"))
                            .and_then(|v| v.as_str())
                            == Some(id.as_str())
                    });
                    let text = output_buffers
                        .and_then(|b| b.get(id.as_str()))
                        .and_then(|buf| buf.get("text"))
                        .and_then(|v| v.as_str())
                        .map(clip_tail)
                        .unwrap_or_default();
                    serde_json::json!({
                        "id": id,
                        "active": active_terminal_id.as_deref() == Some(id.as_str()),
                        "title": meta.and_then(|m| m.get("title")).and_then(|v| v.as_str()),
                        "cwd": meta.and_then(|m| m.get("cwd")).and_then(|v| v.as_str()),
                        "output": text,
                    })
                })
                .collect();

            // A sessionId filter that matched nothing is worth flagging to the model.
            let mut notes: Vec<String> = Vec::new();
            if let Some(want) = &session_filter {
                if sessions.is_empty() {
                    notes.push(format!("No terminal session matched sessionId '{want}'."));
                }
            }
            Ok(serde_json::json!({
                "sessionCount": sessions.len(),
                "activeTerminalId": active_terminal_id,
                "maxChars": max_chars,
                "sessions": sessions,
                "notes": notes,
            })
            .to_string())
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
                &input.tool_permission_rules,
                &data,
                false,
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
            // `resume` is advertised but not implemented — do NOT silently drop it.
            // Reject any non-empty value so the model knows to start fresh instead
            // of assuming a prior subagent context was reattached.
            if json_str_opt(&args, "resume").is_some() {
                return Err(
                    "Task resume is not yet supported; omit `resume` to start a fresh subagent."
                        .to_string(),
                );
            }
            let subagent_type = json_str_opt(&args, "subagent_type")
                .unwrap_or_else(|| "generalPurpose".to_string());
            // Optional per-subagent model override; falls back to the parent/session
            // model inside run_subagent when absent.
            let model_override = json_str_opt(&args, "model");
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
                model_override.as_deref(),
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
            // Honor the advertised knobs the arm previously ignored. Defaults
            // reproduce today's behavior (tool context on, no editor text).
            let include_active = args
                .get("includeActiveText")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let include_open = args
                .get("includeOpenDocuments")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let include_tools = args
                .get("includeToolContext")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            let max_items = json_usize(&args, "maxItems", 28).clamp(4, 80);
            // Compose ranked context from native tools, then budget-select by score.
            let mut items: Vec<(String, String, i64)> = Vec::new(); // (kind, content, score)
                                                                    // Editor buffers (active/open) pulled first; the lock is scoped so the
                                                                    // guard is dropped before any of the async context sources below (a
                                                                    // MutexGuard must not be held across an await).
            if include_active || include_open {
                if let Ok(documents) = state.documents.lock() {
                    let snaps = documents.snapshots();
                    let active = input.active_document_path.as_deref();
                    if include_active {
                        if let Some(active_path) = active {
                            if let Some(doc) = snaps.iter().find(|doc| {
                                doc.path
                                    .as_ref()
                                    .is_some_and(|p| p.to_string_lossy() == active_path)
                            }) {
                                let text: String = doc.text.chars().take(4_000).collect();
                                items.push((
                                    "active-text".into(),
                                    format!("{active_path}: {text}"),
                                    90,
                                ));
                            }
                        }
                    }
                    if include_open {
                        for doc in &snaps {
                            let path = doc
                                .path
                                .as_ref()
                                .map_or_else(String::new, |p| p.to_string_lossy().to_string());
                            // Skip the active buffer already added above.
                            if include_active && active == Some(path.as_str()) {
                                continue;
                            }
                            let excerpt: String = doc.text.chars().take(2_000).collect();
                            items.push(("open-document".into(), format!("{path}: {excerpt}"), 70));
                        }
                    }
                }
            }
            // Gate the ranked read-only tool sources behind `includeToolContext`.
            if include_tools {
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
            }
            // Rank by score desc, then budget-select.
            items.sort_by_key(|item| std::cmp::Reverse(item.2));
            let mut selected = Vec::new();
            let mut used = 0usize;
            for (kind, content, score) in items {
                // Honor `maxItems` as a hard cap on packet size alongside the
                // character budget.
                if used >= target_chars || selected.len() >= max_items {
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
                    &input.tool_permission_rules,
                    id.as_deref().unwrap_or("latest"),
                    false,
                )
                .await?;
            }
            let now_ms = chrono::Utc::now().timestamp_millis();
            let result = crate::ai_checkpoint::ai_checkpoint(
                app.clone(),
                state.clone(),
                action,
                id.clone(),
                // No session-scoping from turn loop; pass None so the global
                // workspace pool is used (caller can scope explicitly).
                None,
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
                // resolve_symbol returns ALL matches (exact → prefix → substring);
                // resolve_one only surfaced the first, hiding ambiguity from the
                // model. Keep the first as the primary and surface the rest.
                let matches = lux_codegraph::resolve_symbol(graph, &symbol);
                if let Some(first) = matches.first() {
                    let definitions: Vec<serde_json::Value> = matches
                        .iter()
                        .map(|n| serde_json::json!({"name": n.name, "file": n.file, "line": n.line}))
                        .collect();
                    let match_count = definitions.len();
                    let mut out = serde_json::json!({
                        "found": true,
                        "name": first.name,
                        "file": first.file,
                        "line": first.line,
                        "matchCount": match_count,
                        "definitions": definitions,
                    });
                    if match_count > 1 {
                        out["ambiguous"] = serde_json::json!(true);
                        out["note"] = serde_json::json!(format!(
                            "'{symbol}' matched {match_count} definitions (exact/prefix/substring); \"name/file/line\" is the best (first) match. See \"definitions\" for the rest."
                        ));
                    }
                    Ok(out)
                } else {
                    Ok(serde_json::json!({"found": false, "note": format!("No symbol matching '{symbol}' in the code graph.")}))
                }
            }).await?;
            Ok(result.to_string())
        }
        "CodeGraphCallers" => {
            let symbol = json_str(&args, "symbol");
            let result = crate::code_graph::with_index(state.inner(), |index| {
                let graph = index.graph();
                let matches = lux_codegraph::resolve_symbol(graph, &symbol);
                let Some(nr) = matches.first().cloned() else {
                    return Ok(serde_json::json!({"found": false, "note": format!("Unknown symbol: {symbol}")}));
                };
                let match_count = matches.len();
                let callers: Vec<serde_json::Value> = lux_codegraph::callers(graph, nr.node)
                    .into_iter()
                    .map(|r| serde_json::json!({"name": r.name, "file": r.file, "line": r.line}))
                    .collect();
                let mut out = serde_json::json!({
                    "symbol": nr.name,
                    "matchCount": match_count,
                    "ambiguous": match_count > 1,
                    "callers": callers,
                });
                if match_count > 1 {
                    // Surface the alternates in-band so the model can pick the right
                    // node (CodeGraphDefinition also collapses, so it cannot help).
                    let alternates: Vec<serde_json::Value> = matches.iter().take(6)
                        .map(|m| serde_json::json!({"name": m.name, "file": m.file, "line": m.line}))
                        .collect();
                    out["note"] = serde_json::json!(format!(
                        "'{symbol}' fuzzily matched {match_count} symbols (exact->prefix->substring, case-insensitive); using the first ('{}'). Re-query with a fully-qualified/exact name if this is the wrong one.", nr.name));
                    out["alternates"] = serde_json::json!(alternates);
                }
                Ok(out)
            }).await?;
            Ok(result.to_string())
        }
        "CodeGraphCallees" => {
            let symbol = json_str(&args, "symbol");
            let result = crate::code_graph::with_index(state.inner(), |index| {
                let graph = index.graph();
                let matches = lux_codegraph::resolve_symbol(graph, &symbol);
                let Some(nr) = matches.first().cloned() else {
                    return Ok(serde_json::json!({"found": false, "note": format!("Unknown symbol: {symbol}")}));
                };
                let match_count = matches.len();
                let callees: Vec<serde_json::Value> = lux_codegraph::callees(graph, nr.node)
                    .into_iter()
                    .map(|r| serde_json::json!({"name": r.name, "file": r.file, "line": r.line}))
                    .collect();
                let mut out = serde_json::json!({
                    "symbol": nr.name,
                    "matchCount": match_count,
                    "ambiguous": match_count > 1,
                    "callees": callees,
                });
                if match_count > 1 {
                    let alternates: Vec<serde_json::Value> = matches.iter().take(6)
                        .map(|m| serde_json::json!({"name": m.name, "file": m.file, "line": m.line}))
                        .collect();
                    out["note"] = serde_json::json!(format!(
                        "'{symbol}' fuzzily matched {match_count} symbols (exact->prefix->substring, case-insensitive); using the first ('{}'). Re-query with a fully-qualified/exact name if this is the wrong one.", nr.name));
                    out["alternates"] = serde_json::json!(alternates);
                }
                Ok(out)
            }).await?;
            Ok(result.to_string())
        }
        "CodeGraphExplain" => {
            let symbol = json_str(&args, "symbol");
            let result = crate::code_graph::with_index(state.inner(), |index| {
                let graph = index.graph();
                let matches = lux_codegraph::resolve_symbol(graph, &symbol);
                let Some(nr) = matches.first().cloned() else {
                    return Ok(serde_json::json!({"found": false, "note": format!("Unknown symbol: {symbol}")}));
                };
                let match_count = matches.len();
                let Some(expl) = lux_codegraph::explain(graph, nr.node) else {
                    return Ok(serde_json::json!({"found": false}));
                };
                let mut out = serde_json::json!({
                    "name": expl.node.name,
                    "kind": format!("{:?}", expl.kind).to_lowercase(),
                    "degree": expl.degree,
                    "totalConnections": expl.total_connections,
                    "matchCount": match_count,
                    "ambiguous": match_count > 1,
                    "connections": expl.connections.into_iter().map(|n| serde_json::json!({
                        "name": n.node.name,
                        "file": n.node.file,
                        "line": n.node.line,
                        "relation": format!("{:?}", n.relation).to_lowercase(),
                        "direction": format!("{:?}", n.direction).to_lowercase(),
                    })).collect::<Vec<_>>(),
                });
                if match_count > 1 {
                    let alternates: Vec<serde_json::Value> = matches.iter().take(6)
                        .map(|m| serde_json::json!({"name": m.name, "file": m.file, "line": m.line}))
                        .collect();
                    out["note"] = serde_json::json!(format!(
                        "'{symbol}' fuzzily matched {match_count} symbols (exact->prefix->substring, case-insensitive); using the first ('{}'). Re-query with a fully-qualified/exact name if this is the wrong one.", nr.name));
                    out["alternates"] = serde_json::json!(alternates);
                }
                Ok(out)
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
                    // Enrich god nodes with file+line (public accessors, mirroring
                    // NodeRef::of) so they are locatable/round-trippable — a bare name
                    // re-resolves to an arbitrary node for non-unique hub names.
                    "godNodes": gods.into_iter().map(|g| {
                        let (file, line) = graph.node(g.node).map_or_else(
                            || (String::new(), 0u32),
                            |data| (
                                graph.file_path(data.file).map(|p| p.display().to_string()).unwrap_or_default(),
                                data.name_span.start_row + 1,
                            ),
                        );
                        serde_json::json!({
                            "name": g.name,
                            "degree": g.degree,
                            "file": file,
                            "line": line,
                        })
                    }).collect::<Vec<_>>(),
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
    // Optional per-subagent model override (Task `model` arg). Falls back to the
    // parent/session model when absent.
    model_override: Option<&str>,
) -> Result<String, String> {
    const MAX_SUBAGENT_ROUNDS: usize = 16;
    let read_only = matches!(subagent_type, "codeReviewer" | "explorer");
    // Honor the Task `model` override; otherwise inherit the parent turn's model.
    let subagent_model: &str = model_override
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(parent.model.as_str());

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
    // F4: build the subagent's own allowlist so it cannot dispatch tools outside
    // its mode either (a read_only subagent must not execute Write/Shell/etc.).
    let subagent_allowed: std::collections::HashSet<String> = tools
        .iter()
        .filter_map(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .or_else(|| t.get("name"))
                .and_then(|n| n.as_str())
        })
        .map(str::to_string)
        .collect();

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
            "model": subagent_model,
            "messages": messages,
            "stream": true,
            "stream_options": { "include_usage": true },
            "tools": tools,
            "tool_choice": "auto",
        });
        // Subagents inherit the parent turn's reasoning effort.
        crate::ai_chat_backend::merge_reasoning(&mut payload, parent.reasoning.as_ref());
        crate::ai_chat_backend::apply_temperature(&mut payload, parent.reasoning.as_ref(), 0.2);
        let request = crate::ai_chat_backend::AiChatCompletionRequest::with_protocol(
            parent.base_url.clone(),
            parent.api_key.clone(),
            payload,
            parent.prompt_input.provider_protocol.clone(),
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
                Box::pin(execute_tool(
                    app,
                    state,
                    parent,
                    agent_id,
                    false,
                    child,
                    &subagent_allowed,
                ))
                .await
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
//
// The authoritative gate runs here for EVERY side-effecting tool on the native
// loop (C1/C2/H7): the user's deny/ask/allow rules are evaluated first — a Deny
// is a hard block even in full-access/automatic, an Allow skips the prompt, an
// Ask forces one. `permission_input` is the glob target (`deny:Write(*.env)`
// matches a path, `deny:Shell(curl *)` a command). `auto_approve` lets a call
// that is intrinsically safe (e.g. a read-only shell command) skip the prompt
// when no rule intervened, mirroring the TS `autoApproveOnDefault`.
//
// Approval context (tool, summary, preview, risk) is passed positionally; bundling into a
// struct would only shift the boilerplate to every call site without improving clarity.
/// The pure outcome of the approval gate, BEFORE any UI plumbing. Extracted from
/// [`require_tool_approval`] so the security-critical decision order — deny rules
/// beat every mode, allow rules skip prompts, non-interactive callers can never
/// silently run a gated tool — is unit-testable and regression-locked (the C1/C2
/// class of native-loop enforcement bypass must never silently return).
#[derive(Debug, Clone, PartialEq, Eq)]
enum ApprovalGate {
    /// A deny rule fired: hard-block with this error, in EVERY mode.
    Blocked(String),
    /// Allowed without a prompt (allow rule, intrinsically-safe call, or full-access).
    Allowed,
    /// Needs a prompt but the caller has no UI (subagent): reject with this error.
    RejectedNonInteractive(String),
    /// Must ask the user through the UI.
    Prompt,
}

/// Decide the approval gate for one tool call. Pure — no events, no waiting.
///
/// Order is load-bearing:
/// 1. Permission rules are authoritative and evaluated BEFORE mode — a deny
///    applies even in full-access/automatic mode; an explicit allow skips the prompt.
/// 2. Without a forced ask, an intrinsically-safe call (`auto_approve`) or
///    full-access mode auto-approves.
/// 3. Anything still needing a prompt auto-rejects for non-interactive callers
///    (subagents have no approval UI; waiting would deadlock the parent turn).
fn resolve_approval_gate(
    tool: &str,
    permission_input: &str,
    rules: &[String],
    approval_mode: &str,
    interactive: bool,
    auto_approve: bool,
) -> ApprovalGate {
    let ev = crate::ai_permissions::evaluate(tool, permission_input, rules);
    let force_ask = match ev.decision {
        crate::ai_permissions::PermissionDecision::Deny => {
            // F28: name the exact rule that fired so the user (and the model) can
            // see WHY the call was blocked instead of a generic message.
            return ApprovalGate::Blocked(ev.matched_rule.map_or_else(
                || format!("{tool} is blocked by a permission rule."),
                |rule| format!("{tool} is blocked by permission rule `{rule}`."),
            ));
        }
        crate::ai_permissions::PermissionDecision::Allow => return ApprovalGate::Allowed,
        crate::ai_permissions::PermissionDecision::Ask => true,
        crate::ai_permissions::PermissionDecision::Default => false,
    };
    if !force_ask && (auto_approve || approval_mode == "full-access") {
        return ApprovalGate::Allowed;
    }
    if !interactive {
        return ApprovalGate::RejectedNonInteractive(format!(
            "{tool} requires approval and is unavailable to subagents."
        ));
    }
    ApprovalGate::Prompt
}

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
    rules: &[String],
    permission_input: &str,
    auto_approve: bool,
) -> Result<(), String> {
    match resolve_approval_gate(
        tool,
        permission_input,
        rules,
        approval_mode,
        interactive,
        auto_approve,
    ) {
        ApprovalGate::Blocked(error) | ApprovalGate::RejectedNonInteractive(error) => {
            return Err(error);
        }
        ApprovalGate::Allowed => return Ok(()),
        ApprovalGate::Prompt => {}
    }
    // Emit approval request and wait for decision from UI.
    // F2: check emit success; if the event cannot be delivered there is no
    // frontend listener and awaiting forever would deadlock the turn. Clean up
    // the registered channel and return a recoverable error instead.
    let rx = register_approval(turn_id, &tc.id);
    if let Err(emit_err) = emit_turn_event(
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
    ) {
        // Drop the stale channel so it doesn't leak in approval_channels().
        cancel_approvals_for_turn(turn_id);
        return Err(format!(
            "{tool} approval could not be delivered to the UI ({emit_err}); tool skipped."
        ));
    }
    // Timeout prevents the turn from hanging forever when the frontend card is
    // missing or the window is closed mid-turn (F2 — deadlock guard).
    const APPROVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_mins(5);
    match tokio::time::timeout(APPROVAL_TIMEOUT, rx).await {
        Ok(Ok(ApprovalDecision::Approved)) => Ok(()),
        Ok(_) => Err(format!("{tool} was rejected by the user.")),
        Err(_elapsed) => {
            // Timeout: clean up and surface a recoverable error.
            cancel_approvals_for_turn(turn_id);
            Err(format!(
                "{tool} approval timed out after {}s. If the approval card disappeared, retry the action.",
                APPROVAL_TIMEOUT.as_secs()
            ))
        }
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
            // A41: `-s <sel>` and `--urls` are real agent-browser snapshot flags
            // (verified against the CLI) — honor the advertised args instead of
            // dropping them.
            if let Some(sel) = json_str_opt(args, "selector") {
                a.push("-s".to_string());
                a.push(sel);
            }
            if args
                .get("includeUrls")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                a.push("--urls".to_string());
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
            // `quiet` (default true) hides intermediate tool calls; `-q` is a global
            // flag the CLI parses before the subcommand (mirrors the TS runtime).
            let quiet = args
                .get("quiet")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            if quiet {
                vec![
                    "-q".to_string(),
                    "chat".to_string(),
                    instruction.to_string(),
                ]
            } else {
                vec!["chat".to_string(), instruction.to_string()]
            }
        }
        "BrowserDashboard" => {
            // The CLI only has `dashboard start|stop` (bare `dashboard` == start).
            // Map any other/absent action (e.g. the old "status"/"open") to `start`
            // so the model never emits an unknown subcommand.
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("start");
            let sub = if action.eq_ignore_ascii_case("stop") {
                "stop"
            } else {
                "start"
            };
            let mut a = vec!["dashboard".to_string(), sub.to_string()];
            // Honor the advertised `port` (`dashboard start --port <n>`).
            if let Some(port) = args.get("port").and_then(serde_json::Value::as_u64) {
                a.push("--port".to_string());
                a.push(port.to_string());
            }
            a
        }
        "BrowserInstall" => {
            let mut a = vec!["install".to_string()];
            // Honor `withDeps` (Linux: also install OS dependencies); previously the
            // arm returned a bare ["install"] and dropped the flag.
            if args
                .get("withDeps")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                a.push("--with-deps".to_string());
            }
            a
        }
        "BrowserHelp" => {
            // Route the advertised skill/allSkills params to the CLI's `skills`
            // subcommand (matching agent_browser::skills()); previously both were
            // dropped and the model only ever got generic help.
            let all_skills = args
                .get("allSkills")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let skill = args
                .get("skill")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if all_skills {
                vec!["skills".to_string(), "get".to_string(), "--all".to_string()]
            } else if let Some(name) = skill {
                vec![
                    "skills".to_string(),
                    "get".to_string(),
                    name.to_string(),
                    "--full".to_string(),
                ]
            } else if let Some(topic) = args
                .get("topic")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                // There is NO bare `help` subcommand (`agent-browser help` → "Unknown
                // command"); the `skills` content IS the help. Treat a topic as a skill.
                vec!["skills".to_string(), "get".to_string(), topic.to_string()]
            } else {
                // No topic: list the available skills (the discovery/help surface).
                vec!["skills".to_string()]
            }
        }
        "BrowserDoctor" => {
            let mut a = vec!["doctor".to_string()];
            // Honor `offline`/`quick` (the doctor CLI supports both) in addition to
            // `fix`; previously offline/quick were dropped.
            if args
                .get("offline")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                a.push("--offline".to_string());
            }
            if args
                .get("quick")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                a.push("--quick".to_string());
            }
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

/// High-signal credential markers for the dependency-free `SecretGuard` scan.
/// Each entry is `(marker, human description)`. The marker is matched as a
/// case-sensitive substring — these prefixes are distinctive enough that a
/// case-insensitive match would only add false positives.
const SECRET_MARKERS: &[(&str, &str)] = &[
    ("-----BEGIN OPENSSH", "OpenSSH private key"),
    ("-----BEGIN", "PEM private key block"),
    ("AKIA", "AWS access key id"),
    ("aws_secret_access_key", "AWS secret access key"),
    ("github_pat_", "GitHub fine-grained PAT"),
    ("ghp_", "GitHub personal access token"),
    ("xoxb-", "Slack bot token"),
    ("xoxp-", "Slack user token"),
    ("sk-", "OpenAI-style secret key"),
    ("AIza", "Google API key"),
];

/// One `SecretGuard` finding: which marker fired, a human description, and the
/// 1-based line it was found on.
#[derive(Serialize)]
struct SecretFinding {
    marker: String,
    description: String,
    line: usize,
}

/// Dependency-free substring scan for leaked credentials. Returns the findings
/// (capped at `max_findings`) and, when `return_redacted` is set, a copy of the
/// text with every matched credential token region replaced by `***REDACTED***`.
///
/// A "token region" spans from the marker start through the contiguous run of
/// credential-shaped characters that follows, so the whole secret is masked —
/// not just the marker prefix. `-----BEGIN` blocks only mask the marker line
/// itself (the surrounding key body is masked by masking its own `-----BEGIN`).
fn scan_secrets(
    text: &str,
    max_findings: usize,
    return_redacted: bool,
) -> (Vec<SecretFinding>, Option<String>) {
    /// Characters that continue a credential token past its marker.
    const fn is_token_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '+' | '=' | '.')
    }

    // Collect (start_byte, end_byte) match spans, longest-marker-first so a more
    // specific marker (e.g. `-----BEGIN OPENSSH`) is preferred over its prefix.
    let mut spans: Vec<(usize, usize, &'static str, &'static str)> = Vec::new();
    for (marker, description) in SECRET_MARKERS {
        let mut search_from = 0usize;
        while let Some(rel) = text.get(search_from..).and_then(|hay| hay.find(marker)) {
            let start = search_from + rel;
            // Extend past the marker over contiguous token characters to cover the
            // full secret value (bounded by whitespace / end of string).
            let mut end = start + marker.len();
            for c in text[end..].chars() {
                if is_token_char(c) {
                    end += c.len_utf8();
                } else {
                    break;
                }
            }
            spans.push((start, end, marker, description));
            // Advance past this match; guaranteed progress since end > start.
            search_from = end;
        }
    }
    // Sort by start, then by longest span first so overlapping matches (a marker
    // that is a prefix of another) collapse to the most specific one.
    spans.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    // Deduplicate overlaps: keep the first (most specific) span at each position.
    let mut kept: Vec<(usize, usize, &'static str, &'static str)> = Vec::new();
    let mut covered_to = 0usize;
    for span in spans {
        if span.0 >= covered_to {
            covered_to = span.1;
            kept.push(span);
        }
    }

    // Precompute line-start byte offsets so a match byte can map to a 1-based line.
    let mut line_starts: Vec<usize> = vec![0];
    for (idx, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            line_starts.push(idx + 1);
        }
    }
    let line_of = |byte_pos: usize| -> usize {
        // partition_point → count of line starts at or before byte_pos == 1-based line.
        line_starts
            .partition_point(|&start| start <= byte_pos)
            .max(1)
    };

    let findings: Vec<SecretFinding> = kept
        .iter()
        .take(max_findings)
        .map(|(start, _end, marker, description)| SecretFinding {
            marker: (*marker).to_string(),
            description: (*description).to_string(),
            line: line_of(*start),
        })
        .collect();

    let redacted = if return_redacted && !kept.is_empty() {
        // Rebuild the text, replacing each kept span with the redaction marker.
        // Spans are already sorted by start and non-overlapping.
        let mut out = String::with_capacity(text.len());
        let mut cursor = 0usize;
        for (start, end, _marker, _description) in &kept {
            if *start >= cursor {
                out.push_str(&text[cursor..*start]);
                out.push_str("***REDACTED***");
                cursor = *end;
            }
        }
        out.push_str(&text[cursor..]);
        Some(out)
    } else {
        None
    };

    (findings, redacted)
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

/// Collect a JSON string array at `key`, trimming, dropping empties, capped at `max`.
fn json_str_array(value: &serde_json::Value, key: &str, max: usize) -> Vec<String> {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .take(max)
                .collect()
        })
        .unwrap_or_default()
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

    fn step(title: &str, detail: &str, file: &str) -> PlanStep {
        PlanStep {
            title: title.to_string(),
            detail: detail.to_string(),
            file: file.to_string(),
        }
    }

    fn decision(option: &str, tradeoff: &str) -> PlanDecision {
        PlanDecision {
            option: option.to_string(),
            tradeoff: tradeoff.to_string(),
        }
    }

    #[test]
    fn parse_reads_reasoning_when_content_empty() {
        // A reasoning model can finish a trivial prompt with empty content and only
        // thinking text. parse_assistant_message must expose that reasoning so the
        // turn can fall back to it instead of surfacing a bare "no answer".
        let body = serde_json::json!({
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "", "reasoning_content": "Hi there" },
                "finish_reason": "stop",
            }],
        });
        let parsed = parse_assistant_message(&body);
        assert!(parsed.content.is_empty());
        assert_eq!(parsed.reasoning, "Hi there");
    }

    #[test]
    fn parse_reads_alternate_reasoning_field() {
        let body = serde_json::json!({
            "choices": [{
                "message": { "role": "assistant", "content": serde_json::Value::Null, "reasoning": "thinking..." },
            }],
        });
        let parsed = parse_assistant_message(&body);
        assert_eq!(parsed.reasoning, "thinking...");
    }

    #[test]
    fn plan_gate_flags_vague_and_missing_verification() {
        let steps = vec![
            step("Set up the project", "", ""),
            step("Implement business logic", "", ""),
            step("Add documentation", "", ""),
        ];
        let (quality, coaching) = assess_plan_quality("Build a thing", "", &steps, &[], &[], &[]);
        assert!(quality < 0.6, "vague plan should score low, got {quality}");
        assert!(coaching.iter().any(|c| c.contains("vague")));
        assert!(coaching.iter().any(|c| c.contains("verification")));
    }

    #[test]
    fn plan_gate_passes_a_concrete_verified_plan() {
        let steps = vec![
            step(
                "Add SQLite models",
                "Define User/Sticker models in db/models.py with FK constraints",
                "db/models.py",
            ),
            step(
                "Wire moderation handler",
                "Add /ban command handler in handlers/moderation.py reading the banlist",
                "handlers/moderation.py",
            ),
            step(
                "Verify",
                "Run pytest -q and `python -m bot --dry-run`; both pass clean",
                "tests/test_moderation.py",
            ),
        ];
        let (quality, coaching) = assess_plan_quality(
            "Moderation bot",
            "aiogram + SQLAlchemy",
            &steps,
            &[],
            &[],
            &[],
        );
        assert!(
            quality >= 0.8,
            "concrete plan should score high, got {quality}: {coaching:?}"
        );
    }

    #[test]
    fn plan_gate_flags_missing_alternatives_and_critique_on_risky_work() {
        // Risk markers (auth/token) raise the bar: a plan with no named decision and
        // no failure-mode analysis must be coached on both, even if otherwise concrete.
        let steps = vec![
            step(
                "Add auth guard",
                "Validate the bearer token in auth/guard.rs",
                "auth/guard.rs",
            ),
            step(
                "Wire login route",
                "POST /login issues a token in auth/login.rs",
                "auth/login.rs",
            ),
            step("Verify", "cargo test auth:: passes", "auth/login.rs"),
        ];
        let (_quality, coaching) =
            assess_plan_quality("Add auth", "token-based login", &steps, &[], &[], &[]);
        assert!(
            coaching
                .iter()
                .any(|c| c.to_lowercase().contains("key decision")),
            "risky plan must nudge a key decision: {coaching:?}"
        );
        assert!(
            coaching
                .iter()
                .any(|c| c.to_lowercase().contains("failure mode")),
            "risky plan must nudge critique: {coaching:?}"
        );
    }

    #[test]
    fn plan_gate_rewards_full_five_phase_plan() {
        // Risky-enough work (auth) so alternatives + critique are expected, now with
        // a named decision, explicit risks, and verification — the complete 5-phase
        // plan should score high with no alternatives/critique coaching left.
        let steps = vec![
            step(
                "Add guard",
                "Validate the bearer credential in auth/guard.rs",
                "auth/guard.rs",
            ),
            step(
                "Wire login route",
                "POST /login issues a session in routes/login.rs",
                "routes/login.rs",
            ),
            step(
                "Hash storage",
                "Store argon2 hashes in db/users.rs",
                "db/users.rs",
            ),
            step(
                "Verify",
                "cargo test auth:: + manual login smoke",
                "routes/login.rs",
            ),
        ];
        let alternatives = vec![decision(
            "Stateless sessions",
            "Chosen over a shared store — simpler, at the cost of revocation latency",
        )];
        let risks = vec![
            "Replay if the clock skews — short TTL mitigates it".to_string(),
            "Assumes argon2 is present at build time".to_string(),
        ];
        let verification = vec![
            "cargo test auth:: passes".to_string(),
            "Checkpoint before deploy; revert on failure".to_string(),
        ];
        let (quality, coaching) = assess_plan_quality(
            "Add login",
            "session-based auth",
            &steps,
            &alternatives,
            &risks,
            &verification,
        );
        assert!(
            quality >= 0.8,
            "complete 5-phase plan should score high, got {quality}: {coaching:?}"
        );
        assert!(
            !coaching
                .iter()
                .any(|c| c.to_lowercase().contains("key decision")),
            "decision is named — no alternatives coaching expected: {coaching:?}"
        );
    }

    #[test]
    fn plan_gate_demands_rollback_for_high_risk() {
        let steps = vec![
            step(
                "Add auth migration",
                "Alter users schema; add password_hash",
                "migrations/004.sql",
            ),
            step(
                "Update login",
                "Hash + verify token in auth/login.rs",
                "auth/login.rs",
            ),
            step("Verify", "cargo test auth:: passes", "auth/login.rs"),
        ];
        let (_quality, coaching) =
            assess_plan_quality("Auth + payment migration", "", &steps, &[], &[], &[]);
        assert!(
            coaching
                .iter()
                .any(|c| c.to_lowercase().contains("rollback")),
            "high-risk plan must nudge rollback: {coaching:?}"
        );
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

    #[test]
    fn tool_names_from_defs_reads_both_shapes() {
        let defs = vec![
            serde_json::json!({ "type": "function", "function": { "name": "Read" } }),
            serde_json::json!({ "name": "Grep" }),
            serde_json::json!({ "function": { "description": "no name" } }),
        ];
        let names = tool_names_from_defs(&defs);
        assert!(names.contains("Read"));
        assert!(names.contains("Grep"));
        assert_eq!(names.len(), 2, "the nameless entry must be skipped");
    }

    #[test]
    fn forged_edit_tool_blocked_in_read_only_modes() {
        // F4: in plan/ask the runtime tool definitions never include edit/execute
        // tools, so the allowlist built from them excludes Write/StrReplace/Delete/
        // PatchEngine/Shell. A model (or compromised proxy) that forges such a call
        // is rejected before dispatch — mode safety is enforced at the Rust boundary,
        // not only via prompt/tool-definition shaping.
        for mode in ["plan", "ask"] {
            let defs = crate::ai_tool_defs::runtime_tool_definitions(mode, false);
            let allowed = tool_names_from_defs(&defs);
            for forged in [
                "Write",
                "StrReplace",
                "Delete",
                "PatchEngine",
                "Shell",
                "McpManage",
            ] {
                assert!(
                    !allowed.contains(forged),
                    "{forged} must NOT be in the {mode}-mode allowlist (forge guard)"
                );
            }
            // Read-only tools the mode does advertise stay allowed.
            assert!(
                allowed.contains("Read"),
                "Read must remain available in {mode}"
            );
        }
    }

    #[test]
    fn full_exec_modes_allow_edit_tools() {
        // Sanity counterpart: agent/automatic DO advertise the edit/execute tools,
        // so the allowlist permits them (the guard only blocks tools never sent).
        for mode in ["agent", "automatic"] {
            let defs = crate::ai_tool_defs::runtime_tool_definitions(mode, false);
            let allowed = tool_names_from_defs(&defs);
            assert!(
                allowed.contains("Write"),
                "Write expected in {mode} allowlist"
            );
            assert!(
                allowed.contains("StrReplace"),
                "StrReplace expected in {mode} allowlist"
            );
        }
    }

    // ── E6: SecretGuard actually scans ──

    #[test]
    fn secret_guard_flags_known_markers() {
        let text =
            "config:\naws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\nkey: AKIAIOSFODNN7EXAMPLE\ntoken: ghp_16CharsOfTokenHere0000\n";
        let (findings, redacted) = scan_secrets(text, 50, false);
        assert!(
            !findings.is_empty(),
            "known credential markers must be flagged"
        );
        // The AWS access-key id lives on line 3.
        let located: Vec<(String, usize)> = findings
            .iter()
            .map(|f| (f.marker.clone(), f.line))
            .collect();
        assert!(
            findings.iter().any(|f| f.marker == "AKIA" && f.line == 3),
            "AKIA finding must report its line: {located:?}"
        );
        assert!(
            findings.iter().any(|f| f.marker == "aws_secret_access_key"),
            "AWS secret marker must be flagged"
        );
        assert!(findings.iter().any(|f| f.marker == "ghp_"), "GitHub PAT");
        // No redaction requested → none returned.
        assert!(redacted.is_none());
    }

    #[test]
    fn secret_guard_passes_clean_text() {
        let text = "This is a perfectly ordinary paragraph with no credentials.\nJust prose.";
        let (findings, redacted) = scan_secrets(text, 50, true);
        assert!(findings.is_empty(), "clean text must produce no findings");
        // returnRedactedText=true but nothing matched → redacted stays None.
        assert!(redacted.is_none());
    }

    #[test]
    fn secret_guard_redacts_matched_tokens() {
        let text = "leak sk-abc123def456ghi in the middle";
        let (findings, redacted) = scan_secrets(text, 50, true);
        assert_eq!(findings.len(), 1);
        let out = redacted.expect("redacted text requested");
        assert!(
            out.contains("***REDACTED***"),
            "matched token must be masked: {out}"
        );
        assert!(
            !out.contains("sk-abc123def456ghi"),
            "the raw secret must not survive redaction: {out}"
        );
    }

    #[test]
    fn secret_guard_caps_findings() {
        let text = "AKIA1 AKIA2 AKIA3 AKIA4 AKIA5";
        let (findings, _) = scan_secrets(text, 2, false);
        assert_eq!(findings.len(), 2, "findings must be capped at maxFindings");
    }

    // ── E3: ReadLints severity match key ──

    #[test]
    fn diagnostic_severity_debug_name_is_lowercase_matchable() {
        // ReadLints compares `format!("{:?}", severity).to_ascii_lowercase()` against
        // the caller's severity filter, so the Debug name must match the words a
        // model would pass ("error", "warning", …).
        assert_eq!(
            format!("{:?}", lux_core::DiagnosticSeverity::Error).to_ascii_lowercase(),
            "error"
        );
        assert_eq!(
            format!("{:?}", lux_core::DiagnosticSeverity::Warning).to_ascii_lowercase(),
            "warning"
        );
        assert_eq!(
            format!("{:?}", lux_core::DiagnosticSeverity::Information).to_ascii_lowercase(),
            "information"
        );
        assert_eq!(
            format!("{:?}", lux_core::DiagnosticSeverity::Hint).to_ascii_lowercase(),
            "hint"
        );
    }

    // ── Browser args: advertised knobs now reach the CLI ──

    #[test]
    fn browser_args_chat_prepends_quiet_flag() {
        // A46: quiet defaults true → global `-q` before the subcommand.
        let default =
            build_browser_args("BrowserChat", &serde_json::json!({ "instruction": "hi" }));
        assert_eq!(default, vec!["-q", "chat", "hi"]);
        // Explicit quiet:false drops the flag.
        let loud = build_browser_args(
            "BrowserChat",
            &serde_json::json!({ "instruction": "hi", "quiet": false }),
        );
        assert_eq!(loud, vec!["chat", "hi"]);
    }

    #[test]
    fn browser_args_dashboard_appends_port() {
        // A42: port must reach the CLI (`dashboard <action> --port <n>`).
        let args = build_browser_args(
            "BrowserDashboard",
            &serde_json::json!({ "action": "start", "port": 5000 }),
        );
        assert_eq!(args, vec!["dashboard", "start", "--port", "5000"]);
        // Unknown/absent action (the CLI only has start|stop) maps to `start`.
        let plain = build_browser_args(
            "BrowserDashboard",
            &serde_json::json!({ "action": "status" }),
        );
        assert_eq!(plain, vec!["dashboard", "start"]);
        assert_eq!(
            build_browser_args("BrowserDashboard", &serde_json::json!({ "action": "stop" })),
            vec!["dashboard", "stop"]
        );
    }

    #[test]
    fn browser_args_install_honors_with_deps() {
        // A43: withDeps → `install --with-deps`; absent → bare `install`.
        assert_eq!(
            build_browser_args("BrowserInstall", &serde_json::json!({ "withDeps": true })),
            vec!["install", "--with-deps"]
        );
        assert_eq!(
            build_browser_args("BrowserInstall", &serde_json::json!({})),
            vec!["install"]
        );
    }

    #[test]
    fn browser_args_doctor_offline_quick_fix() {
        // A44: offline/quick/fix all appended in a stable order.
        let args = build_browser_args(
            "BrowserDoctor",
            &serde_json::json!({ "offline": true, "quick": true, "fix": true }),
        );
        assert_eq!(args, vec!["doctor", "--offline", "--quick", "--fix"]);
        // Default (no flags) is byte-identical to the old bare `doctor`.
        assert_eq!(
            build_browser_args("BrowserDoctor", &serde_json::json!({})),
            vec!["doctor"]
        );
    }

    #[test]
    fn browser_args_help_routes_skills() {
        // A45: allSkills / skill route to the `skills` subcommand; otherwise `help`.
        assert_eq!(
            build_browser_args("BrowserHelp", &serde_json::json!({ "allSkills": true })),
            vec!["skills", "get", "--all"]
        );
        assert_eq!(
            build_browser_args("BrowserHelp", &serde_json::json!({ "skill": "forms" })),
            vec!["skills", "get", "forms", "--full"]
        );
        // No bare `help` subcommand exists → base help lists skills; a topic is a skill.
        assert_eq!(
            build_browser_args("BrowserHelp", &serde_json::json!({ "skill": "  " })),
            vec!["skills"]
        );
        assert_eq!(
            build_browser_args("BrowserHelp", &serde_json::json!({ "topic": "nav" })),
            vec!["skills", "get", "nav"]
        );
    }

    #[test]
    fn context_budgeter_max_items_clamps_to_window() {
        // A1: maxItems defaults to 28 and is clamped to 4..=80, exactly as the
        // ContextBudgeter arm computes its packet cap.
        let cap = |v: serde_json::Value| json_usize(&v, "maxItems", 28).clamp(4, 80);
        assert_eq!(cap(serde_json::json!({})), 28);
        assert_eq!(cap(serde_json::json!({ "maxItems": 1 })), 4);
        assert_eq!(cap(serde_json::json!({ "maxItems": 200 })), 80);
        assert_eq!(cap(serde_json::json!({ "maxItems": 50 })), 50);
    }

    #[test]
    fn present_plan_str_array_keeps_all_then_caps() {
        // A38: json_str_array(usize::MAX) keeps every valid item so the arm can
        // measure the true submitted count before truncating to the cap.
        let big: Vec<String> = (0..20).map(|i| format!("risk {i}")).collect();
        let value = serde_json::json!({ "risks": big });
        let collected = json_str_array(&value, "risks", usize::MAX);
        assert_eq!(collected.len(), 20, "all submitted items are retained");
    }

    // ── Approval-gate regression suite (C1/C2/H7) ─────────────────────────────
    //
    // These lock the security-critical decision order of `resolve_approval_gate`.
    // If any of them starts failing, the native turn loop has re-opened the
    // permission-enforcement bypass class from the 2026-06 project review:
    // deny rules MUST beat every mode, and non-interactive callers (subagents)
    // MUST never silently run a gated tool.

    fn rules(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn approval_gate_deny_rule_blocks_even_in_full_access() {
        // C1: full-access mode must NOT override an explicit deny.
        let gate = resolve_approval_gate(
            "Write",
            "/repo/.env",
            &rules(&["deny:Write(*.env)"]),
            "full-access",
            true,
            false,
        );
        let ApprovalGate::Blocked(error) = gate else {
            panic!("deny rule must hard-block in full-access, got {gate:?}");
        };
        assert!(
            error.contains("deny:Write(*.env)"),
            "block message names the rule that fired: {error}"
        );
    }

    #[test]
    fn approval_gate_deny_rule_blocks_auto_approved_calls() {
        // An intrinsically-safe (auto_approve) call still loses to a deny rule.
        let gate = resolve_approval_gate(
            "Shell",
            "git push --force",
            &rules(&["deny:Shell(git push*)"]),
            "default",
            true,
            true,
        );
        assert!(
            matches!(gate, ApprovalGate::Blocked(_)),
            "auto_approve must not bypass deny, got {gate:?}"
        );
    }

    #[test]
    fn approval_gate_bash_alias_deny_blocks_shell_tool() {
        // Finding #7: users write `Bash` in rules; the native tool is `Shell`.
        let gate = resolve_approval_gate(
            "Shell",
            "rm -rf /",
            &rules(&["deny:Bash(rm *)"]),
            "full-access",
            true,
            false,
        );
        assert!(
            matches!(gate, ApprovalGate::Blocked(_)),
            "Bash-alias deny must match the Shell tool, got {gate:?}"
        );
    }

    #[test]
    fn approval_gate_allow_rule_skips_prompt_in_default_mode() {
        let gate = resolve_approval_gate(
            "Shell",
            "git status",
            &rules(&["allow:Shell(git *)"]),
            "default",
            true,
            false,
        );
        assert_eq!(gate, ApprovalGate::Allowed);
    }

    #[test]
    fn approval_gate_ask_rule_prompts_even_in_full_access() {
        // C2: an explicit ask must force the prompt regardless of mode or
        // auto-approve — rules are authoritative over both.
        let gate = resolve_approval_gate(
            "Shell",
            "rm build/cache",
            &rules(&["ask:Shell(rm *)"]),
            "full-access",
            true,
            true,
        );
        assert_eq!(gate, ApprovalGate::Prompt);
    }

    #[test]
    fn approval_gate_ask_rule_rejects_for_subagents() {
        // H7: a subagent (no approval UI) must get a hard rejection, never a
        // silent run and never a deadlocked wait.
        let gate = resolve_approval_gate(
            "Shell",
            "rm build/cache",
            &rules(&["ask:Shell(rm *)"]),
            "full-access",
            false,
            true,
        );
        assert!(
            matches!(gate, ApprovalGate::RejectedNonInteractive(_)),
            "ask + non-interactive must reject, got {gate:?}"
        );
    }

    #[test]
    fn approval_gate_default_full_access_allows() {
        let gate = resolve_approval_gate("Write", "src/main.rs", &[], "full-access", true, false);
        assert_eq!(gate, ApprovalGate::Allowed);
    }

    #[test]
    fn approval_gate_default_auto_approve_allows() {
        // Intrinsically-safe call (read-only shell) with no rule intervening.
        let gate = resolve_approval_gate("Shell", "git status", &[], "default", true, true);
        assert_eq!(gate, ApprovalGate::Allowed);
    }

    #[test]
    fn approval_gate_default_mode_prompts_interactive() {
        let gate = resolve_approval_gate("Write", "src/main.rs", &[], "default", true, false);
        assert_eq!(gate, ApprovalGate::Prompt);
    }

    #[test]
    fn approval_gate_default_mode_rejects_subagents() {
        // C1 regression shape: gated tool + no UI. Must be an error the caller
        // surfaces, not an implicit allow.
        let gate = resolve_approval_gate("Write", "src/main.rs", &[], "default", false, false);
        let ApprovalGate::RejectedNonInteractive(error) = gate else {
            panic!("gated tool without UI must reject, got {gate:?}");
        };
        assert!(error.contains("Write"), "error names the tool: {error}");
    }

    #[test]
    fn approval_gate_deny_beats_allow_for_same_call() {
        // Precedence: deny > allow even when both match.
        let gate = resolve_approval_gate(
            "Shell",
            "git push origin main",
            &rules(&["allow:Shell(git *)", "deny:Shell(git push*)"]),
            "default",
            true,
            false,
        );
        assert!(
            matches!(gate, ApprovalGate::Blocked(_)),
            "deny must beat allow, got {gate:?}"
        );
    }

    #[test]
    fn approval_gate_compound_shell_command_cannot_hide_denied_segment() {
        // Finding #25: `deny:Shell(rm *)` must fire on `ls && rm -rf /`.
        let gate = resolve_approval_gate(
            "Shell",
            "ls && rm -rf /",
            &rules(&["deny:Shell(rm *)"]),
            "full-access",
            true,
            true,
        );
        assert!(
            matches!(gate, ApprovalGate::Blocked(_)),
            "chained command must still hit the deny, got {gate:?}"
        );
    }

    #[test]
    fn approval_gate_basename_deny_matches_nested_path() {
        // Finding #26: `deny:Write(.env)` fires on a nested `config/.env`.
        let gate = resolve_approval_gate(
            "Write",
            "/repo/config/.env",
            &rules(&["deny:Write(.env)"]),
            "full-access",
            true,
            false,
        );
        assert!(
            matches!(gate, ApprovalGate::Blocked(_)),
            "basename deny must match any directory, got {gate:?}"
        );
    }
}
