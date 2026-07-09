use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum TurnEvent {
    #[serde(rename_all = "camelCase")]
    AssistantCreated { turn_id: String, message_id: String },
    #[serde(rename_all = "camelCase")]
    StreamDelta {
        turn_id: String,
        content: String,
        reasoning: String,
    },
    #[serde(rename_all = "camelCase")]
    StatusChange { turn_id: String, phase: String },
    #[serde(rename_all = "camelCase")]
    UserMessageInjected { turn_id: String, text: String },
    #[serde(rename_all = "camelCase")]
    ToolCallStarted {
        turn_id: String,
        call_id: String,
        tool: String,
        input: String,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallCompleted {
        turn_id: String,
        call_id: String,
        status: String,
        output: String,
        error: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    SubagentProgress {
        turn_id: String,
        call_id: String,
        agent_id: String,
        stage: String,
        content: String,
        tool: String,
    },
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
    #[serde(rename_all = "camelCase")]
    QuestionRequired {
        turn_id: String,
        request_id: String,
        question: String,
        detail: String,
        options: Vec<QuestionOption>,
        multi_select: bool,
        allow_custom: bool,
        html_preview: String,
    },
    #[serde(rename_all = "camelCase")]
    PlanProposed {
        turn_id: String,
        plan_id: String,
        title: String,
        summary: String,
        steps: Vec<PlanStep>,
        alternatives: Vec<PlanDecision>,
        risks: Vec<String>,
        verification: Vec<String>,
        quality: f64,
        coaching: Vec<String>,
        auto_start: bool,
    },
    #[serde(rename_all = "camelCase")]
    TurnUsage {
        turn_id: String,
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
        cached_prompt_tokens: u64,
        model_calls: u64,
    },
    #[serde(rename_all = "camelCase")]
    TurnDone {
        turn_id: String,
        message_id: String,
        content: String,
        duration_ms: u64,
    },
    #[serde(rename_all = "camelCase")]
    TurnError { turn_id: String, error: String },
    #[serde(rename_all = "camelCase")]
    TurnRetry {
        turn_id: String,
        attempt: u32,
        max_attempts: u32,
        reason: String,
        detail: String,
        delay_ms: u64,
    },
    #[serde(rename_all = "camelCase")]
    ReasoningEffortFallback {
        turn_id: String,
        requested: String,
        applied: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub title: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDecision {
    pub option: String,
    #[serde(default)]
    pub tradeoff: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalDecision {
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswer {
    pub answer: String,
    #[serde(default)]
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemPromptInput {
    pub agent_mode: String,
    pub agent_name: String,
    pub agent_instructions: String,
    pub global_instructions: String,
    pub project_instructions: String,
    pub project_agents_snip: String,
    pub tool_approval_mode: String,
    pub tool_round_limit: Option<u32>,
    pub selected_effort_id: String,
    pub selected_model_alias: String,
    pub provider_name: String,
    pub provider_protocol: String,
    pub workspace_root: String,
    pub runtime_tools_available: bool,
    pub agent_browser_enabled: bool,
    #[serde(default)]
    pub token_economy: bool,
    #[serde(default)]
    pub custom_prompt_enabled: bool,
    #[serde(default)]
    pub custom_prompt: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInput {
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    pub session_id: String,
    pub message: String,
    #[serde(default)]
    pub user_content: Option<serde_json::Value>,
    pub history: Vec<serde_json::Value>,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    #[serde(default)]
    pub embedding_model: Option<String>,
    pub agent_mode: String,
    pub tool_round_limit: Option<u32>,
    pub tool_approval_mode: String,
    #[serde(default)]
    pub tool_permission_rules: Vec<String>,
    #[serde(default)]
    pub reasoning: Option<serde_json::Value>,
    #[serde(default)]
    pub anthropic_cache: bool,
    pub prompt_input: SystemPromptInput,
    #[serde(default)]
    pub agent_browser_enabled: bool,
    #[serde(default)]
    pub active_document_path: Option<String>,
    #[serde(default)]
    pub terminal_context: Option<serde_json::Value>,
    #[serde(default)]
    pub file_checkpoint_id: Option<String>,
    #[serde(default)]
    pub available_model_ids: Vec<String>,
}

#[derive(Debug)]
pub struct ParsedAssistant {
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<ParsedToolCall>,
}

#[derive(Debug, Clone)]
pub struct ParsedToolCall {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

pub enum ApprovalGate {
    Blocked,
    Allowed,
    RejectedNonInteractive,
    Prompt,
}

#[derive(Debug, serde::Serialize)]
pub struct SecretFinding {
    pub pattern: String,
    pub line: usize,
    pub column: usize,
    pub snippet: String,
    pub redacted: String,
}

#[derive(Debug, Clone)]
pub struct RetryNotice {
    pub attempt: u32,
    pub max_attempts: u32,
    pub reason: String,
    pub detail: String,
    pub delay_ms: u64,
}
