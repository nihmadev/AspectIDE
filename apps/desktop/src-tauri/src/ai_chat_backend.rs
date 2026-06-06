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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatCompletionRequest {
    base_url: String,
    api_key: Option<String>,
    payload: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatCompletionResponse {
    status: u16,
    body: Value,
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
                let delay = retry_after_delay(response.headers()).unwrap_or_else(|| backoff_delay(attempt));
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
fn is_transient_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

/// Network-level reqwest errors that are safe to retry (connect/timeout/request).
fn is_transient_reqwest_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

/// Exponential backoff: 0.5s, 1s, 2s … capped at the retry ceiling.
fn backoff_delay(attempt: u32) -> Duration {
    let secs = (1u64 << attempt).saturating_mul(500).min(MAX_RETRY_DELAY_SECS * 1000);
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
                    let delay = retry_after_delay(response.headers()).unwrap_or_else(|| backoff_delay(attempt));
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
    loop {
        let chunk = tokio::select! {
            _ = &mut cancel_rx => return Ok(StreamCompletion::Cancelled),
            chunk = stream.next() => chunk,
        };

        let Some(chunk) = chunk else {
            break;
        };
        let bytes = chunk.map_err(|error| error.to_string())?;
        buffer.push_str(&String::from_utf8_lossy(&bytes));
        normalize_sse_buffer_newlines(&mut buffer);
        if emit_stream_sse_events(app, stream_id, &mut buffer)? {
            return Ok(StreamCompletion::Done);
        }
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

    let value = serde_json::from_str::<Value>(&data)
        .map_err(|error| format!("Invalid AI stream JSON chunk: {error}"))?;
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
            assert!(!is_transient_status(status), "{status} should not be transient");
        }
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff_delay(0), Duration::from_millis(500));
        assert_eq!(backoff_delay(1), Duration::from_millis(1000));
        assert_eq!(backoff_delay(2), Duration::from_millis(2000));
        // Large attempt is clamped to the ceiling, never overflows.
        assert!(backoff_delay(40) <= Duration::from_secs(MAX_RETRY_DELAY_SECS));
    }

    #[test]
    fn retry_after_header_parsed_and_capped() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "3".parse().unwrap());
        assert_eq!(retry_after_delay(&headers), Some(Duration::from_secs(3)));

        headers.insert(reqwest::header::RETRY_AFTER, "9999".parse().unwrap());
        assert_eq!(retry_after_delay(&headers), Some(Duration::from_secs(MAX_RETRY_DELAY_SECS)));

        // Non-numeric (HTTP-date form) is ignored → falls back to backoff.
        headers.insert(reqwest::header::RETRY_AFTER, "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap());
        assert_eq!(retry_after_delay(&headers), None);
    }
}
