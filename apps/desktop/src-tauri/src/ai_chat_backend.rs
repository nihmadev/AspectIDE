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
/// Hard ceiling on automatic transient retries (so up to 10 total attempts for the
/// most recoverable failures). The per-failure budget below decides how many of
/// these a given error actually gets — a rate limit earns all of them, a dead
/// socket only a few. Streaming only retries the connection phase (before any token
/// is emitted), so partial output is never replayed.
const MAX_TRANSIENT_RETRIES: u32 = 9;
const MAX_RETRY_DELAY_SECS: u64 = 30;

/// Retries worth attempting for a transient HTTP status. Rate limits and server/
/// overload errors recover by waiting, so they get the full budget; an edge 403 or
/// request-timeout status clears less often, so it gets only a few before surfacing.
const fn retry_budget_for_status(status: u16) -> u32 {
    match status {
        429 | 500 | 502 | 503 | 504 => MAX_TRANSIENT_RETRIES,
        _ => 4,
    }
}

/// Connect/timeout/request errors: ride the same ~10-attempt linear ladder so a
/// transient network blip gets the full gentle backoff before surfacing.
const NETWORK_RETRY_BUDGET: u32 = 9;
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
    /// Provider wire protocol (`openai-compatible` default, or `anthropic` for the
    /// Anthropic Messages API). Selects the endpoint, auth headers, and request/
    /// response translation. Defaults so older frontend payloads stay valid.
    #[serde(default = "default_protocol")]
    protocol: String,
}

fn default_protocol() -> String {
    "openai-compatible".to_string()
}

impl AiChatCompletionRequest {
    /// Build a native completion request, pinning the provider protocol (e.g.
    /// `"anthropic"`). An empty or unrecognized protocol behaves as
    /// OpenAI-compatible.
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
    base_url: String,
    api_key: Option<String>,
    payload: Value,
    stream_id: Option<String>,
    /// Provider wire protocol (`openai-compatible` default, or `anthropic`).
    /// Selects the endpoint, auth headers, and SSE accumulator. Defaults so
    /// older frontend payloads (which omit this field) stay valid.
    #[serde(default = "default_protocol")]
    protocol: String,
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

/// True when the frontend sent a non-empty reasoning blob — i.e. the active model
/// is a reasoning model (non-reasoning models send `{}`). Reasoning models reject
/// explicit sampling params and the legacy `max_tokens` name on several providers
/// (`OpenAI` o-series / gpt-5 return HTTP 400), so callers gate those fields on this.
pub fn reasoning_present(reasoning: Option<&Value>) -> bool {
    matches!(reasoning, Some(Value::Object(map)) if !map.is_empty())
}

/// Insert the sampling `temperature` only for standard models. Reasoning models
/// (non-empty reasoning blob) omit it and fall back to the provider default,
/// avoiding the HTTP 400 reasoning models raise on an explicit non-default
/// temperature. Pair with `merge_reasoning` at every native call site.
pub fn apply_temperature(payload: &mut Value, reasoning: Option<&Value>, temperature: f64) {
    if reasoning_present(reasoning) {
        return;
    }
    if let Some(target) = payload.as_object_mut() {
        target.insert("temperature".to_string(), serde_json::json!(temperature));
    }
}

/// Build the `/models` listing endpoint from a provider base URL (`OpenAI` shape),
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

pub async fn completion<R>(
    request: AiChatCompletionRequest,
    mut on_retry: R,
) -> Result<AiChatCompletionResponse, String>
where
    R: FnMut(RetryNotice),
{
    let anthropic = crate::ai_anthropic::is_anthropic(&request.protocol);
    let endpoint = if anthropic {
        crate::ai_anthropic::messages_endpoint(&request.base_url)?
    } else {
        completion_endpoint(&request.base_url)?
    };
    let payload = if anthropic {
        crate::ai_anthropic::to_anthropic_request(&request.payload)
    } else {
        request.payload.clone()
    };
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
        let builder = client
            .post(endpoint.as_str())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "application/json")
            .json(&payload);
        let builder = apply_auth(builder, anthropic, api_key.as_deref());

        let send_result = timeout(Duration::from_secs(CHAT_TIMEOUT_SECS + 5), builder.send()).await;
        let response = match send_result {
            Err(_) => {
                if attempt < NETWORK_RETRY_BUDGET {
                    let delay = backoff_delay(attempt);
                    emit_retry(
                        &mut on_retry,
                        attempt,
                        NETWORK_RETRY_BUDGET,
                        "timeout",
                        "request timed out",
                        delay,
                    );
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                    continue;
                }
                return Err("AI request timed out".to_string());
            }
            Ok(Err(error)) => {
                if attempt < NETWORK_RETRY_BUDGET && is_transient_reqwest_error(&error) {
                    let delay = backoff_delay(attempt);
                    emit_retry(
                        &mut on_retry,
                        attempt,
                        NETWORK_RETRY_BUDGET,
                        retry_reason_for_error(&error),
                        "connection failed",
                        delay,
                    );
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                    continue;
                }
                return Err(error.to_string());
            }
            Ok(Ok(response)) => response,
        };

        let status = response.status().as_u16();
        if status >= 400 {
            let budget = retry_budget_for_status(status);
            if attempt < budget && is_transient_status(status) {
                let delay = transient_retry_delay(status, response.headers(), attempt);
                emit_retry(
                    &mut on_retry,
                    attempt,
                    budget,
                    retry_reason_for_status(status),
                    format!("HTTP {status}"),
                    delay,
                );
                tokio::time::sleep(delay).await;
                attempt += 1;
                continue;
            }
            let body = response.json::<Value>().await.unwrap_or(Value::Null);
            return Err(response_error(status, &body));
        }

        // Read the body as text first so a non-JSON response (HTML error page, an
        // empty body, or an SSE stream returned to a non-stream request) yields an
        // actionable message instead of the opaque "error decoding response body".
        let text = response
            .text()
            .await
            .map_err(|error| format!("Failed to read AI provider response: {error}"))?;
        let body = serde_json::from_str::<Value>(&text).map_err(|_| {
            let preview: String = text.trim().chars().take(180).collect();
            if preview.is_empty() {
                "AI provider returned an empty response. Check the model id, base URL, and that the endpoint is an OpenAI-compatible /chat/completions.".to_string()
            } else {
                format!("AI provider returned a non-JSON response (is the base URL correct?): {preview}")
            }
        })?;
        // Map Anthropic's Messages response back to the OpenAI shape callers parse.
        let body = if anthropic {
            crate::ai_anthropic::from_anthropic_response(&body)
        } else {
            body
        };
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
// The future is awaited inline by the native turn loop (never `tokio::spawn`ed),
// so the non-Send `should_cancel: Fn` closure it holds across awaits is fine.
#[allow(clippy::future_not_send)]
pub async fn completion_streaming<F, C, R>(
    request: AiChatCompletionRequest,
    mut on_delta: F,
    should_cancel: C,
    mut on_retry: R,
) -> Result<AiChatCompletionResponse, String>
where
    F: FnMut(&str, &str),
    C: Fn() -> bool,
    R: FnMut(RetryNotice),
{
    let anthropic = crate::ai_anthropic::is_anthropic(&request.protocol);
    let endpoint = if anthropic {
        crate::ai_anthropic::messages_endpoint(&request.base_url)?
    } else {
        completion_endpoint(&request.base_url)?
    };
    // Use connect_timeout only — not a whole-request timeout — so a long but
    // actively-streaming agent turn is never killed by a per-request deadline.
    // Genuine idle stalls are caught per-chunk below via tokio::select!.
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CHAT_TIMEOUT_SECS))
        .build()
        .map_err(|error| error.to_string())?;
    let stream_ready = stream_payload(request.payload);
    let payload = if anthropic {
        crate::ai_anthropic::to_anthropic_request(&stream_ready)
    } else {
        stream_ready
    };
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
            let builder = client
                .post(endpoint.as_str())
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .json(&payload);
            let builder = apply_auth(builder, anthropic, api_key.as_deref());
            // Race the connect/send against the cancel flag so a Stop pressed while
            // connecting to a slow/stalled provider interrupts within one poll tick
            // instead of waiting out the full send deadline.
            let send = match race_cancel(
                builder.send(),
                Duration::from_secs(CHAT_TIMEOUT_SECS + 5),
                &should_cancel,
            )
            .await
            {
                CancelRace::Ready(result) => Ok(result),
                CancelRace::Cancelled => return Err("cancelled".to_string()),
                CancelRace::TimedOut => Err(()),
            };
            let response = match send {
                Err(()) => {
                    if attempt < NETWORK_RETRY_BUDGET {
                        let delay = backoff_delay(attempt);
                        emit_retry(
                            &mut on_retry,
                            attempt,
                            NETWORK_RETRY_BUDGET,
                            "timeout",
                            "request timed out",
                            delay,
                        );
                        // Cancellable sleep: a Stop during retry backoff exits immediately.
                        if sleep_backoff_cancelable(attempt, &should_cancel).await {
                            return Err("cancelled".to_string());
                        }
                        attempt += 1;
                        continue;
                    }
                    return Err("AI stream request timed out".to_string());
                }
                Ok(Err(error)) => {
                    if attempt < NETWORK_RETRY_BUDGET && is_transient_reqwest_error(&error) {
                        let delay = backoff_delay(attempt);
                        emit_retry(
                            &mut on_retry,
                            attempt,
                            NETWORK_RETRY_BUDGET,
                            retry_reason_for_error(&error),
                            "connection failed",
                            delay,
                        );
                        if sleep_backoff_cancelable(attempt, &should_cancel).await {
                            return Err("cancelled".to_string());
                        }
                        attempt += 1;
                        continue;
                    }
                    return Err(error.to_string());
                }
                Ok(Ok(response)) => response,
            };
            let status = response.status().as_u16();
            if status >= 400 {
                let budget = retry_budget_for_status(status);
                if attempt < budget && is_transient_status(status) {
                    let delay = transient_retry_delay(status, response.headers(), attempt);
                    emit_retry(
                        &mut on_retry,
                        attempt,
                        budget,
                        retry_reason_for_status(status),
                        format!("HTTP {status}"),
                        delay,
                    );
                    if sleep_backoff_cancelable(attempt, &should_cancel).await {
                        return Err("cancelled".to_string());
                    }
                    attempt += 1;
                    continue;
                }
                let text = response.text().await.unwrap_or_default();
                return Err(stream_response_error(status, &text));
            }
            break response;
        }
    };

    let mut accumulator = if anthropic {
        StreamMode::Anthropic(AnthropicStreamAccumulator::default())
    } else {
        StreamMode::OpenAi(StreamAccumulator::default())
    };
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    // Full decoded body, kept so a provider that ignores `stream:true` and returns a
    // single non-SSE JSON object (some gateways do) can still be parsed below — the
    // SSE loop would otherwise find no `data:` events and finalize an empty answer.
    let mut raw_body = String::new();
    // Whether any SSE `data:` event was actually ingested. If not, raw_body is parsed
    // as a complete (non-streamed) response.
    let mut ingested_event = false;
    // Carry an incomplete trailing UTF-8 sequence across chunks: bytes_stream()
    // splits on arbitrary byte boundaries, so decoding each chunk independently
    // would mangle multibyte code points (Cyrillic/CJK/emoji) into U+FFFD.
    let mut byte_tail: Vec<u8> = Vec::new();
    'outer: loop {
        // Per-chunk wait that honours both the idle deadline AND a Stop. `race_cancel`
        // polls `should_cancel` while awaiting the next chunk, so a Stop pressed during
        // a *silent* stall (provider sending no bytes) interrupts within one poll tick
        // instead of being noticed only after a chunk finally arrives — or never, until
        // the CHAT_TIMEOUT_SECS idle deadline. Aborts only true stalls (no bytes for
        // CHAT_TIMEOUT_SECS), never long-but-actively-streaming generations. Matches the
        // intent of the Tauri event-streaming path (`stream_completion`).
        let chunk = match race_cancel(
            stream.next(),
            Duration::from_secs(CHAT_TIMEOUT_SECS),
            &should_cancel,
        )
        .await
        {
            CancelRace::Ready(chunk) => chunk,
            // The in-flight response (and its socket) is dropped on return; the caller's
            // post-stream cancellation check finalizes the accumulated-so-far body.
            CancelRace::Cancelled => break 'outer,
            CancelRace::TimedOut => return Err("AI stream stalled".to_string()),
        };
        let Some(chunk) = chunk else { break 'outer };

        // A Stop that landed while this chunk was already in flight (arriving faster
        // than one poll tick): bail before processing it so the model's remaining
        // generation isn't drained. The next iteration's race_cancel entry check would
        // also catch it, but this keeps the original zero-latency per-chunk guarantee.
        if should_cancel() {
            break 'outer;
        }
        let bytes = chunk.map_err(|error| stream_chunk_error(&error))?;
        // Keep a bounded copy of the raw body for the non-SSE fallback below.
        if raw_body.len() < MAX_SSE_BUFFER {
            raw_body.push_str(&String::from_utf8_lossy(&bytes));
        }
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
            if let Some(invalid_len) = invalid_len {
                buffer.push('\u{FFFD}');
                byte_tail.drain(..valid_up_to + invalid_len);
            } else {
                byte_tail.drain(..valid_up_to);
                break;
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
                ingested_event = true;
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
            ingested_event = true;
            accumulator.ingest(&value, &mut on_delta);
        }
    }

    // Non-SSE fallback: some gateways ignore `stream:true` and return a single plain
    // JSON object (no `data:` events). The SSE loops above then ingest nothing and the
    // turn would finalize empty. Parse the whole raw body as a complete response and
    // stream it through on_delta so the answer renders. Handles both the Anthropic
    // shape ({content:[{type:text,…}]}) and the OpenAI shape ({choices:[…]}).
    if !ingested_event {
        let trimmed = raw_body.trim();
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            let normalized = if anthropic && value.get("choices").is_none() {
                crate::ai_anthropic::from_anthropic_response(&value)
            } else {
                value
            };
            if let Some(message) = normalized
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(|c| c.get("message"))
            {
                let content = message.get("content").and_then(Value::as_str).unwrap_or("");
                let reasoning = message
                    .get("reasoning_content")
                    .or_else(|| message.get("reasoning"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !content.is_empty() || !reasoning.is_empty() {
                    on_delta(content, reasoning);
                }
                return Ok(AiChatCompletionResponse {
                    status: 200,
                    body: normalized,
                });
            }
        }
    }

    // Emit any buffered partial `<think>` tail so the last fragment isn't dropped.
    accumulator.flush(&mut on_delta);

    // A mid-stream operational error (Anthropic delivers overload/rate-limit/
    // api_error as a typed SSE event on an already-200 stream) must fail the turn,
    // not be finalized as a truncated success — route it through the same error/
    // retry path as a non-200 connection-phase failure.
    if let Some(error) = accumulator.stream_error() {
        return Err(error.to_string());
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
    // Inline-`<think>` extraction state. Many local proxies don't map a model's
    // thinking onto `reasoning_content`/`reasoning`; they emit a leading
    // `<think>…</think>` block inside `content`. We strip that block out of the
    // answer and route it to the reasoning channel so the UI shows it as thinking.
    in_think: bool,
    think_resolved: bool,
    think_carry: String,
    /// A mid-stream error event (`{"error":{...}}`) sent by an OpenAI-compatible
    /// gateway on an already-200 SSE stream. Must fail the turn rather than being
    /// finalized as empty/truncated content.
    stream_error: Option<String>,
}

/// Opening / closing inline-thinking tags recognized in streamed `content`.
const THINK_OPEN_TAGS: [&str; 2] = ["<think>", "<thinking>"];
const THINK_CLOSE_TAGS: [&str; 2] = ["</think>", "</thinking>"];

#[derive(Default)]
struct StreamToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl StreamAccumulator {
    fn ingest<F: FnMut(&str, &str)>(&mut self, value: &Value, on_delta: &mut F) {
        // OpenAI-compatible gateways can send `{"error":{...}}` on a 200 stream.
        // Capture the first one so the turn fails instead of returning empty content.
        if self.stream_error.is_none() {
            let err_msg = value
                .get("error")
                .and_then(|e| {
                    e.get("message")
                        .and_then(Value::as_str)
                        .or_else(|| e.as_str())
                })
                .or_else(|| {
                    // Some gateways send a top-level `{"message":"..."}` error shape.
                    if value.get("choices").is_none() {
                        value.get("message").and_then(Value::as_str)
                    } else {
                        None
                    }
                });
            if let Some(msg) = err_msg {
                let kind = value
                    .get("error")
                    .and_then(|e| e.get("code").or_else(|| e.get("type")))
                    .and_then(Value::as_str);
                self.stream_error = Some(match kind {
                    Some(k) => format!("AI provider stream error ({k}): {msg}"),
                    None => format!("AI provider stream error: {msg}"),
                });
                return;
            }
        }

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
        let raw_content = delta.get("content").and_then(Value::as_str).unwrap_or("");
        // Reasoning the provider reports explicitly (various field shapes).
        let explicit_reasoning = extract_reasoning_field(delta);
        // A provider that reports reasoning through its own field is not inlining a
        // `<think>` block, so its answer content must be left verbatim (it may even
        // legitimately contain a literal `<think>`). Stop scanning for inline think.
        if !explicit_reasoning.is_empty() {
            self.think_resolved = true;
        }
        // Reasoning carried inline as a `<think>` block inside `content`.
        let (content, inline_reasoning) = if raw_content.is_empty() {
            (String::new(), String::new())
        } else {
            self.split_inline_think(raw_content)
        };
        let mut reasoning = explicit_reasoning;
        reasoning.push_str(&inline_reasoning);

        if !content.is_empty() {
            self.content.push_str(&content);
        }
        if !reasoning.is_empty() {
            self.reasoning.push_str(&reasoning);
        }
        if !content.is_empty() || !reasoning.is_empty() {
            on_delta(&content, &reasoning);
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                self.merge_tool_call(call);
            }
        }
    }

    /// Split a streamed `content` chunk into (answer, thinking), extracting an
    /// inline `<think>…</think>` block. The block is only recognized at the very
    /// start of the answer (thinking always precedes the answer), so a literal
    /// `<think>` appearing later in real content is left untouched. Tags split
    /// across chunks are handled via `think_carry`.
    fn split_inline_think(&mut self, chunk: &str) -> (String, String) {
        self.think_carry.push_str(chunk);
        let mut out_content = String::new();
        let mut out_reasoning = String::new();
        loop {
            if self.in_think {
                if let Some((index, len)) = find_tag(&self.think_carry, &THINK_CLOSE_TAGS) {
                    out_reasoning.push_str(&self.think_carry[..index]);
                    self.think_carry.drain(..index + len);
                    self.in_think = false;
                    self.think_resolved = true;
                    continue;
                }
                // No close tag yet: emit reasoning but hold back a possible partial
                // close-tag suffix that might complete in the next chunk.
                let keep = partial_tag_tail(&self.think_carry, &THINK_CLOSE_TAGS);
                let emit_to = self.think_carry.len() - keep;
                out_reasoning.push_str(&self.think_carry[..emit_to]);
                self.think_carry.drain(..emit_to);
                break;
            }
            if self.think_resolved {
                out_content.push_str(&self.think_carry);
                self.think_carry.clear();
                break;
            }
            // Undecided: a `<think>` block is only valid as the leading content.
            let lead_ws = self.think_carry.len() - self.think_carry.trim_start().len();
            let rest = &self.think_carry[lead_ws..];
            if rest.is_empty() {
                // Only whitespace so far. A `<think>` may still follow it in the next
                // chunk, so hold the whitespace back rather than committing it to the
                // answer (flush() will emit it if the stream ends here).
                break;
            }
            if let Some(len) = prefix_tag(rest, &THINK_OPEN_TAGS) {
                // Confirmed leading think block: drop the whitespace that preceded
                // it (pre-think formatting) along with the open tag.
                self.think_carry.drain(..lead_ws + len);
                self.in_think = true;
                continue;
            }
            if is_tag_prefix(rest, &THINK_OPEN_TAGS) {
                // `rest` could still grow into an opening tag — hold everything
                // (including leading whitespace) until the next chunk decides.
                break;
            }
            // Definitely not a leading think block — stop scanning for one.
            self.think_resolved = true;
        }
        (out_content, out_reasoning)
    }

    /// Flush any buffered partial tag at end of stream so nothing is dropped.
    fn flush<F: FnMut(&str, &str)>(&mut self, on_delta: &mut F) {
        if self.think_carry.is_empty() {
            return;
        }
        let carry = std::mem::take(&mut self.think_carry);
        if self.in_think {
            self.reasoning.push_str(&carry);
            on_delta("", &carry);
        } else {
            self.content.push_str(&carry);
            on_delta(&carry, "");
        }
    }

    fn merge_tool_call(&mut self, call: &Value) {
        // A single response never has more than a handful of tool calls. Clamp the
        // attacker-controlled `index` to a hard ceiling so a hostile endpoint
        // streaming `{"index":4e9}` can't drive the `while push` loop into a
        // multi-gigabyte allocation / abort (H8).
        const MAX_TOOL_CALLS: usize = 256;
        let index = match call.get("index").and_then(Value::as_u64) {
            Some(value) => usize::try_from(value)
                .unwrap_or(usize::MAX)
                .min(MAX_TOOL_CALLS - 1),
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

/// Reassembles an Anthropic Messages SSE stream into the same OpenAI-style response
/// body the rest of the transport expects. Anthropic emits typed events
/// (`message_start`, `content_block_start/delta/stop`, `message_delta`, …) keyed by
/// content-block `index`; text → answer, `thinking` → reasoning, `tool_use` +
/// `input_json_delta` → an `OpenAI` tool call whose arguments stream as a JSON
/// fragment. Mixed text/tool blocks keep their content-block index; empty (text)
/// slots are filtered out of the final tool-call list.
#[derive(Default)]
struct AnthropicStreamAccumulator {
    content: String,
    reasoning: String,
    tool_calls: Vec<StreamToolCall>,
    stop_reason: Option<String>,
    input_tokens: u64,
    output_tokens: u64,
    cache_read: u64,
    cache_creation: u64,
    /// A mid-stream `error` event (overload / rate limit / `api_error`). Anthropic
    /// delivers these on an already-200 stream, so the connection-phase status
    /// check can't catch them; the caller must surface this as a failure instead
    /// of finalizing a truncated answer as success.
    error: Option<String>,
}

impl AnthropicStreamAccumulator {
    fn ingest<F: FnMut(&str, &str)>(&mut self, value: &Value, on_delta: &mut F) {
        match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "message_start" => {
                if let Some(usage) = value.pointer("/message/usage") {
                    self.input_tokens += usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    self.cache_read += usage
                        .get("cache_read_input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    self.cache_creation += usage
                        .get("cache_creation_input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                }
            }
            "content_block_start" => {
                let block = value.get("content_block");
                if block.and_then(|b| b.get("type")).and_then(Value::as_str) == Some("tool_use") {
                    let index = block_index(value);
                    self.ensure_slot(index);
                    let slot = &mut self.tool_calls[index];
                    if let Some(id) = block.and_then(|b| b.get("id")).and_then(Value::as_str) {
                        slot.id = id.to_string();
                    }
                    if let Some(name) = block.and_then(|b| b.get("name")).and_then(Value::as_str) {
                        slot.name = name.to_string();
                    }
                    // A tool_use that ships its full input up front (no streamed
                    // input_json_delta) seeds arguments here.
                    if let Some(input) = block.and_then(|b| b.get("input")) {
                        if input.as_object().is_some_and(|object| !object.is_empty()) {
                            slot.arguments = input.to_string();
                        }
                    }
                }
            }
            "content_block_delta" => {
                let delta = value.get("delta");
                match delta.and_then(|d| d.get("type")).and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(text) =
                            delta.and_then(|d| d.get("text")).and_then(Value::as_str)
                        {
                            self.content.push_str(text);
                            on_delta(text, "");
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = delta
                            .and_then(|d| d.get("thinking"))
                            .and_then(Value::as_str)
                        {
                            self.reasoning.push_str(text);
                            on_delta("", text);
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(fragment) = delta
                            .and_then(|d| d.get("partial_json"))
                            .and_then(Value::as_str)
                        {
                            let index = block_index(value);
                            self.ensure_slot(index);
                            self.tool_calls[index].arguments.push_str(fragment);
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(reason) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = Some(reason.to_string());
                }
                if let Some(output) = value
                    .pointer("/usage/output_tokens")
                    .and_then(Value::as_u64)
                {
                    self.output_tokens += output;
                }
            }
            // Anthropic streams operational failures as a typed event on an
            // already-200 stream (often after some tokens). Capture the first one so
            // the caller fails the turn instead of returning truncated content.
            "error" if self.error.is_none() => {
                let detail = value.pointer("/error/message").and_then(Value::as_str);
                let kind = value.pointer("/error/type").and_then(Value::as_str);
                self.error = Some(match (kind, detail) {
                    (Some(kind), Some(detail)) => {
                        format!("AI provider stream error ({kind}): {detail}")
                    }
                    (Some(kind), None) => format!("AI provider stream error: {kind}"),
                    (None, Some(detail)) => format!("AI provider stream error: {detail}"),
                    (None, None) => "AI provider stream error".to_string(),
                });
            }
            // ping / content_block_stop / message_stop carry nothing to fold.
            _ => {}
        }
    }

    fn ensure_slot(&mut self, index: usize) {
        // Bound an attacker-controlled content-block index the same way the OpenAI
        // accumulator clamps tool-call indices.
        const MAX_TOOL_CALLS: usize = 256;
        let index = index.min(MAX_TOOL_CALLS - 1);
        while self.tool_calls.len() <= index {
            self.tool_calls.push(StreamToolCall::default());
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
                    "function": {
                        "name": tc.name,
                        "arguments": if tc.arguments.is_empty() { "{}".to_string() } else { tc.arguments },
                    },
                })
            })
            .collect();
        let has_tools = !tool_calls.is_empty();
        if has_tools {
            message["tool_calls"] = Value::Array(tool_calls);
        }
        serde_json::json!({
            "choices": [{
                "index": 0,
                "message": message,
                "finish_reason": crate::ai_anthropic::finish_reason(self.stop_reason.as_deref(), has_tools),
            }],
            "usage": crate::ai_anthropic::anthropic_usage(self.input_tokens, self.output_tokens, Some(&serde_json::json!({
                "cache_read_input_tokens": self.cache_read,
                "cache_creation_input_tokens": self.cache_creation,
            }))),
        })
    }
}

/// `index` field of an Anthropic content-block event, defaulting to 0.
fn block_index(value: &Value) -> usize {
    value
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(0)
}

/// Streaming accumulator that adapts to the provider protocol so the single SSE
/// reassembly loop in `completion_streaming` stays protocol-agnostic.
enum StreamMode {
    OpenAi(StreamAccumulator),
    Anthropic(AnthropicStreamAccumulator),
}

impl StreamMode {
    fn ingest<F: FnMut(&str, &str)>(&mut self, value: &Value, on_delta: &mut F) {
        match self {
            Self::OpenAi(acc) => acc.ingest(value, on_delta),
            Self::Anthropic(acc) => acc.ingest(value, on_delta),
        }
    }

    fn flush<F: FnMut(&str, &str)>(&mut self, on_delta: &mut F) {
        // Only the OpenAI accumulator buffers a partial inline-think tail.
        if let Self::OpenAi(acc) = self {
            acc.flush(on_delta);
        }
    }

    /// A mid-stream operational error captured during ingest (Anthropic typed
    /// event or OpenAI-compatible `{"error":...}` frame on a 200 stream).
    /// `None` means the stream completed without a typed error event.
    fn stream_error(&self) -> Option<&str> {
        match self {
            Self::Anthropic(acc) => acc.error.as_deref(),
            Self::OpenAi(acc) => acc.stream_error.as_deref(),
        }
    }

    fn into_response_body(self) -> Value {
        match self {
            Self::OpenAi(acc) => acc.into_response_body(),
            Self::Anthropic(acc) => acc.into_response_body(),
        }
    }
}

/// Pull a provider's explicitly-reported reasoning out of a streamed delta,
/// tolerating the several shapes providers use: `reasoning_content`, a string
/// `reasoning`, `thinking`, or a `reasoning: { content | text }` object.
fn extract_reasoning_field(delta: &Value) -> String {
    for key in ["reasoning_content", "reasoning", "thinking"] {
        if let Some(text) = delta.get(key).and_then(Value::as_str) {
            if !text.is_empty() {
                return text.to_string();
            }
        }
    }
    if let Some(object) = delta.get("reasoning").and_then(Value::as_object) {
        for key in ["content", "text"] {
            if let Some(text) = object.get(key).and_then(Value::as_str) {
                if !text.is_empty() {
                    return text.to_string();
                }
            }
        }
    }
    String::new()
}

/// First (lowest-index) occurrence of any `tags` entry in `haystack`, matched
/// case-insensitively. Returns `(byte_index, tag_len)`. Tags are ASCII, so
/// lowercasing preserves byte positions and lengths.
fn find_tag(haystack: &str, tags: &[&str]) -> Option<(usize, usize)> {
    let lower = haystack.to_ascii_lowercase();
    tags.iter()
        .filter_map(|tag| lower.find(tag).map(|index| (index, tag.len())))
        .min_by_key(|(index, _)| *index)
}

/// `Some(tag_len)` when `text` starts with one of `tags` (case-insensitive).
fn prefix_tag(text: &str, tags: &[&str]) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    tags.iter()
        .find(|tag| lower.starts_with(**tag))
        .map(|tag| tag.len())
}

/// True when `text` is a non-empty *proper* prefix of some tag (so the tag may
/// still complete in a later chunk).
fn is_tag_prefix(text: &str, tags: &[&str]) -> bool {
    let lower = text.to_ascii_lowercase();
    !lower.is_empty()
        && tags
            .iter()
            .any(|tag| lower.len() < tag.len() && tag.starts_with(&lower))
}

/// Length of the longest suffix of `s` that is a prefix of some tag — i.e. how
/// many trailing bytes to hold back in case a tag is split across chunks.
fn partial_tag_tail(s: &str, tags: &[&str]) -> usize {
    let lower = s.to_ascii_lowercase();
    let n = lower.len();
    let max = tags
        .iter()
        .map(|t| t.len())
        .max()
        .unwrap_or(0)
        .saturating_sub(1)
        .min(n);
    for k in (1..=max).rev() {
        if !s.is_char_boundary(n - k) {
            continue;
        }
        let suffix = &lower[n - k..];
        if tags.iter().any(|tag| tag.starts_with(suffix)) {
            return k;
        }
    }
    0
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
    let serialized = serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?;

    // Serialize all writes through a process-level mutex. A unique temp-file id
    // per write (defense in depth) ensures two callers that somehow both acquire
    // the lock sequentially each operate on their own temp path.
    let _guard = history_save_lock()
        .lock()
        .map_err(|_| "history save lock poisoned".to_string())?;

    let write_id = uuid::Uuid::new_v4().to_string();
    let temporary_path = history_temp_path(&path, &write_id);
    std::fs::write(&temporary_path, serialized).map_err(|error| error.to_string())?;
    // Atomic replace: on POSIX `rename` is atomic; on Windows it may fail if the
    // destination exists (NTFS), so remove first under the lock.
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
    let anthropic = crate::ai_anthropic::is_anthropic(&request.protocol);
    let endpoint = if anthropic {
        crate::ai_anthropic::messages_endpoint(&request.base_url)?
    } else {
        completion_endpoint(&request.base_url)?
    };
    let payload = if anthropic {
        crate::ai_anthropic::to_anthropic_request(&request.payload)
    } else {
        request.payload.clone()
    };
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
    let builder = client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&payload);
    let api_key = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    let builder = apply_auth(builder, anthropic, api_key);

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

/// Process-level mutex that serializes all history writes. A single shared temp
/// path plus concurrent callers is a classic read-modify-rename race: two saves
/// could overwrite each other's temp file, or a rename could follow a different
/// caller's delete. Holding this lock for the entire write+rename sequence turns
/// concurrent autosaves into safe sequential ones.
fn history_save_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

/// Unique temp path for each write: `ai-chat-history.<uuid>.tmp` in the same
/// directory. A per-write unique name means even without the process mutex two
/// writes can never clobber the same temp file — defense in depth on Windows
/// where the rename is also atomic.
fn history_temp_path(path: &Path, id: &str) -> PathBuf {
    path.with_extension(format!("{id}.tmp"))
}

// Returns `Result` for call-site symmetry with the rest of the history I/O path,
// even though recovery is best-effort and never surfaces an error.
#[allow(clippy::unnecessary_wraps)]
fn recover_history_temp_file(path: &Path) -> Result<(), String> {
    // Main file is intact — nothing to recover.
    if path.exists() {
        return Ok(());
    }
    // Scan the directory for any leftover `ai-chat-history.*.tmp` files written
    // by the new per-write unique-id scheme and recover the newest one.
    let Some(dir) = path.parent() else {
        return Ok(());
    };
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("ai-chat-history");
    let candidate = std::fs::read_dir(dir).ok().and_then(|entries| {
        entries
            .filter_map(std::result::Result::ok)
            .filter(|e| {
                let name = e.file_name();
                let s = name.to_string_lossy();
                // Match both the legacy single-path `ai-chat-history.json.tmp`
                // and the new per-write `ai-chat-history.<uuid>.tmp` names.
                s.starts_with(stem) && s.ends_with(".tmp")
            })
            .max_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
    });
    if let Some(entry) = candidate {
        let _ = std::fs::rename(entry.path(), path);
    }
    Ok(())
}

/// Attach provider auth to a request builder. Anthropic uses `x-api-key` plus the
/// required `anthropic-version` header; every other (OpenAI-compatible) provider
/// uses bearer auth.
fn apply_auth(
    builder: reqwest::RequestBuilder,
    anthropic: bool,
    api_key: Option<&str>,
) -> reqwest::RequestBuilder {
    if anthropic {
        let builder = builder.header("anthropic-version", crate::ai_anthropic::ANTHROPIC_VERSION);
        return match api_key {
            Some(key) => builder.header("x-api-key", key),
            None => builder,
        };
    }
    match api_key {
        Some(key) => builder.bearer_auth(key),
        None => builder,
    }
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
///
/// `403` is included deliberately: edge/CDN layers in front of OpenAI-compatible
/// providers (Cloudflare challenges, regional WAFs, brief key-propagation gaps)
/// routinely return a transient `403` that clears on a retry. A genuinely
/// permanent forbidden just costs the two bounded backoff attempts before the
/// real error surfaces.
const fn is_transient_status(status: u16) -> bool {
    matches!(status, 403 | 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

/// Network-level reqwest errors that are safe to retry (connect/timeout/request).
fn is_transient_reqwest_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

/// Exponential backoff: 0.5s, 1s, 2s … capped at the retry ceiling.
/// Automatic-retry backoff: a gentle LINEAR ladder instead of an exponential jump.
/// attempt 0 → 1s, then +3s per step: 1, 3, 6, 9, 12, 15, … capped at the ceiling.
/// This rides out a blip without the old "wait 30s on the 3rd try" cliff, and the
/// ladder is shallow enough that all ~10 attempts fit a reasonable window.
fn backoff_delay(attempt: u32) -> Duration {
    let secs = if attempt == 0 {
        1
    } else {
        (3 * u64::from(attempt)).min(MAX_RETRY_DELAY_SECS)
    };
    Duration::from_secs(secs)
}

async fn sleep_backoff(attempt: u32) {
    tokio::time::sleep(backoff_delay(attempt)).await;
}

/// Cancellable backoff sleep: sleeps in 250ms increments and checks `should_cancel`
/// after each tick. Returns `true` if the sleep was interrupted by cancellation.
/// This lets `completion_streaming` honour a Stop during connection-phase retries
/// without requiring a full `CancellationToken` plumbing through every call site.
#[allow(clippy::future_not_send)] // awaited inline; `should_cancel` need not be Send.
async fn sleep_backoff_cancelable<C: Fn() -> bool>(attempt: u32, should_cancel: &C) -> bool {
    const TICK_MS: u64 = 250;
    let total = backoff_delay(attempt);
    let ticks = u32::try_from((total.as_millis() / u128::from(TICK_MS)).max(1)).unwrap_or(u32::MAX);
    for _ in 0..ticks {
        if should_cancel() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(TICK_MS)).await;
    }
    should_cancel()
}

/// How a `race_cancel` wait ended: the awaited future completed, the poll-based
/// `should_cancel` predicate fired, or the overall idle deadline elapsed.
enum CancelRace<T> {
    Ready(T),
    Cancelled,
    TimedOut,
}

/// Await `fut` while honouring a poll-based cancellation predicate and an overall
/// idle deadline. The native turn loop only exposes cancellation as a `Fn() -> bool`
/// flag (no async primitive), so a bare `tokio::select!` cannot wake on Stop. This
/// races the future against a `CANCEL_POLL_MS` ticker that checks `should_cancel`
/// and a `deadline` timeout, so a Stop pressed during `builder.send()` or a silent
/// stalled stream is observed within one tick instead of after the whole deadline.
/// Without this, the per-chunk `tokio::select!` only checked cancellation *after* a
/// chunk arrived — a provider that stops sending bytes would ignore Stop for up to
/// `CHAT_TIMEOUT_SECS`.
#[allow(clippy::future_not_send)] // awaited inline; `should_cancel` need not be Send.
async fn race_cancel<T, Fut, C>(fut: Fut, deadline: Duration, should_cancel: &C) -> CancelRace<T>
where
    Fut: std::future::Future<Output = T>,
    C: Fn() -> bool,
{
    const CANCEL_POLL_MS: u64 = 200;
    if should_cancel() {
        return CancelRace::Cancelled;
    }
    tokio::pin!(fut);
    let started = std::time::Instant::now();
    loop {
        let remaining = match deadline.checked_sub(started.elapsed()) {
            Some(left) if !left.is_zero() => left,
            _ => return CancelRace::TimedOut,
        };
        let tick = remaining.min(Duration::from_millis(CANCEL_POLL_MS));
        tokio::select! {
            output = &mut fut => return CancelRace::Ready(output),
            () = tokio::time::sleep(tick) => {
                if should_cancel() {
                    return CancelRace::Cancelled;
                }
            }
        }
    }
}

/// Pick a backoff for a transient HTTP failure. A `Retry-After` header always
/// wins (the server told us exactly how long). A rate limit (429) without that
/// header gets a longer floor than the generic backoff — 0.5s/1s does nothing
/// against a real limit, so wait long enough to have a genuine chance (and to
/// make the live "retrying in Ns" notice actually visible).
fn transient_retry_delay(
    status: u16,
    headers: &reqwest::header::HeaderMap,
    attempt: u32,
) -> Duration {
    if let Some(delay) = retry_after_delay(headers) {
        return delay;
    }
    // 429 and the rest share the linear ladder (1s, 3s, 6s, …). A server-provided
    // Retry-After above always wins when present.
    let _ = status;
    backoff_delay(attempt)
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

/// One bounded automatic-retry notice, emitted right before a backoff sleep so a
/// caller can surface "retrying because X (attempt n/m)" to the user. Retries only
/// happen in the connection phase (before any token streams), so a notice always
/// means nothing has been shown yet and the request is safe to replay.
#[derive(Debug, Clone)]
pub struct RetryNotice {
    /// 1-based number of the upcoming attempt (the first try is attempt 1).
    pub attempt: u32,
    /// Total attempts that will be made before giving up.
    pub max_attempts: u32,
    /// Machine reason: `rate-limited` | `server` | `forbidden` | `timeout` | `network`.
    pub reason: String,
    /// Short human detail (e.g. `HTTP 429`).
    pub detail: String,
    /// How long the backoff will wait before the retry.
    pub delay_ms: u64,
}

/// Emit a retry notice for the upcoming attempt. `attempt` is the loop's 0-based
/// retry counter, so the upcoming try is `attempt + 2` (try 1 already failed).
/// `budget` is the per-failure retry ceiling, so the notice reads "attempt n of
/// budget+1" and reflects the real number of tries this error type will get.
fn emit_retry<R: FnMut(RetryNotice)>(
    on_retry: &mut R,
    attempt: u32,
    budget: u32,
    reason: &str,
    detail: impl Into<String>,
    delay: Duration,
) {
    let max_attempts = budget + 1;
    on_retry(RetryNotice {
        attempt: (attempt + 2).min(max_attempts),
        max_attempts,
        reason: reason.to_string(),
        detail: detail.into(),
        delay_ms: u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
    });
}

/// Map a transient HTTP status to a stable retry reason the UI can label.
const fn retry_reason_for_status(status: u16) -> &'static str {
    match status {
        429 => "rate-limited",
        403 => "forbidden",
        408 | 425 => "timeout",
        _ => "server",
    }
}

/// Map a retryable reqwest error to a stable retry reason.
fn retry_reason_for_error(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else {
        "network"
    }
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
    // Respect the provider protocol — Anthropic needs a different endpoint,
    // auth headers, and request translation. Previously this path always used
    // `/chat/completions` + bearer auth, breaking Anthropic streaming entirely.
    let anthropic = crate::ai_anthropic::is_anthropic(&request.protocol);
    let endpoint = if anthropic {
        crate::ai_anthropic::messages_endpoint(&request.base_url)?
    } else {
        completion_endpoint(&request.base_url)?
    };
    let base_payload = if anthropic {
        crate::ai_anthropic::to_anthropic_request(&request.payload)
    } else {
        request.payload.clone()
    };
    // No total request timeout here: a long agent turn may actively stream for
    // longer than CHAT_TIMEOUT_SECS and must not be aborted mid-generation. The
    // connection phase is bounded by connect_timeout (and the wrapping send
    // timeout); genuine stalls are caught by the per-chunk idle timeout below.
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CHAT_TIMEOUT_SECS))
        .build()
        .map_err(|error| error.to_string())?;
    let payload = stream_payload(base_payload);
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
            let builder = client
                .post(endpoint.as_str())
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .json(&payload);
            // Use provider-correct auth (x-api-key + anthropic-version vs bearer).
            let builder = apply_auth(builder, anthropic, api_key.as_deref());

            let send = tokio::select! {
                _ = &mut cancel_rx => return Ok(StreamCompletion::Cancelled),
                send = timeout(Duration::from_secs(CHAT_TIMEOUT_SECS + 5), builder.send()) => send,
            };

            let response = match send {
                Err(_) => {
                    if attempt < MAX_TRANSIENT_RETRIES {
                        // Cancellable sleep: a Stop during backoff exits immediately.
                        tokio::select! {
                            _ = &mut cancel_rx => return Ok(StreamCompletion::Cancelled),
                            () = sleep_backoff(attempt) => {}
                        }
                        attempt += 1;
                        continue;
                    }
                    return Err("AI stream request timed out".to_string());
                }
                Ok(Err(error)) => {
                    if attempt < MAX_TRANSIENT_RETRIES && is_transient_reqwest_error(&error) {
                        tokio::select! {
                            _ = &mut cancel_rx => return Ok(StreamCompletion::Cancelled),
                            () = sleep_backoff(attempt) => {}
                        }
                        attempt += 1;
                        continue;
                    }
                    return Err(error.to_string());
                }
                Ok(Ok(response)) => response,
            };

            let status = response.status().as_u16();
            if status >= 400 {
                // Per-status retry budget: a 403/4xx is not a transient blip, so it
                // gets a small budget (4) rather than the full rate-limit budget (9),
                // matching `completion_streaming` (M6 — previously 403 was hammered
                // up to 9×).
                let budget = retry_budget_for_status(status);
                if attempt < budget && is_transient_status(status) {
                    let delay = retry_after_delay(response.headers())
                        .unwrap_or_else(|| backoff_delay(attempt));
                    tokio::select! {
                        _ = &mut cancel_rx => return Ok(StreamCompletion::Cancelled),
                        () = tokio::time::sleep(delay) => {}
                    }
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
    // Full decoded body, kept so a provider that ignores `stream:true` and
    // returns a single non-SSE JSON object can still be parsed below.
    let mut raw_body = String::new();
    // Whether any SSE `data:` event was actually forwarded to the frontend.
    let mut ingested_event = false;
    // Carry an incomplete trailing UTF-8 sequence across chunks.
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
        let bytes = chunk.map_err(|error| stream_chunk_error(&error))?;
        if raw_body.len() < MAX_SSE_BUFFER {
            raw_body.push_str(&String::from_utf8_lossy(&bytes));
        }
        byte_tail.extend_from_slice(&bytes);
        // Drain byte_tail fully each chunk, handling partial multibyte sequences.
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
            if let Some(invalid_len) = invalid_len {
                buffer.push('\u{FFFD}');
                byte_tail.drain(..valid_up_to + invalid_len);
            } else {
                byte_tail.drain(..valid_up_to);
                break;
            }
        }
        normalize_sse_buffer_newlines(&mut buffer);
        if buffer.len() > MAX_SSE_BUFFER {
            return Err("AI stream buffer exceeded limit".to_string());
        }
        if emit_stream_sse_events(app, stream_id, &mut buffer, &mut ingested_event)? {
            return Ok(StreamCompletion::Done);
        }
    }

    // Flush any trailing bytes.
    if !byte_tail.is_empty() {
        buffer.push_str(&String::from_utf8_lossy(&byte_tail));
    }
    normalize_sse_buffer_newlines(&mut buffer);
    if emit_stream_sse_events(app, stream_id, &mut buffer, &mut ingested_event)? {
        return Ok(StreamCompletion::Done);
    }
    if !buffer.trim().is_empty()
        && emit_stream_sse_event(app, stream_id, buffer.trim(), &mut ingested_event)?
    {
        return Ok(StreamCompletion::Done);
    }

    // Non-SSE fallback: some gateways ignore `stream:true` and return a plain
    // JSON object. Parse it and emit a synthetic chunk+done so the frontend
    // renders the answer instead of seeing an empty completed turn.
    if !ingested_event {
        let trimmed = raw_body.trim();
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            let normalized = if anthropic && value.get("choices").is_none() {
                crate::ai_anthropic::from_anthropic_response(&value)
            } else {
                value
            };
            // Only emit if the response carries actual content/tool_calls.
            let has_content = normalized
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty());
            let has_tools = normalized
                .pointer("/choices/0/message/tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty());
            if has_content || has_tools {
                emit_stream_event(
                    app,
                    AiChatStreamEvent {
                        stream_id: stream_id.to_string(),
                        kind: "chunk".to_string(),
                        data: Some(serde_json::json!({
                            "choices": [{ "delta": normalized.pointer("/choices/0/message")
                                .cloned().unwrap_or(Value::Null) }]
                        })),
                        error: None,
                    },
                )?;
            }
        }
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

/// Turn a mid-stream `bytes_stream()` failure into an actionable message.
///
/// reqwest surfaces a dropped/transcoded streaming body as the opaque "error
/// decoding response body", which tells the user nothing and isn't recognized by
/// the frontend error classifier (so it shows as a bare "AI request failed").
/// Tagging it as a stream error routes it to the retry-able "stream" branch and
/// names the real cause (the connection dropped mid-generation).
fn stream_chunk_error(error: &reqwest::Error) -> String {
    let detail = error.to_string();
    if detail.contains("error decoding response body") {
        return "AI provider stream interrupted: the connection dropped mid-response. Retry to continue.".to_string();
    }
    format!("AI provider stream interrupted: {detail}")
}

fn normalize_sse_buffer_newlines(buffer: &mut String) {
    if buffer.contains('\r') {
        *buffer = buffer.replace("\r\n", "\n").replace('\r', "\n");
    }
}

/// Drain all complete SSE events from `buffer` (delimited by `\n\n`), emitting
/// each to the Tauri event bus. Returns `true` when a `[DONE]` sentinel is found
/// (caller should stop draining). `ingested` is set to `true` whenever a real
/// `data:` event (not keep-alive or [DONE]) is forwarded — used by the non-SSE
/// fallback to detect whether the provider spoke SSE at all.
fn emit_stream_sse_events(
    app: &AppHandle,
    stream_id: &str,
    buffer: &mut String,
    ingested: &mut bool,
) -> Result<bool, String> {
    while let Some(index) = buffer.find("\n\n") {
        let event = buffer[..index].to_string();
        buffer.drain(..index + 2);
        if emit_stream_sse_event(app, stream_id, &event, ingested)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn emit_stream_sse_event(
    app: &AppHandle,
    stream_id: &str,
    event: &str,
    ingested: &mut bool,
) -> Result<bool, String> {
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
    *ingested = true;
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

    // Drive the accumulator with a sequence of content/reasoning deltas and return
    // the final (answer, thinking) after flush.
    fn run_stream(chunks: &[(&str, &str)]) -> (String, String) {
        let mut acc = StreamAccumulator::default();
        let mut seen_content = String::new();
        let mut seen_reasoning = String::new();
        let mut on_delta = |c: &str, r: &str| {
            seen_content.push_str(c);
            seen_reasoning.push_str(r);
        };
        for (content, reasoning) in chunks {
            let mut delta = serde_json::Map::new();
            if !content.is_empty() {
                delta.insert("content".into(), Value::String((*content).into()));
            }
            if !reasoning.is_empty() {
                delta.insert(
                    "reasoning_content".into(),
                    Value::String((*reasoning).into()),
                );
            }
            let value = serde_json::json!({ "choices": [{ "delta": Value::Object(delta) }] });
            acc.ingest(&value, &mut on_delta);
        }
        acc.flush(&mut on_delta);
        // The streamed deltas and the accumulated body must agree.
        assert_eq!(seen_content, acc.content);
        assert_eq!(seen_reasoning, acc.reasoning);
        (acc.content, acc.reasoning)
    }

    #[test]
    fn inline_think_block_is_routed_to_reasoning() {
        let (content, reasoning) =
            run_stream(&[("<think>planning the answer</think>Hello world", "")]);
        assert_eq!(content, "Hello world");
        assert_eq!(reasoning, "planning the answer");
    }

    #[test]
    fn inline_think_split_across_chunks() {
        // Tag boundaries land mid-token across deltas.
        let (content, reasoning) = run_stream(&[
            ("<thi", ""),
            ("nk>step one ", ""),
            ("step two</thi", ""),
            ("nk>Final ", ""),
            ("answer", ""),
        ]);
        assert_eq!(content, "Final answer");
        assert_eq!(reasoning, "step one step two");
    }

    #[test]
    fn content_without_think_is_untouched() {
        let (content, reasoning) = run_stream(&[("Just ", ""), ("a normal answer", "")]);
        assert_eq!(content, "Just a normal answer");
        assert_eq!(reasoning, "");
    }

    #[test]
    fn think_tag_mid_answer_is_not_stripped() {
        // A `<think>` that is not the leading content stays in the answer.
        let (content, reasoning) = run_stream(&[("Use the ", ""), ("<think> HTML tag here", "")]);
        assert_eq!(content, "Use the <think> HTML tag here");
        assert_eq!(reasoning, "");
    }

    #[test]
    fn explicit_reasoning_field_still_works() {
        let (content, reasoning) = run_stream(&[("", "deep thought"), ("the answer", "")]);
        assert_eq!(content, "the answer");
        assert_eq!(reasoning, "deep thought");
    }

    #[test]
    fn thinking_variant_and_case_insensitive() {
        let (content, reasoning) = run_stream(&[("<THINKING>hmm</Thinking>done", "")]);
        assert_eq!(content, "done");
        assert_eq!(reasoning, "hmm");
    }

    #[test]
    fn unterminated_think_flushes_as_reasoning() {
        let (content, reasoning) =
            run_stream(&[("<think>still thinking when the stream ended", "")]);
        assert_eq!(content, "");
        assert_eq!(reasoning, "still thinking when the stream ended");
    }

    #[test]
    fn explicit_reasoning_disables_inline_think_stripping() {
        // Provider reports reasoning via its own field, then its answer legitimately
        // starts with a literal `<think>` (e.g. talking about the tag). It must NOT
        // be treated as a thinking block.
        let (content, reasoning) =
            run_stream(&[("", "real reasoning"), ("<think> is an HTML-ish tag", "")]);
        assert_eq!(content, "<think> is an HTML-ish tag");
        assert_eq!(reasoning, "real reasoning");
    }

    #[test]
    fn whitespace_then_think_split_across_chunks_does_not_leak_space() {
        // A leading space arrives alone, then `<think>` in the next chunk: the space
        // must not leak into the answer ahead of the (stripped) think block.
        let (content, reasoning) = run_stream(&[(" ", ""), ("<think>x</think>answer", "")]);
        assert_eq!(reasoning, "x");
        assert_eq!(content.trim(), "answer");
        assert!(
            !content.starts_with(' '),
            "leading space must not precede the answer"
        );
    }

    #[test]
    fn leading_whitespace_before_think_is_dropped_with_the_block() {
        let (content, reasoning) = run_stream(&[("\n<think>x</think>answer", "")]);
        assert_eq!(reasoning, "x");
        assert_eq!(content, "answer");
    }

    #[test]
    fn sse_event_data_collects_multiline_data_and_ignores_comments() {
        let event = ": keep-alive\nevent: message\ndata: {\"a\":\ndata: 1}\n";

        assert_eq!(sse_event_data(event).as_deref(), Some("{\"a\":\n1}"));
    }

    #[test]
    fn rate_limit_without_header_uses_longer_floor() {
        let empty = reqwest::header::HeaderMap::new();
        // All transient statuses share the linear ladder now: 1s, then 3s, 6s, …
        assert_eq!(
            transient_retry_delay(429, &empty, 0),
            Duration::from_secs(1)
        );
        assert_eq!(
            transient_retry_delay(429, &empty, 1),
            Duration::from_secs(3)
        );
        assert_eq!(
            transient_retry_delay(503, &empty, 0),
            Duration::from_secs(1)
        );
        // A Retry-After header always wins, even for 429.
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "5".parse().unwrap());
        assert_eq!(
            transient_retry_delay(429, &headers, 0),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn retry_reason_classification() {
        assert_eq!(retry_reason_for_status(429), "rate-limited");
        assert_eq!(retry_reason_for_status(403), "forbidden");
        assert_eq!(retry_reason_for_status(408), "timeout");
        assert_eq!(retry_reason_for_status(425), "timeout");
        assert_eq!(retry_reason_for_status(500), "server");
        assert_eq!(retry_reason_for_status(503), "server");
    }

    #[test]
    fn emit_retry_reports_1_based_attempt_and_total() {
        // The 0-based loop counter maps to a 1-based "attempt n of budget+1" the UI
        // shows: the first failure (attempt 0) is the upcoming try 2.
        let mut seen: Vec<RetryNotice> = Vec::new();
        let mut on_retry = |notice: RetryNotice| seen.push(notice);
        emit_retry(
            &mut on_retry,
            0,
            9,
            "rate-limited",
            "HTTP 429",
            Duration::from_millis(500),
        );
        emit_retry(
            &mut on_retry,
            1,
            9,
            "server",
            "HTTP 503",
            Duration::from_secs(1),
        );
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].attempt, 2);
        assert_eq!(seen[0].max_attempts, 10);
        assert_eq!(seen[0].reason, "rate-limited");
        assert_eq!(seen[0].delay_ms, 500);
        assert_eq!(seen[1].attempt, 3);
        // The notice reflects the real per-failure ceiling, not a global one.
        let mut net: Option<RetryNotice> = None;
        emit_retry(
            &mut |n: RetryNotice| net = Some(n),
            0,
            NETWORK_RETRY_BUDGET,
            "network",
            "",
            Duration::ZERO,
        );
        assert_eq!(net.unwrap().max_attempts, NETWORK_RETRY_BUDGET + 1);
        // Attempt never advertises past the total even if the counter would overflow it.
        let mut last: Option<RetryNotice> = None;
        emit_retry(
            &mut |n: RetryNotice| last = Some(n),
            99,
            9,
            "timeout",
            "",
            Duration::ZERO,
        );
        assert_eq!(last.unwrap().attempt, 10);
    }

    #[test]
    fn retry_budget_differs_by_failure() {
        // Rate limit / server / overload recover by waiting → full budget.
        assert_eq!(retry_budget_for_status(429), MAX_TRANSIENT_RETRIES);
        assert_eq!(retry_budget_for_status(503), MAX_TRANSIENT_RETRIES);
        assert_eq!(retry_budget_for_status(500), MAX_TRANSIENT_RETRIES);
        // Edge 403 / request-timeout statuses clear less often → fewer tries.
        assert_eq!(retry_budget_for_status(403), 4);
        assert_eq!(retry_budget_for_status(408), 4);
        // Network blips ride the full ~10-attempt ladder now.
        assert_eq!(NETWORK_RETRY_BUDGET, 9);
    }

    #[test]
    fn stream_payload_forces_stream_true() {
        let payload = serde_json::json!({ "model": "gpt-5.5", "stream": false });

        let payload = stream_payload(payload);

        assert_eq!(payload.get("stream").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn transient_status_classification() {
        for status in [403, 408, 425, 429, 500, 502, 503, 504] {
            assert!(is_transient_status(status), "{status} should be transient");
        }
        for status in [400, 401, 404, 422] {
            assert!(
                !is_transient_status(status),
                "{status} should not be transient"
            );
        }
    }

    #[test]
    fn backoff_grows_and_caps() {
        // Linear ladder: 1s, then +3s per step (1, 3, 6, 9, 12, …).
        assert_eq!(backoff_delay(0), Duration::from_secs(1));
        assert_eq!(backoff_delay(1), Duration::from_secs(3));
        assert_eq!(backoff_delay(2), Duration::from_secs(6));
        assert_eq!(backoff_delay(3), Duration::from_secs(9));
        assert_eq!(backoff_delay(4), Duration::from_secs(12));
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

    #[test]
    fn anthropic_stream_accumulates_text_thinking_and_tool_use() {
        let mut acc = AnthropicStreamAccumulator::default();
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut on_delta = |c: &str, r: &str| {
            content.push_str(c);
            reasoning.push_str(r);
        };
        let events = [
            serde_json::json!({ "type": "message_start", "message": { "usage": { "input_tokens": 20, "cache_read_input_tokens": 4 } } }),
            serde_json::json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "thinking" } }),
            serde_json::json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "thinking_delta", "thinking": "let me think" } }),
            serde_json::json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "text" } }),
            serde_json::json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "text_delta", "text": "Hello " } }),
            serde_json::json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "text_delta", "text": "world" } }),
            serde_json::json!({ "type": "content_block_start", "index": 2, "content_block": { "type": "tool_use", "id": "tu1", "name": "Read" } }),
            serde_json::json!({ "type": "content_block_delta", "index": 2, "delta": { "type": "input_json_delta", "partial_json": "{\"path\":" } }),
            serde_json::json!({ "type": "content_block_delta", "index": 2, "delta": { "type": "input_json_delta", "partial_json": "\"a.rs\"}" } }),
            serde_json::json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" }, "usage": { "output_tokens": 9 } }),
            serde_json::json!({ "type": "message_stop" }),
        ];
        for event in &events {
            acc.ingest(event, &mut on_delta);
        }
        assert_eq!(content, "Hello world");
        assert_eq!(reasoning, "let me think");
        let body = acc.into_response_body();
        assert_eq!(body["choices"][0]["message"]["content"], "Hello world");
        assert_eq!(
            body["choices"][0]["message"]["reasoning_content"],
            "let me think"
        );
        let call = &body["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(call["id"], "tu1");
        assert_eq!(call["function"]["name"], "Read");
        assert_eq!(call["function"]["arguments"], "{\"path\":\"a.rs\"}");
        assert_eq!(body["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(body["usage"]["prompt_tokens"], 20);
        assert_eq!(body["usage"]["completion_tokens"], 9);
        assert_eq!(body["usage"]["total_tokens"], 29);
        assert_eq!(body["usage"]["cache_read_input_tokens"], 4);
    }

    #[test]
    fn anthropic_stream_plain_text_has_stop_finish_and_no_tools() {
        let mut acc = AnthropicStreamAccumulator::default();
        let mut noop = |_: &str, _: &str| {};
        for event in [
            serde_json::json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text" } }),
            serde_json::json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "done" } }),
            serde_json::json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" }, "usage": { "output_tokens": 2 } }),
        ] {
            acc.ingest(&event, &mut noop);
        }
        let body = acc.into_response_body();
        assert_eq!(body["choices"][0]["finish_reason"], "stop");
        assert!(body["choices"][0]["message"].get("tool_calls").is_none());
    }

    #[test]
    fn anthropic_stream_captures_mid_stream_error_event() {
        let mut acc = AnthropicStreamAccumulator::default();
        let mut noop = |_: &str, _: &str| {};
        for event in [
            serde_json::json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text" } }),
            serde_json::json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "partial" } }),
            serde_json::json!({ "type": "error", "error": { "type": "overloaded_error", "message": "Overloaded" } }),
        ] {
            acc.ingest(&event, &mut noop);
        }
        // The error is captured so the caller fails the turn instead of returning
        // the truncated "partial" content as a success.
        let error = acc.error.clone().expect("error event should be captured");
        assert!(error.contains("overloaded_error"));
        assert!(error.contains("Overloaded"));
        // The StreamMode wrapper surfaces it to completion_streaming.
        let mode = StreamMode::Anthropic(acc);
        assert!(mode.stream_error().is_some());
    }

    #[test]
    fn openai_stream_captures_mid_stream_error_frame() {
        // OpenAI-compatible gateways send `{"error":{...}}` on a 200 SSE stream.
        let mut acc = StreamAccumulator::default();
        let mut noop = |_: &str, _: &str| {};

        // A partial content frame followed by an error frame.
        acc.ingest(
            &serde_json::json!({"choices":[{"delta":{"content":"partial"}}]}),
            &mut noop,
        );
        acc.ingest(
            &serde_json::json!({"error":{"code":"rate_limit_exceeded","message":"Rate limit hit"}}),
            &mut noop,
        );

        let mode = StreamMode::OpenAi(acc);
        let err = mode.stream_error().expect("error must be captured");
        assert!(
            err.contains("rate_limit_exceeded"),
            "error should include code"
        );
        assert!(
            err.contains("Rate limit hit"),
            "error should include message"
        );
    }

    #[test]
    fn openai_stream_error_does_not_swallow_partial_content() {
        // Even when an error arrives, the partial content accumulated before it
        // should still be accessible via into_response_body.
        let mut acc = StreamAccumulator::default();
        let mut noop = |_: &str, _: &str| {};
        acc.ingest(
            &serde_json::json!({"choices":[{"delta":{"content":"hello"}}]}),
            &mut noop,
        );
        acc.ingest(
            &serde_json::json!({"error":{"message":"context length exceeded"}}),
            &mut noop,
        );
        // Content accumulated before the error is preserved.
        assert_eq!(acc.content, "hello");
        assert!(acc.stream_error.is_some());
    }

    #[tokio::test]
    async fn race_cancel_returns_ready_when_future_completes_first() {
        let never_cancel = || false;
        let fut = async { 42_u32 };
        match race_cancel(fut, Duration::from_secs(5), &never_cancel).await {
            CancelRace::Ready(v) => assert_eq!(v, 42),
            _ => panic!("a completed future must yield Ready"),
        }
    }

    #[tokio::test]
    async fn race_cancel_fires_on_pending_cancellation() {
        // A future that never completes plus a cancel flag that is already set must
        // return Cancelled promptly (before the deadline) — this is the silent-stall
        // case where a Stop must interrupt even though no bytes are arriving.
        let always_cancel = || true;
        let fut = std::future::pending::<()>();
        let started = std::time::Instant::now();
        match race_cancel(fut, Duration::from_secs(30), &always_cancel).await {
            CancelRace::Cancelled => {}
            _ => panic!("a set cancel flag must yield Cancelled"),
        }
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "cancellation must not wait out the deadline"
        );
    }

    #[tokio::test]
    async fn race_cancel_times_out_on_idle() {
        let never_cancel = || false;
        let fut = std::future::pending::<()>();
        match race_cancel(fut, Duration::from_millis(50), &never_cancel).await {
            CancelRace::TimedOut => {}
            _ => panic!("an idle wait past the deadline must yield TimedOut"),
        }
    }
}
