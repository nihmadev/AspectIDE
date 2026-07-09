use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default protocol for provider-agnostic requests.
fn default_protocol() -> String {
    "openai-compatible".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatCompletionRequest {
    pub base_url: String,
    pub api_key: Option<String>,
    pub payload: Value,
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

impl AiChatCompletionRequest {
    pub const fn with_protocol(
        base_url: String,
        api_key: Option<String>,
        payload: Value,
        protocol: String,
    ) -> Self {
        Self {
            base_url,
            api_key,
            payload,
            protocol,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatCompletionResponse {
    pub status: u16,
    pub body: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatCompletionStreamRequest {
    pub base_url: String,
    pub api_key: Option<String>,
    pub payload: Value,
    stream_id: Option<String>,
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

impl AiChatCompletionStreamRequest {
    pub fn resolved_stream_id(&self) -> String {
        self.stream_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map_or_else(|| uuid::Uuid::new_v4().to_string(), ToString::to_string)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatCompletionStreamResponse {
    pub stream_id: String,
}

impl AiChatCompletionStreamResponse {
    pub const fn new(stream_id: String) -> Self {
        Self { stream_id }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatStreamEvent {
    pub stream_id: String,
    pub kind: String,
    pub data: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatHistoryDocument {
    pub schema_version: u32,
    pub active_session_id: String,
    pub sessions: Vec<Value>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatHistoryResponse {
    pub schema_version: u32,
    pub active_session_id: String,
    pub sessions: Vec<Value>,
    pub path: PathBuf,
    pub recovered: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatHistorySaveRequest {
    pub active_session_id: String,
    pub sessions: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderDiagnosticResponse {
    pub ok: bool,
    pub status: Option<u16>,
    pub latency_ms: u128,
    pub error: Option<String>,
    pub model: String,
    pub base_url: String,
}

/// Outcome of a reasoning-effort auto-fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasoningEffortFix {
    pub requested: String,
    pub applied: String,
}

/// One bounded automatic-retry notice, emitted right before a backoff sleep.
#[derive(Debug, Clone)]
pub struct RetryNotice {
    pub attempt: u32,
    pub max_attempts: u32,
    pub reason: String,
    pub detail: String,
    pub delay_ms: u64,
}

/// How a raced future completed.
pub enum CancelRace<T> {
    Ready(T),
    Cancelled,
    TimedOut,
}

/// Outcome of a streaming completion (internal use).
pub enum StreamCompletion {
    Done,
    Cancelled,
}
