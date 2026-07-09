use aspect_ai_core::*;

use super::run::ai_run_turn_inner;
use super::helpers::handle_turn_error;

/// Resolve a pending approval from the UI side.
#[tauri::command]
pub fn ai_resolve_turn_approval(
    turn_id: String,
    request_id: String,
    decision: ApprovalDecision,
) -> Result<(), String> {
    resolve_approval(&turn_id, &request_id, decision)
}

/// Resolve a pending question from the UI side (delivers the human answer).
#[tauri::command]
pub fn ai_resolve_turn_question(
    turn_id: String,
    request_id: String,
    answer: QuestionAnswer,
) -> Result<(), String> {
    resolve_question(&turn_id, &request_id, answer)
}

/// Start a native AI turn. Runs the full model↔tool loop in Rust,
/// emitting `aspect://ai-turn` events for the frontend to render.
#[tauri::command]
pub async fn ai_run_turn(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::SharedState>,
    input: TurnInput,
) -> Result<(), String> {
    let result = ai_run_turn_inner(&app, &state, input).await;
    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            handle_turn_error(&app, &e);
            Err(e)
        }
    }
}

/// Cancel a running native turn — signals stop and aborts pending approvals.
#[tauri::command]
pub fn ai_cancel_turn(turn_id: String) {
    mark_turn_cancelled(&turn_id);
    cancel_approvals_for_turn(&turn_id);
    cancel_questions_for_turn(&turn_id);
}

/// Cancel ONE running subagent (Task tool call) by its call id.
#[tauri::command]
pub fn ai_cancel_subagent(call_id: String) {
    mark_subagent_cancelled(&call_id);
}

/// Stage a user message for injection into a specific running turn.
#[tauri::command]
pub fn ai_inject_message(session_id: String, turn_id: String, text: String) {
    enqueue_injection(&session_id, &turn_id, text);
}

/// Emit a turn event to the frontend.
pub fn emit_turn_event(app: &tauri::AppHandle, event: &TurnEvent) -> Result<(), String> {
    use tauri::Emitter;
    app.emit("aspect://ai-turn", event).map_err(|e| e.to_string())
}

/// Map a backend `RetryNotice` onto a `TurnRetry` event for the active turn.
pub(crate) fn emit_retry_event(
    app: &tauri::AppHandle,
    turn_id: &str,
    notice: &crate::aspector::transport::RetryNotice,
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

pub use aspect_ai_core::TurnInput;
