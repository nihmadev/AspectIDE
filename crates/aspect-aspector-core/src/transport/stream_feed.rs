use std::time::Duration;

use futures_util::StreamExt;
use serde_json::Value;
use tokio::sync::oneshot;

use crate::protocol;

use super::auth::apply_auth;
use super::endpoints::{is_transient_reqwest_error, is_transient_status, CHAT_TIMEOUT_SECS, MAX_TRANSIENT_RETRIES, MAX_SSE_BUFFER};
use super::retry::{backoff_delay, retry_after_delay, retry_budget_for_status, stream_chunk_error, stream_response_error};
use super::sse::{normalize_sse_buffer_newlines, sse_event_data, sse_stream_error};
use super::types::{AiChatCompletionStreamRequest, StreamCompletion};

/// Platform abstraction for emitting stream events to the UI.
pub trait EventEmitter: Send + Sync {
    fn emit_stream(&self, stream_id: &str, kind: &str, data: Option<Value>, error: Option<String>) -> Result<(), String>;
}

/// Run a streaming completion, emitting SSE events via the emitter.
pub async fn run_completion_stream<E: EventEmitter + Send + Sync + 'static>(
    emitter: E,
    stream_id: String,
    request: AiChatCompletionStreamRequest,
    cancel_rx: oneshot::Receiver<()>,
) {
    let result = stream_completion(&emitter, &stream_id, request, cancel_rx).await;

    match result {
        Ok(StreamCompletion::Done) => {}
        Ok(StreamCompletion::Cancelled) => {
            let _ = emitter.emit_stream(&stream_id, "cancelled", None, None);
        }
        Err(error) => {
            let _ = emitter.emit_stream(&stream_id, "error", None, Some(error));
        }
    }
}

async fn stream_completion(
    emitter: &dyn EventEmitter,
    stream_id: &str,
    request: AiChatCompletionStreamRequest,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<StreamCompletion, String> {
    let anthropic = protocol::is_anthropic(&request.protocol);
    let endpoint = if anthropic {
        protocol::messages_endpoint(&request.base_url)?
    } else {
        super::endpoints::completion_endpoint(&request.base_url)?
    };
    let base_payload = if anthropic {
        protocol::to_anthropic_request(&request.payload)
    } else {
        request.payload.clone()
    };

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

    let response = {
        let mut attempt: u32 = 0;
        loop {
            let builder = client
                .post(endpoint.as_str())
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .json(&payload);
            let builder = apply_auth(builder, anthropic, api_key.as_deref());

            let send = tokio::select! {
                _ = &mut cancel_rx => return Ok(StreamCompletion::Cancelled),
                send = tokio::time::timeout(Duration::from_secs(CHAT_TIMEOUT_SECS + 5), builder.send()) => send,
            };

            let response = match send {
                Err(_) => {
                    if attempt < MAX_TRANSIENT_RETRIES {
                        tokio::select! {
                            _ = &mut cancel_rx => return Ok(StreamCompletion::Cancelled),
                            () = super::retry::sleep_backoff(attempt) => {}
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
                            () = super::retry::sleep_backoff(attempt) => {}
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
                    let delay = retry_after_delay(response.headers())
                        .unwrap_or_else(|| backoff_delay(attempt));
                    tokio::select! {
                        _ = &mut cancel_rx => return Ok(StreamCompletion::Cancelled),
                        () = tokio::time::sleep(delay) => {}
                    }
                    attempt += 1;
                    continue;
                }
                let text = tokio::select! {
                    _ = &mut cancel_rx => return Ok(StreamCompletion::Cancelled),
                    body = tokio::time::timeout(Duration::from_secs(15), response.text()) => {
                        body.map(Result::unwrap_or_default).unwrap_or_default()
                    }
                };
                return Err(stream_response_error(status, &text));
            }
            break response;
        }
    };

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut raw_bytes: Vec<u8> = Vec::new();
    let mut ingested_event = false;
    let mut byte_tail: Vec<u8> = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = &mut cancel_rx => return Ok(StreamCompletion::Cancelled),
            chunk = tokio::time::timeout(Duration::from_secs(CHAT_TIMEOUT_SECS), stream.next()) => match chunk {
                Ok(chunk) => chunk,
                Err(_) => return Err("AI stream stalled".to_string()),
            },
        };

        let Some(chunk) = chunk else { break };
        let bytes = chunk.map_err(|error| stream_chunk_error(&error))?;
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
        if emit_stream_sse_events(emitter, stream_id, &mut buffer, &mut ingested_event)? {
            return Ok(StreamCompletion::Done);
        }
    }

    if !byte_tail.is_empty() {
        buffer.push_str(&String::from_utf8_lossy(&byte_tail));
    }
    normalize_sse_buffer_newlines(&mut buffer);
    if emit_stream_sse_events(emitter, stream_id, &mut buffer, &mut ingested_event)? {
        return Ok(StreamCompletion::Done);
    }
    if !buffer.trim().is_empty()
        && emit_stream_sse_event(emitter, stream_id, buffer.trim(), &mut ingested_event)?
    {
        return Ok(StreamCompletion::Done);
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
                    emitter,
                    stream_id,
                    "chunk",
                    Some(serde_json::json!({
                        "choices": [{ "delta": normalized.pointer("/choices/0/message")
                            .cloned().unwrap_or(Value::Null) }]
                    })),
                    None,
                )?;
            }
        }
    }

    emit_stream_event(emitter, stream_id, "done", None, None)?;
    Ok(StreamCompletion::Done)
}

fn stream_payload(mut payload: Value) -> Value {
    if let Value::Object(object) = &mut payload {
        object.insert("stream".to_string(), Value::Bool(true));
    }
    payload
}

fn emit_stream_sse_events(
    emitter: &dyn EventEmitter,
    stream_id: &str,
    buffer: &mut String,
    ingested: &mut bool,
) -> Result<bool, String> {
    while let Some(index) = buffer.find("\n\n") {
        let event = buffer[..index].to_string();
        buffer.drain(..index + 2);
        if emit_stream_sse_event(emitter, stream_id, &event, ingested)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn emit_stream_sse_event(
    emitter: &dyn EventEmitter,
    stream_id: &str,
    event: &str,
    ingested: &mut bool,
) -> Result<bool, String> {
    let Some(data) = sse_event_data(event) else {
        return Ok(false);
    };
    if data.trim() == "[DONE]" {
        emit_stream_event(emitter, stream_id, "done", None, None)?;
        return Ok(true);
    }
    if data.trim().is_empty() {
        return Ok(false);
    }
    let Ok(value) = serde_json::from_str::<Value>(&data) else {
        return Ok(false);
    };
    if let Some(message) = sse_stream_error(&value) {
        return Err(message);
    }
    *ingested = true;
    emit_stream_event(emitter, stream_id, "chunk", Some(value), None)?;
    Ok(false)
}

fn emit_stream_event(
    emitter: &dyn EventEmitter,
    stream_id: &str,
    kind: &str,
    data: Option<Value>,
    error: Option<String>,
) -> Result<(), String> {
    emitter.emit_stream(stream_id, kind, data, error)
}
