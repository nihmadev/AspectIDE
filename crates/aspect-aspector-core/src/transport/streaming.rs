use std::time::Duration;

use futures_util::StreamExt;
use serde_json::Value;

use crate::protocol;

use super::auth::apply_auth;
use super::endpoints::{is_transient_reqwest_error, is_transient_status, CHAT_TIMEOUT_SECS, STREAM_CONNECT_TIMEOUT_SECS, TCP_KEEPALIVE_SECS, NETWORK_RETRY_BUDGET, MAX_SSE_BUFFER};
use super::race::{race_cancel, sleep_backoff_cancelable};
use super::retry::{backoff_delay, emit_retry, retry_budget_for_status, retry_reason_for_error, retry_reason_for_status, stream_chunk_error, stream_response_error};
use super::sse::{normalize_sse_buffer_newlines, sse_event_data};
use super::stream_mode::StreamMode;
use super::stream_acc::StreamAccumulator;
use super::anthropic_acc::AnthropicStreamAccumulator;
use super::types::{AiChatCompletionRequest, AiChatCompletionResponse, RetryNotice, CancelRace};

/// Streaming completion for the native turn-loop.
pub async fn completion_streaming<F, C, R, T>(
    request: AiChatCompletionRequest,
    mut on_delta: F,
    should_cancel: C,
    mut on_retry: R,
    mut on_tool_start: T,
) -> Result<AiChatCompletionResponse, String>
where
    F: FnMut(&str, &str),
    C: Fn() -> bool,
    R: FnMut(RetryNotice),
    T: FnMut(&str),
{
    let anthropic = protocol::is_anthropic(&request.protocol);
    let endpoint = if anthropic {
        protocol::messages_endpoint(&request.base_url)?
    } else {
        super::endpoints::completion_endpoint(&request.base_url)?
    };
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(STREAM_CONNECT_TIMEOUT_SECS))
        .tcp_keepalive(Duration::from_secs(TCP_KEEPALIVE_SECS))
        .build()
        .map_err(|error| error.to_string())?;
    let stream_ready = stream_payload(request.payload);
    let payload = if anthropic {
        protocol::to_anthropic_request(&stream_ready)
    } else {
        stream_ready
    };
    let api_key = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(ToString::to_string);

    let mut attempt: u32 = 0;
    let emitted_any = std::sync::atomic::AtomicBool::new(false);
    'request: loop {
        let response = {
            loop {
                let builder = client
                    .post(endpoint.as_str())
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .header(reqwest::header::ACCEPT, "text/event-stream")
                    .json(&payload);
                let builder = apply_auth(builder, anthropic, api_key.as_deref());

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
                            emit_retry(&mut on_retry, attempt, NETWORK_RETRY_BUDGET, "timeout", "request timed out", delay);
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
                            emit_retry(&mut on_retry, attempt, NETWORK_RETRY_BUDGET, retry_reason_for_error(&error), "connection failed", delay);
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
                        let delay = super::retry::transient_retry_delay(status, response.headers(), attempt);
                        emit_retry(&mut on_retry, attempt, budget, retry_reason_for_status(status), format!("HTTP {status}"), delay);
                        if sleep_backoff_cancelable(attempt, &should_cancel).await {
                            return Err("cancelled".to_string());
                        }
                        attempt += 1;
                        continue;
                    }
                    let text = match race_cancel(response.text(), Duration::from_secs(15), &should_cancel).await {
                        CancelRace::Ready(result) => result.unwrap_or_default(),
                        CancelRace::Cancelled => return Err("cancelled".to_string()),
                        CancelRace::TimedOut => String::new(),
                    };
                    return Err(stream_response_error(status, &text));
                }
                break response;
            }
        };

        let mut delta_emit = |content: &str, reasoning: &str| {
            if !content.is_empty() || !reasoning.is_empty() {
                emitted_any.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            on_delta(content, reasoning);
        };
        let mut tool_emit = |name: &str| {
            emitted_any.store(true, std::sync::atomic::Ordering::Relaxed);
            on_tool_start(name);
        };

        let mut accumulator = if anthropic {
            StreamMode::Anthropic(AnthropicStreamAccumulator::default())
        } else {
            StreamMode::OpenAi(StreamAccumulator::default())
        };
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut raw_bytes: Vec<u8> = Vec::new();
        let mut ingested_event = false;
        let mut byte_tail: Vec<u8> = Vec::new();
        'outer: loop {
            let chunk = match race_cancel(
                stream.next(),
                Duration::from_secs(CHAT_TIMEOUT_SECS),
                &should_cancel,
            )
            .await
            {
                CancelRace::Ready(chunk) => chunk,
                CancelRace::Cancelled => break 'outer,
                CancelRace::TimedOut => {
                    if !emitted_any.load(std::sync::atomic::Ordering::Relaxed)
                        && attempt < NETWORK_RETRY_BUDGET
                    {
                        let delay = backoff_delay(attempt);
                        emit_retry(&mut on_retry, attempt, NETWORK_RETRY_BUDGET, "stream", "stream stalled", delay);
                        if sleep_backoff_cancelable(attempt, &should_cancel).await {
                            return Err("cancelled".to_string());
                        }
                        attempt += 1;
                        continue 'request;
                    }
                    return Err("AI stream stalled".to_string());
                }
            };

            let Some(chunk) = chunk else { break 'outer };
            if should_cancel() {
                break 'outer;
            }
            let bytes = match chunk {
                Ok(bytes) => bytes,
                Err(error) => {
                    if !emitted_any.load(std::sync::atomic::Ordering::Relaxed)
                        && attempt < NETWORK_RETRY_BUDGET
                    {
                        let delay = backoff_delay(attempt);
                        emit_retry(&mut on_retry, attempt, NETWORK_RETRY_BUDGET, "stream", "connection dropped", delay);
                        if sleep_backoff_cancelable(attempt, &should_cancel).await {
                            return Err("cancelled".to_string());
                        }
                        attempt += 1;
                        continue 'request;
                    }
                    return Err(stream_chunk_error(&error));
                }
            };
            if raw_bytes.len() < MAX_SSE_BUFFER {
                raw_bytes.extend_from_slice(&bytes);
            }
            byte_tail.extend_from_slice(&bytes);
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
                    accumulator.ingest(&value, &mut delta_emit, &mut tool_emit);
                }
            }
        }

        if !byte_tail.is_empty() {
            buffer.push_str(&String::from_utf8_lossy(&byte_tail));
        }
        normalize_sse_buffer_newlines(&mut buffer);
        while let Some(index) = buffer.find("\n\n") {
            let event = buffer[..index].to_string();
            buffer.drain(..index + 2);
            let Some(data) = sse_event_data(&event) else { continue };
            if data.trim() == "[DONE]" { break; }
            if let Ok(value) = serde_json::from_str::<Value>(&data) {
                ingested_event = true;
                accumulator.ingest(&value, &mut delta_emit, &mut tool_emit);
            }
        }
        if !buffer.trim().is_empty() {
            if let Some(data) = sse_event_data(buffer.trim()) {
                if data.trim() != "[DONE]" {
                    if let Ok(value) = serde_json::from_str::<Value>(&data) {
                        ingested_event = true;
                        accumulator.ingest(&value, &mut delta_emit, &mut tool_emit);
                    }
                }
            }
        }

        if !ingested_event {
            let decoded = String::from_utf8_lossy(&raw_bytes);
            let trimmed = decoded.trim();
            if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
                let normalized = if anthropic && value.get("choices").is_none() {
                    protocol::from_anthropic_response(&value)
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
                        delta_emit(content, reasoning);
                    }
                    return Ok(AiChatCompletionResponse { status: 200, body: normalized });
                }
            }
        }

        accumulator.flush(&mut delta_emit);

        if let Some(error) = accumulator.stream_error() {
            return Err(error.to_string());
        }

        return Ok(AiChatCompletionResponse { status: 200, body: accumulator.into_response_body() });
    }
}

fn stream_payload(mut payload: Value) -> Value {
    if let Value::Object(object) = &mut payload {
        object.insert("stream".to_string(), Value::Bool(true));
    }
    payload
}
