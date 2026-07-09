use std::time::Duration;

use serde_json::Value;

use super::endpoints::{MAX_RETRY_DELAY_SECS, MAX_TRANSIENT_RETRIES};
use super::types::RetryNotice;

/// Retries worth attempting for a transient HTTP status.
pub const fn retry_budget_for_status(status: u16) -> u32 {
    match status {
        429 | 500 | 502 | 503 | 504 => MAX_TRANSIENT_RETRIES,
        _ => 4,
    }
}

/// Exponential backoff: 1s, then +3s per step capped at `MAX_RETRY_DELAY_SECS`.
pub fn backoff_delay(attempt: u32) -> Duration {
    let secs = if attempt == 0 {
        1
    } else {
        (3 * u64::from(attempt)).min(MAX_RETRY_DELAY_SECS)
    };
    Duration::from_secs(secs)
}

pub async fn sleep_backoff(attempt: u32) {
    tokio::time::sleep(backoff_delay(attempt)).await;
}

/// Honor a numeric `Retry-After` header (seconds), capped to a sane maximum.
pub fn retry_after_delay(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let seconds = headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(Duration::from_secs(seconds.min(MAX_RETRY_DELAY_SECS)))
}

/// Pick a backoff for a transient HTTP failure.
pub fn transient_retry_delay(
    status: u16,
    headers: &reqwest::header::HeaderMap,
    attempt: u32,
) -> Duration {
    if let Some(delay) = retry_after_delay(headers) {
        return delay;
    }
    let _ = status;
    backoff_delay(attempt)
}

/// Emit a retry notice for the upcoming attempt.
pub fn emit_retry<R: FnMut(RetryNotice)>(
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

/// Map a transient HTTP status to a stable retry reason.
pub const fn retry_reason_for_status(status: u16) -> &'static str {
    match status {
        429 => "rate-limited",
        403 => "forbidden",
        408 | 425 => "timeout",
        _ => "server",
    }
}

/// Map a retryable reqwest error to a stable retry reason.
pub fn retry_reason_for_error(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else {
        "network"
    }
}

/// Format an error response from an AI provider.
pub fn response_error(status: u16, body: &Value) -> String {
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

/// Format a streaming error response with status code.
pub fn stream_response_error(status: u16, text: &str) -> String {
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
pub fn stream_chunk_error(error: &reqwest::Error) -> String {
    let detail = error.to_string();
    if detail.contains("error decoding response body") {
        return "AI provider stream interrupted: the connection dropped mid-response. Retry to continue.".to_string();
    }
    format!("AI provider stream interrupted: {detail}")
}
