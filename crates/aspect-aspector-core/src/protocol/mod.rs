use serde_json::Value;

pub mod request;
pub mod response;
pub mod tools;

pub use request::*;
pub use response::*;

/// API version header value. Stable contract version Anthropic recommends pinning.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// True when a provider's protocol selects the Anthropic Messages API.
pub const fn is_anthropic(protocol: &str) -> bool {
    let bytes = protocol.as_bytes();
    let target = b"anthropic";
    if bytes.len() != target.len() {
        return false;
    }
    let mut i = 0;
    while i < target.len() {
        if bytes[i].to_ascii_lowercase() != target[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Resolve the `/v1/messages` endpoint from a provider base URL. Mirrors
/// `completion_endpoint`: tolerates a trailing slash and a base that already points
/// at `/chat/completions` (rewritten) or `/messages` (kept).
pub fn messages_endpoint(base_url: &str) -> Result<String, String> {
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
    if text.ends_with("/messages") {
        return Ok(text.to_string());
    }
    let root = text.strip_suffix("/chat/completions").unwrap_or(text);
    Ok(format!("{root}/messages"))
}

/// Anthropic `stop_reason` values that must be surfaced to the caller even though
/// `finish_reason` normalizes them to `OpenAI`'s "stop" for model-agnostic
/// consumers. Currently just `refusal` (Anthropic's safety-refusal signal) —
/// silently normalizing it away would hide a distinction a caller may want to log
/// or act on.
pub fn stop_reason_needs_marker(stop_reason: Option<&str>) -> bool {
    stop_reason == Some("refusal")
}

/// Map an Anthropic `stop_reason` (plus whether tool calls were emitted) onto an
/// `OpenAI` `finish_reason`. `refusal` (the model declined on safety grounds) and
/// `pause_turn` (a long-running server-side tool call was paused mid-turn, not a
/// real end) both normalize to `"stop"` here for model-agnostic callers — the
/// distinct Anthropic value survives separately via `anthropic_stop_reason`
/// (`stop_reason_needs_marker`) so a caller that cares can still see it.
/// `stop_sequence` (a caller-supplied stop string was hit) is treated the same as
/// a normal end-of-turn; `OpenAI` has no closer equivalent than `"stop"`.
pub fn finish_reason(stop_reason: Option<&str>, has_tools: bool) -> &'static str {
    match stop_reason {
        Some("tool_use") => "tool_calls",
        Some("max_tokens") => "length",
        _ if has_tools => "tool_calls",
        _ => "stop",
    }
}

/// Build an `OpenAI`-shaped `usage` object that also carries Anthropic's native field
/// names, so `accumulate_usage` (which reads either shape) and cache-token parsing
/// both work.
pub fn anthropic_usage(input_tokens: u64, output_tokens: u64, raw: Option<&Value>) -> Value {
    let cache_read = raw
        .and_then(|u| u.get("cache_read_input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_creation = raw
        .and_then(|u| u.get("cache_creation_input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    serde_json::json!({
        "prompt_tokens": input_tokens,
        "completion_tokens": output_tokens,
        "total_tokens": input_tokens + output_tokens,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "cache_read_input_tokens": cache_read,
        "cache_creation_input_tokens": cache_creation,
    })
}
