use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::Utc;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tokio::{sync::oneshot, time::timeout};

const CHAT_TIMEOUT_SECS: u64 = 180;
const HISTORY_FILE: &str = "ai-chat-history.json";
const HISTORY_SCHEMA_VERSION: u32 = 1;
/// Automatic retries for transient provider failures (429 / 5xx / network).
/// Concept ported from claw-code (MIT) recovery recipes: one bounded automatic
/// recovery before surfacing the error. Streaming only retries the connection
/// phase (before any token is emitted) so partial output is never replayed.
const MAX_TRANSIENT_RETRIES: u32 = 2;
const MAX_RETRY_DELAY_SECS: u64 = 20;
/// Hard cap on the SSE reassembly buffer. A server that streams bytes without an
/// event delimiter (`\n\n`) could otherwise grow this without bound; cutting at
/// 8 MiB bounds memory against a misbehaving or malicious provider.
const MAX_SSE_BUFFER: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatCompletionRequest {
    base_url: String,
    api_key: Option<String>,
    payload: Value,
}

impl AiChatCompletionRequest {
    pub const fn new(base_url: String, api_key: Option<String>, payload: Value) -> Self {
        Self {
            base_url,
            api_key,
            payload,
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
    base_url: String,
    api_key: Option<String>,
    payload: Value,
    stream_id: Option<String>,
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
    stream_id: String,
}

impl AiChatCompletionStreamResponse {
    pub const fn new(stream_id: String) -> Self {
        Self { stream_id }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiChatStreamEvent {
    stream_id: String,
    kind: String,
    data: Option<Value>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiChatHistoryDocument {
    schema_version: u32,
    active_session_id: String,
    sessions: Vec<Value>,
    updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatHistoryResponse {
    schema_version: u32,
    active_session_id: String,
    sessions: Vec<Value>,
    path: PathBuf,
    recovered: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatHistorySaveRequest {
    active_session_id: String,
    sessions: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderDiagnosticResponse {
    ok: bool,
    status: Option<u16>,
    latency_ms: u128,
    error: Option<String>,
    model: String,
    base_url: String,
}

/// Merge a frontend-provided reasoning payload (e.g. `reasoning_effort` and
/// `reasoning.effort`) into an outgoing request payload. No-op when reasoning is
/// absent, null, or not an object — non-reasoning models send `{}`, so the request
/// shape is unchanged for models that don't support reasoning effort. Canonical
/// home for this so every native call site (turn loop, compaction, goal eval)
/// honors the selected effort identically.
pub fn merge_reasoning(payload: &mut Value, reasoning: Option<&Value>) {
    let (Some(Value::Object(extra)), Some(target)) = (reasoning, payload.as_object_mut()) else {
        return;
    };
    for (key, value) in extra {
        target.insert(key.clone(), value.clone());
    }
}

/// Build the `/models` listing endpoint from a provider base URL (OpenAI shape),
/// mirroring `completion_endpoint` so trailing-slash and `/chat/completions` bases
/// both resolve correctly.
fn models_endpoint(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("AI provider base URL is empty".to_string());
    }
    let url = reqwest::Url::parse(trimmed)
        .map_err(|error| format!("Invalid AI provider URL: {error}"))?;
    match url.scheme() {
        "http" | "https" => {}
        scheme => return Err(format!("Unsupported AI provider URL scheme: {scheme}")),
    }
    let text = url.as_str().trim_end_matches('/');
    // Accept a base that already points at chat/completions and rewrite it to /models.
    let root = text.strip_suffix("/chat/completions").unwrap_or(text);
    Ok(format!("{root}/models"))
}

/// Fetch a provider's available model ids from its OpenAI-compatible `/models`
/// endpoint. Returns the raw `id` strings exactly as the provider reports them —
/// the frontend decides naming, ordering (e.g. free-first), and context. Nothing
/// is hardcoded; this is the single source of truth for dynamic model discovery.
#[tauri::command]
pub async fn ai_list_provider_models(
    base_url: String,
    api_key: Option<String>,
) -> Result<Vec<String>, String> {
    let endpoint = models_endpoint(&base_url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?;
    let key = api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut builder = client
        .get(&endpoint)
        .header(reqwest::header::ACCEPT, "application/json");
    if let Some(key) = key {
        builder = builder.bearer_auth(key);
    }
    let response = builder
        .send()
        .await
        .map_err(|error| format!("Failed to reach {endpoint}: {error}"))?;
    let status = response.status();
    let body: Value = response
        .json()
        .await
        .map_err(|error| format!("Invalid models response: {error}"))?;
    if !status.is_success() {
        let detail = body
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("provider returned an error");
        return Err(format!("Models request failed ({status}): {detail}"));
    }
    // OpenAI shape: { data: [ { id, ... } ] }. Some providers return a bare array.
    let items = body
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| body.as_array())
        .ok_or_else(|| "Models response had no `data` array".to_string())?;
    let ids: Vec<String> = items
        .iter()
        .filter_map(|item| {
            item.get("id")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string)
        })
        .collect();
    Ok(ids)
}

pub async fn completion(
    request: AiChatCompletionRequest,
) -> Result<AiChatCompletionResponse, String> {
    let endpoint = completion_endpoint(&request.base_url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CHAT_TIMEOUT_SECS))
        .build()
        .map_err(|error| error.to_string())?;
    let api_key = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(ToString::to_string);

    let mut attempt: u32 = 0;
    loop {
        let mut builder = client
            .post(endpoint.as_str())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "application/json")
            .json(&request.payload);
        if let Some(key) = &api_key {
            builder = builder.bearer_auth(key);
        }

        let send_result = timeout(Duration::from_secs(CHAT_TIMEOUT_SECS + 5), builder.send()).await;
        let response = match send_result {
            Err(_) => {
                if attempt < MAX_TRANSIENT_RETRIES {
                    sleep_backoff(attempt).await;
                    attempt += 1;
                    continue;
                }
                return Err("AI request timed out".to_string());
            }
            Ok(Err(error)) => {
                if attempt < MAX_TRANSIENT_RETRIES && is_transient_reqwest_error(&error) {
                    sleep_backoff(attempt).await;
                    attempt += 1;
                    continue;
                }
                return Err(error.to_string());
            }
            Ok(Ok(response)) => response,
        };

        let status = response.status().as_u16();
        if status >= 400 {
            if attempt < MAX_TRANSIENT_RETRIES && is_transient_status(status) {
                let delay =
                    retry_after_delay(response.headers()).unwrap_or_else(|| backoff_delay(attempt));
                tokio::time::sleep(delay).await;
                attempt += 1;
                continue;
            }
            let body = response.json::<Value>().await.unwrap_or(Value::Null);
            return Err(response_error(status, &body));
        }

        let body = response
            .json::<Value>()
            .await
            .map_err(|error| error.to_string())?;
        return Ok(AiChatCompletionResponse { status, body });
    }
}

/// Streaming completion for the native turn-loop. Sends `stream: true`, invokes
/// `on_delta(content_chunk, reasoning_chunk)` for every SSE token as it arrives
/// (so the UI renders text in real time), accumulates content + reasoning + the
/// incrementally-delivered tool calls, and returns a response whose
/// `choices[0].message` matches the non-streaming shape — so the caller parses
/// it identically. Connection-phase failures retry; once tokens flow the request
/// is never replayed.
///
/// `should_cancel` is polled once per received SSE chunk: when it returns true the
/// loop stops and the response (and its underlying HTTP connection) is dropped, so
/// a Stop pressed mid-stream truly aborts the in-flight request instead of draining
/// the model's full generation. The partial body is returned so the caller's own
/// post-stream cancellation check can finalize the turn as cancelled.
pub async fn completion_streaming<F, C>(
    request: AiChatCompletionRequest,
    mut on_delta: F,
    should_cancel: C,
) -> Result<AiChatCompletionResponse, String>
where
    F: FnMut(&str, &str),
    C: Fn() -> bool,
{
    let endpoint = completion_endpoint(&request.base_url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CHAT_TIMEOUT_SECS))
        .build()
        .map_err(|error| error.to_string())?;
    let payload = stream_payload(request.payload);
    let api_key = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(ToString::to_string);

    // Connection phase (retryable until the first byte streams).
    let response = {
        let mut attempt: u32 = 0;
        loop {
            let mut builder = client
                .post(endpoint.as_str())
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .json(&payload);
            if let Some(key) = &api_key {
                builder = builder.bearer_auth(key);
            }
            let send = timeout(Duration::from_secs(CHAT_TIMEOUT_SECS + 5), builder.send()).await;
            let response = match send {
                Err(_) => {
                    if attempt < MAX_TRANSIENT_RETRIES {
                        sleep_backoff(attempt).await;
                        attempt += 1;
                        continue;
                    }
                    return Err("AI stream request timed out".to_string());
                }
                Ok(Err(error)) => {
                    if attempt < MAX_TRANSIENT_RETRIES && is_transient_reqwest_error(&error) {
                        sleep_backoff(attempt).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(error.to_string());
                }
                Ok(Ok(response)) => response,
            };
            let status = response.status().as_u16();
            if status >= 400 {
                if attempt < MAX_TRANSIENT_RETRIES && is_transient_status(status) {
                    let delay = retry_after_delay(response.headers())
                        .unwrap_or_else(|| backoff_delay(attempt));
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                    continue;
                }
                let text = response.text().await.unwrap_or_default();
                return Err(stream_response_error(status, &text));
            }
            break response;
        }
    };

    let mut accumulator = StreamAccumulator::default();
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    // Carry an incomplete trailing UTF-8 sequence across chunks: bytes_stream()
    // splits on arbitrary byte boundaries, so decoding each chunk independently
    // would mangle multibyte code points (Cyrillic/CJK/emoji) into U+FFFD.
    let mut byte_tail: Vec<u8> = Vec::new();
    'outer: while let Some(chunk) = stream.next().await {
        // A Stop pressed mid-stream: bail before processing this chunk so the
        // response (and its in-flight HTTP connection) is dropped instead of
        // draining the model's full generation. The accumulated-so-far body is
        // returned; the caller's post-stream cancellation check finalizes it.
        if should_cancel() {
            break 'outer;
        }
        let bytes = chunk.map_err(|error| error.to_string())?;
        byte_tail.extend_from_slice(&bytes);
        // Drain byte_tail fully each chunk. Branch on error_len(): a genuinely
        // invalid byte (e.g. a stray 0xFF from a misbehaving provider) is replaced
        // with U+FFFD like the old from_utf8_lossy and skipped, instead of pinning
        // valid_up_to at 0 forever (which would stall emission and grow byte_tail
        // without bound past MAX_SSE_BUFFER). Looping means valid content *after*
        // an invalid byte is appended to `buffer` now rather than deferred a chunk
        // or dropped at end-of-stream, so byte_tail only ever retains an incomplete
        // trailing code point (<= 3 bytes) carried to the next chunk.
        loop {
            let (valid_up_to, invalid_len) = match std::str::from_utf8(&byte_tail) {
                Ok(text) => {
                    buffer.push_str(text);
                    byte_tail.clear();
                    break;
                }
                Err(error) => (error.valid_up_to(), error.error_len()),
            };
            if valid_up_to > 0 {
                buffer.push_str(std::str::from_utf8(&byte_tail[..valid_up_to]).unwrap());
            }
            match invalid_len {
                Some(invalid_len) => {
                    buffer.push('\u{FFFD}');
                    byte_tail.drain(..valid_up_to + invalid_len);
                }
                None => {
                    byte_tail.drain(..valid_up_to);
                    break;
                }
            }
        }
        normalize_sse_buffer_newlines(&mut buffer);
        if buffer.len() > MAX_SSE_BUFFER {
            return Err("AI stream buffer exceeded limit".to_string());
        }
        while let Some(index) = buffer.find("\n\n") {
            let event = buffer[..index].to_string();
            buffer.drain(..index + 2);
            let Some(data) = sse_event_data(&event) else {
                continue;
            };
            if data.trim() == "[DONE]" {
                break 'outer;
            }
            if let Ok(value) = serde_json::from_str::<Value>(&data) {
                accumulator.ingest(&value, &mut on_delta);
            }
        }
    }

    // Flush any trailing bytes (a truncated final code point becomes U+FFFD) so a
    // final event that ends without a `\n\n` delimiter is still processed below,
    // instead of being silently dropped when the accumulator is returned.
    if !byte_tail.is_empty() {
        buffer.push_str(&String::from_utf8_lossy(&byte_tail));
    }
    normalize_sse_buffer_newlines(&mut buffer);
    while let Some(index) = buffer.find("\n\n") {
        let event = buffer[..index].to_string();
        buffer.drain(..index + 2);
        let Some(data) = sse_event_data(&event) else {
            continue;
        };
        if data.trim() == "[DONE]" {
            break;
        }
        if let Ok(value) = serde_json::from_str::<Value>(&data) {
            accumulator.ingest(&value, &mut on_delta);
        }
    }

    Ok(AiChatCompletionResponse {
        status: 200,
        body: accumulator.into_response_body(),
    })
}

/// Assembles streamed SSE delta chunks into a single OpenAI-style response body.
/// Tool calls arrive incrementally (by `index`, with `id`/`name` once and
/// `arguments` in fragments), so they are merged per index.
#[derive(Default)]
struct StreamAccumulator {
    content: String,
    reasoning: String,
    tool_calls: Vec<StreamToolCall>,
    usage: Option<Value>,
    finish_reason: Option<String>,
}

#[derive(Default)]
struct StreamToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl StreamAccumulator {
    fn ingest<F: FnMut(&str, &str)>(&mut self, value: &Value, on_delta: &mut F) {
        if let Some(usage) = value.get("usage") {
            if !usage.is_null() {
                self.usage = Some(usage.clone());
            }
        }
        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
        else {
            return;
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_string());
        }
        let delta = choice.get("delta").unwrap_or(choice);
        let content = delta.get("content").and_then(Value::as_str).unwrap_or("");
        let reasoning = delta
            .get("reasoning_content")
            .or_else(|| delta.get("reasoning"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if !content.is_empty() {
            self.content.push_str(content);
        }
        if !reasoning.is_empty() {
            self.reasoning.push_str(reasoning);
        }
        if !content.is_empty() || !reasoning.is_empty() {
            on_delta(content, reasoning);
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                self.merge_tool_call(call);
            }
        }
    }

    fn merge_tool_call(&mut self, call: &Value) {
        let index = match call.get("index").and_then(Value::as_u64) {
            Some(value) => usize::try_from(value).unwrap_or(0),
            // No explicit index: extend the most recent call (or start the first).
            None => self.tool_calls.len().saturating_sub(1),
        };
        while self.tool_calls.len() <= index {
            self.tool_calls.push(StreamToolCall::default());
        }
        let slot = &mut self.tool_calls[index];
        if let Some(id) = call.get("id").and_then(Value::as_str) {
            if !id.is_empty() {
                slot.id = id.to_string();
            }
        }
        if let Some(function) = call.get("function") {
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                if !name.is_empty() {
                    slot.name = name.to_string();
                }
            }
            if let Some(args) = function.get("arguments").and_then(Value::as_str) {
                slot.arguments.push_str(args);
            }
        }
    }

    fn into_response_body(self) -> Value {
        let mut message = serde_json::json!({
            "role": "assistant",
            "content": self.content,
        });
        if !self.reasoning.is_empty() {
            message["reasoning_content"] = Value::String(self.reasoning);
        }
        let tool_calls: Vec<Value> = self
            .tool_calls
            .into_iter()
            .filter(|tc| !tc.id.is_empty() || !tc.name.is_empty())
            .map(|tc| {
                serde_json::json!({
                    "id": tc.id,
                    "type": "function",
                    "function": { "name": tc.name, "arguments": tc.arguments },
                })
            })
            .collect();
        if !tool_calls.is_empty() {
            message["tool_calls"] = Value::Array(tool_calls);
        }
        let mut body = serde_json::json!({
            "choices": [{
                "index": 0,
                "message": message,
                "finish_reason": self.finish_reason.unwrap_or_else(|| "stop".to_string()),
            }],
        });
        if let Some(usage) = self.usage {
            body["usage"] = usage;
        }
        body
    }
}

pub fn history_load(app: &AppHandle) -> Result<AiChatHistoryResponse, String> {
    let path = history_path(app)?;
    recover_history_temp_file(&path)?;
    if !path.exists() {
        return Ok(AiChatHistoryResponse {
            schema_version: HISTORY_SCHEMA_VERSION,
            active_session_id: String::new(),
            sessions: Vec::new(),
            path,
            recovered: false,
        });
    }

    let raw = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
    match serde_json::from_str::<AiChatHistoryDocument>(&raw) {
        Ok(document) => Ok(AiChatHistoryResponse {
            schema_version: document.schema_version,
            active_session_id: document.active_session_id,
            sessions: document.sessions,
            path,
            recovered: false,
        }),
        Err(error) => {
            let backup_path = path.with_extension(format!(
                "json.recovered-{}",
                Utc::now().format("%Y%m%d%H%M%S")
            ));
            let _ = std::fs::rename(&path, &backup_path);
            Err(format!(
                "AI chat history was corrupted and moved to {}: {error}",
                backup_path.display()
            ))
        }
    }
}

pub fn history_save(
    app: &AppHandle,
    request: AiChatHistorySaveRequest,
) -> Result<AiChatHistoryResponse, String> {
    let path = history_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let document = AiChatHistoryDocument {
        schema_version: HISTORY_SCHEMA_VERSION,
        active_session_id: request.active_session_id,
        sessions: request.sessions,
        updated_at: Utc::now(),
    };
    let temporary_path = history_temp_path(&path);
    std::fs::write(
        &temporary_path,
        serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|error| error.to_string())?;
    }
    std::fs::rename(&temporary_path, &path).map_err(|error| error.to_string())?;

    Ok(AiChatHistoryResponse {
        schema_version: document.schema_version,
        active_session_id: document.active_session_id,
        sessions: document.sessions,
        path,
        recovered: false,
    })
}

pub async fn provider_diagnostic(
    request: AiChatCompletionRequest,
) -> Result<AiProviderDiagnosticResponse, String> {
    let endpoint = completion_endpoint(&request.base_url)?;
    let model = request
        .payload
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let started = std::time::Instant::now();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?;
    let mut builder = client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&request.payload);

    if let Some(api_key) = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        builder = builder.bearer_auth(api_key);
    }

    match timeout(Duration::from_secs(25), builder.send()).await {
        Ok(Ok(response)) => {
            let status = response.status().as_u16();
            let error = if status >= 400 {
                let text = response.text().await.unwrap_or_default();
                Some(stream_response_error(status, &text))
            } else {
                None
            };
            Ok(AiProviderDiagnosticResponse {
                ok: status < 400,
                status: Some(status),
                latency_ms: started.elapsed().as_millis(),
                error,
                model,
                base_url: request.base_url,
            })
        }
        Ok(Err(error)) => Ok(AiProviderDiagnosticResponse {
            ok: false,
            status: None,
            latency_ms: started.elapsed().as_millis(),
            error: Some(error.to_string()),
            model,
            base_url: request.base_url,
        }),
        Err(_) => Ok(AiProviderDiagnosticResponse {
            ok: false,
            status: None,
            latency_ms: started.elapsed().as_millis(),
            error: Some("AI provider diagnostic timed out".to_string()),
            model,
            base_url: request.base_url,
        }),
    }
}

pub async fn run_completion_stream(
    app: AppHandle,
    stream_id: String,
    request: AiChatCompletionStreamRequest,
    cancel_rx: oneshot::Receiver<()>,
) {
    let result = stream_completion(&app, &stream_id, request, cancel_rx).await;

    match result {
        Ok(StreamCompletion::Done) => {}
        Ok(StreamCompletion::Cancelled) => {
            let _ = emit_stream_event(
                &app,
                AiChatStreamEvent {
                    stream_id,
                    kind: "cancelled".to_string(),
                    data: None,
                    error: None,
                },
            );
        }
        Err(error) => {
            let _ = emit_stream_event(
                &app,
                AiChatStreamEvent {
                    stream_id,
                    kind: "error".to_string(),
                    data: None,
                    error: Some(error),
                },
            );
        }
    }
}

fn history_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join(HISTORY_FILE))
}

fn history_temp_path(path: &Path) -> PathBuf {
    path.with_extension("json.tmp")
}

fn recover_history_temp_file(path: &Path) -> Result<(), String> {
    let temporary_path = history_temp_path(path);
    if path.exists() || !temporary_path.exists() {
        return Ok(());
    }
    std::fs::rename(&temporary_path, path).map_err(|error| error.to_string())
}

fn completion_endpoint(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("AI provider base URL is empty".to_string());
    }
    let url = reqwest::Url::parse(trimmed)
        .map_err(|error| format!("Invalid AI provider URL: {error}"))?;
    match url.scheme() {
        "http" | "https" => {}
        scheme => return Err(format!("Unsupported AI provider URL scheme: {scheme}")),
    }
    let text = url.as_str().trim_end_matches('/');
    if text.ends_with("/chat/completions") {
        Ok(text.to_string())
    } else {
        Ok(format!("{text}/chat/completions"))
    }
}

/// Transient HTTP statuses worth one bounded automatic retry.
const fn is_transient_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

/// Network-level reqwest errors that are safe to retry (connect/timeout/request).
fn is_transient_reqwest_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

/// Exponential backoff: 0.5s, 1s, 2s … capped at the retry ceiling.
fn backoff_delay(attempt: u32) -> Duration {
    let secs = (1u64 << attempt)
        .saturating_mul(500)
        .min(MAX_RETRY_DELAY_SECS * 1000);
    Duration::from_millis(secs)
}

async fn sleep_backoff(attempt: u32) {
    tokio::time::sleep(backoff_delay(attempt)).await;
}

/// Honor a numeric `Retry-After` header (seconds), capped to a sane maximum.
fn retry_after_delay(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let seconds = headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(Duration::from_secs(seconds.min(MAX_RETRY_DELAY_SECS)))
}

fn response_error(status: u16, body: &Value) -> String {
    let message = body
        .get("error")
        .and_then(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
        })
        .or_else(|| body.get("message").and_then(Value::as_str))
        .unwrap_or("AI provider returned an error");
    format!("AI provider error {status}: {message}")
}

enum StreamCompletion {
    Done,
    Cancelled,
}

async fn stream_completion(
    app: &AppHandle,
    stream_id: &str,
    request: AiChatCompletionStreamRequest,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<StreamCompletion, String> {
    let endpoint = completion_endpoint(&request.base_url)?;
    // No total request timeout here: a long agent turn may actively stream for
    // longer than CHAT_TIMEOUT_SECS and must not be aborted mid-generation. The
    // connection phase is bounded by connect_timeout (and the wrapping send
    // timeout); genuine stalls are caught by the per-chunk idle timeout below.
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CHAT_TIMEOUT_SECS))
        .build()
        .map_err(|error| error.to_string())?;
    let payload = stream_payload(request.payload);
    let api_key = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(ToString::to_string);

    // Retry only the connection phase: once the first byte streams we can never
    // safely replay the request, so the loop only re-runs before `bytes_stream()`.
    let response = {
        let mut attempt: u32 = 0;
        loop {
            let mut builder = client
                .post(endpoint.as_str())
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .json(&payload);
            if let Some(key) = &api_key {
                builder = builder.bearer_auth(key);
            }

            let send = tokio::select! {
                _ = &mut cancel_rx => return Ok(StreamCompletion::Cancelled),
                send = timeout(Duration::from_secs(CHAT_TIMEOUT_SECS + 5), builder.send()) => send,
            };

            let response = match send {
                Err(_) => {
                    if attempt < MAX_TRANSIENT_RETRIES {
                        sleep_backoff(attempt).await;
                        attempt += 1;
                        continue;
                    }
                    return Err("AI stream request timed out".to_string());
                }
                Ok(Err(error)) => {
                    if attempt < MAX_TRANSIENT_RETRIES && is_transient_reqwest_error(&error) {
                        sleep_backoff(attempt).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(error.to_string());
                }
                Ok(Ok(response)) => response,
            };

            let status = response.status().as_u16();
            if status >= 400 {
                if attempt < MAX_TRANSIENT_RETRIES && is_transient_status(status) {
                    let delay = retry_after_delay(response.headers())
                        .unwrap_or_else(|| backoff_delay(attempt));
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                    continue;
                }
                let text = response.text().await.unwrap_or_default();
                return Err(stream_response_error(status, &text));
            }
            break response;
        }
    };

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    // Carry an incomplete trailing UTF-8 sequence across chunks: bytes_stream()
    // splits on arbitrary byte boundaries, so decoding each chunk independently
    // would mangle multibyte code points (Cyrillic/CJK/emoji) into U+FFFD.
    let mut byte_tail: Vec<u8> = Vec::new();
    loop {
        // Per-chunk idle timeout: cuts only stalls (no bytes for CHAT_TIMEOUT_SECS),
        // never long-but-actively-streaming generations.
        let chunk = tokio::select! {
            _ = &mut cancel_rx => return Ok(StreamCompletion::Cancelled),
            chunk = timeout(Duration::from_secs(CHAT_TIMEOUT_SECS), stream.next()) => match chunk {
                Ok(chunk) => chunk,
                Err(_) => return Err("AI stream stalled".to_string()),
            },
        };

        let Some(chunk) = chunk else {
            break;
        };
        let bytes = chunk.map_err(|error| error.to_string())?;
        byte_tail.extend_from_slice(&bytes);
        // Drain byte_tail fully each chunk. Branch on error_len(): a genuinely
        // invalid byte (e.g. a stray 0xFF from a misbehaving provider) is replaced
        // with U+FFFD like the old from_utf8_lossy and skipped, instead of pinning
        // valid_up_to at 0 forever (which would stall emission and grow byte_tail
        // without bound past MAX_SSE_BUFFER). Looping means valid content *after*
        // an invalid byte is appended to `buffer` now rather than deferred a chunk
        // or dropped at end-of-stream, so byte_tail only ever retains an incomplete
        // trailing code point (<= 3 bytes) carried to the next chunk.
        loop {
            let (valid_up_to, invalid_len) = match std::str::from_utf8(&byte_tail) {
                Ok(text) => {
                    buffer.push_str(text);
                    byte_tail.clear();
                    break;
                }
                Err(error) => (error.valid_up_to(), error.error_len()),
            };
            if valid_up_to > 0 {
                buffer.push_str(std::str::from_utf8(&byte_tail[..valid_up_to]).unwrap());
            }
            match invalid_len {
                Some(invalid_len) => {
                    buffer.push('\u{FFFD}');
                    byte_tail.drain(..valid_up_to + invalid_len);
                }
                None => {
                    byte_tail.drain(..valid_up_to);
                    break;
                }
            }
        }
        normalize_sse_buffer_newlines(&mut buffer);
        if buffer.len() > MAX_SSE_BUFFER {
            return Err("AI stream buffer exceeded limit".to_string());
        }
        if emit_stream_sse_events(app, stream_id, &mut buffer)? {
            return Ok(StreamCompletion::Done);
        }
    }

    // Flush any trailing bytes (a truncated final code point becomes U+FFFD) so a
    // final event that ends without a `\n\n` delimiter is still processed below.
    if !byte_tail.is_empty() {
        buffer.push_str(&String::from_utf8_lossy(&byte_tail));
    }
    normalize_sse_buffer_newlines(&mut buffer);
    if emit_stream_sse_events(app, stream_id, &mut buffer)? {
        return Ok(StreamCompletion::Done);
    }
    if !buffer.trim().is_empty() && emit_stream_sse_event(app, stream_id, buffer.trim())? {
        return Ok(StreamCompletion::Done);
    }

    emit_stream_event(
        app,
        AiChatStreamEvent {
            stream_id: stream_id.to_string(),
            kind: "done".to_string(),
            data: None,
            error: None,
        },
    )?;
    Ok(StreamCompletion::Done)
}

fn stream_payload(mut payload: Value) -> Value {
    if let Value::Object(object) = &mut payload {
        object.insert("stream".to_string(), Value::Bool(true));
    }
    payload
}

fn stream_response_error(status: u16, text: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return response_error(status, &value);
    }
    let message = text.trim();
    if message.is_empty() {
        format!("AI provider stream error {status}")
    } else {
        format!("AI provider stream error {status}: {message}")
    }
}

fn normalize_sse_buffer_newlines(buffer: &mut String) {
    if buffer.contains('\r') {
        *buffer = buffer.replace("\r\n", "\n").replace('\r', "\n");
    }
}

fn emit_stream_sse_events(
    app: &AppHandle,
    stream_id: &str,
    buffer: &mut String,
) -> Result<bool, String> {
    while let Some(index) = buffer.find("\n\n") {
        let event = buffer[..index].to_string();
        buffer.drain(..index + 2);
        if emit_stream_sse_event(app, stream_id, &event)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn emit_stream_sse_event(app: &AppHandle, stream_id: &str, event: &str) -> Result<bool, String> {
    let Some(data) = sse_event_data(event) else {
        return Ok(false);
    };
    if data.trim() == "[DONE]" {
        emit_stream_event(
            app,
            AiChatStreamEvent {
                stream_id: stream_id.to_string(),
                kind: "done".to_string(),
                data: None,
                error: None,
            },
        )?;
        return Ok(true);
    }

    // Empty keep-alive (`data:` with no payload) and malformed non-JSON data lines
    // must be skipped, not propagated: a single bad line would otherwise `?`-abort
    // the whole turn and discard every token already streamed.
    if data.trim().is_empty() {
        return Ok(false);
    }
    let Ok(value) = serde_json::from_str::<Value>(&data) else {
        return Ok(false);
    };
    emit_stream_event(
        app,
        AiChatStreamEvent {
            stream_id: stream_id.to_string(),
            kind: "chunk".to_string(),
            data: Some(value),
            error: None,
        },
    )?;
    Ok(false)
}

fn sse_event_data(event: &str) -> Option<String> {
    let lines = event
        .lines()
        .filter_map(|line| {
            let line = line.trim_end();
            if line.starts_with(':') {
                return None;
            }
            let data = line.strip_prefix("data:")?;
            Some(data.strip_prefix(' ').unwrap_or(data).to_string())
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn emit_stream_event(app: &AppHandle, event: AiChatStreamEvent) -> Result<(), String> {
    app.emit("lux://ai-chat-stream", event)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_event_data_collects_multiline_data_and_ignores_comments() {
        let event = ": keep-alive\nevent: message\ndata: {\"a\":\ndata: 1}\n";

        assert_eq!(sse_event_data(event).as_deref(), Some("{\"a\":\n1}"));
    }

    #[test]
    fn stream_payload_forces_stream_true() {
        let payload = serde_json::json!({ "model": "gpt-5.5", "stream": false });

        let payload = stream_payload(payload);

        assert_eq!(payload.get("stream").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn transient_status_classification() {
        for status in [408, 425, 429, 500, 502, 503, 504] {
            assert!(is_transient_status(status), "{status} should be transient");
        }
        for status in [400, 401, 403, 404, 422] {
            assert!(
                !is_transient_status(status),
                "{status} should not be transient"
            );
        }
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff_delay(0), Duration::from_millis(500));
        assert_eq!(backoff_delay(1), Duration::from_secs(1));
        assert_eq!(backoff_delay(2), Duration::from_secs(2));
        // Large attempt is clamped to the ceiling, never overflows.
        assert!(backoff_delay(40) <= Duration::from_secs(MAX_RETRY_DELAY_SECS));
    }

    #[test]
    fn retry_after_header_parsed_and_capped() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "3".parse().unwrap());
        assert_eq!(retry_after_delay(&headers), Some(Duration::from_secs(3)));

        headers.insert(reqwest::header::RETRY_AFTER, "9999".parse().unwrap());
        assert_eq!(
            retry_after_delay(&headers),
            Some(Duration::from_secs(MAX_RETRY_DELAY_SECS))
        );

        // Non-numeric (HTTP-date form) is ignored → falls back to backoff.
        headers.insert(
            reqwest::header::RETRY_AFTER,
            "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(retry_after_delay(&headers), None);
    }

    #[test]
    fn stream_accumulator_concatenates_content_and_emits_deltas() {
        let mut acc = StreamAccumulator::default();
        let mut seen = String::new();
        let mut push = |c: &str, _r: &str| seen.push_str(c);
        for token in ["Hel", "lo ", "world"] {
            let chunk = serde_json::json!({
                "choices": [{ "delta": { "content": token } }]
            });
            acc.ingest(&chunk, &mut push);
        }
        assert_eq!(seen, "Hello world");
        let body = acc.into_response_body();
        assert_eq!(body["choices"][0]["message"]["content"], "Hello world");
    }

    #[test]
    fn stream_accumulator_merges_fragmented_tool_calls() {
        let mut acc = StreamAccumulator::default();
        let mut noop = |_: &str, _: &str| {};
        // id+name arrive first, then arguments stream in fragments (OpenAI shape).
        let frames = [
            serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"Read","arguments":""}}]}}]}),
            serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]}}]}),
            serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"a.rs\"}"}}]}}]}),
        ];
        for frame in frames {
            acc.ingest(&frame, &mut noop);
        }
        let body = acc.into_response_body();
        let call = &body["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(call["id"], "call_1");
        assert_eq!(call["function"]["name"], "Read");
        assert_eq!(call["function"]["arguments"], "{\"path\":\"a.rs\"}");
    }

    #[test]
    fn stream_accumulator_captures_usage() {
        let mut acc = StreamAccumulator::default();
        let mut noop = |_: &str, _: &str| {};
        acc.ingest(
            &serde_json::json!({
                "choices": [{ "delta": { "content": "hi" } }],
                "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
            }),
            &mut noop,
        );
        let body = acc.into_response_body();
        assert_eq!(body["usage"]["total_tokens"], 15);
    }
}
