use std::collections::HashSet;

use crate::types::{
    ParsedToolCall, RetryNotice, TurnEvent, TurnInput, ApprovalDecision, QuestionAnswer,
};

/// Environment abstraction for the turn loop.
///
/// Implemented by the desktop crate to bridge Tauri state and IO operations.
/// The core crate's turn runner calls through this trait instead of depending
/// on Tauri directly, keeping the domain logic pure and testable.
#[allow(clippy::too_many_arguments)]
pub trait TurnEnv {
    /// Emit a turn event to the frontend.
    fn emit_event(&self, event: &TurnEvent) -> Result<(), String>;

    /// Check if a turn has been cancelled.
    fn is_turn_cancelled(&self, turn_id: &str) -> bool;

    /// Clear the cancellation flag for a turn.
    fn clear_turn_cancelled(&self, turn_id: &str);

    /// Clear staged injections for a session/turn.
    fn clear_injections(&self, session_id: &str, turn_id: &str);

    /// Drain staged user messages for injection.
    fn drain_injections(&self, session_id: &str, turn_id: &str) -> Vec<String>;

    /// Get the workspace root path.
    fn workspace_root(&self) -> Result<std::path::PathBuf, String>;

    /// Resolve a workspace path (model-spelling to canonical).
    fn resolve_workspace_path(&self, path: &std::path::Path) -> Result<std::path::PathBuf, String>;

    /// Execute a single tool call.
    fn execute_tool(
        &self,
        turn_id: &str,
        is_interactive: bool,
        tc: &ParsedToolCall,
        allowed: &HashSet<String>,
        input: &TurnInput,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send;

    /// Check if a subagent was cancelled.
    fn is_subagent_cancelled(&self, call_id: &str) -> bool;

    /// Clear subagent cancellation flag.
    fn clear_subagent_cancelled(&self, call_id: &str);

    /// Register a file as "read" for read-before-edit guard.
    fn mark_file_read(&self, session_id: &str, path: &std::path::Path);

    /// Check if file was read.
    fn was_file_read(&self, session_id: &str, path: &std::path::Path) -> bool;

    /// Set session goal.
    fn set_goal(&self, session_id: &str, goal: &str);

    /// Get session goal.
    fn get_goal(&self, session_id: &str) -> String;

    /// Set session todos.
    fn set_todos(&self, session_id: &str, todos: &[SessionTodo]);

    /// Get session todos.
    fn get_todos(&self, session_id: &str) -> Vec<SessionTodo>;

    /// Clear all read files for a session.
    fn clear_read_files(&self, session_id: &str);

    /// Register approval request and return receiver.
    fn register_approval(&self, turn_id: &str, request_id: &str) -> tokio::sync::oneshot::Receiver<ApprovalDecision>;

    /// Cancel all approvals for a turn.
    fn cancel_approvals_for_turn(&self, turn_id: &str);

    /// Register question and return receiver.
    fn register_question(&self, turn_id: &str, request_id: &str) -> tokio::sync::oneshot::Receiver<QuestionAnswer>;

    /// Cancel all questions for a turn.
    fn cancel_questions_for_turn(&self, turn_id: &str);

    /// Perform a streaming completion request.
    fn completion_streaming(
        &self,
        request: serde_json::Value,
        on_delta: Box<dyn Fn(&str, &str) + Send>,
        should_cancel: Box<dyn Fn() -> bool + Send>,
        on_retry: Box<dyn Fn(&RetryNotice) + Send>,
        on_tool_start: Box<dyn Fn(&str) + Send>,
    ) -> impl std::future::Future<Output = Result<serde_json::Value, String>> + Send;

    /// Merge reasoning effort into a payload.
    fn merge_reasoning(&self, payload: &mut serde_json::Value, reasoning: Option<&serde_json::Value>);

    /// Apply temperature to payload.
    fn apply_temperature(&self, payload: &mut serde_json::Value, reasoning: Option<&serde_json::Value>, temperature: f64);

    /// Build request URL and API key for a protocol.
    fn build_request_parts(&self, base_url: &str, api_key: Option<&str>, protocol: &str, payload: serde_json::Value) -> serde_json::Value;

    /// Mark a turn as live (for injection).
    fn register_live_turn(&self, session_id: &str, turn_id: &str) -> LiveTurnGuard;

    /// Set turn_id on TurnInput if missing.
    fn ensure_turn_id(&self, input: &mut TurnInput) -> String;
}

/// Copy of SessionTodo for the trait interface.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTodo {
    pub id: String,
    pub content: String,
    pub status: String,
    pub priority: String,
    pub notes: Option<String>,
}

/// RAII guard for live turn tracking (re-export).
pub use crate::registry::LiveTurnGuard;
