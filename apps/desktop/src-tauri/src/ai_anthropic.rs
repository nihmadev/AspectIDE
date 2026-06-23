//! Anthropic Messages API adapter.
//!
//! The whole Lux AI stack speaks `OpenAI` Chat Completions internally — payloads are
//! built and responses are parsed in that shape everywhere. A provider configured
//! with the `anthropic` protocol points at Anthropic's native Messages API, which
//! is a different endpoint (`/v1/messages`), a different auth scheme (`x-api-key` +
//! `anthropic-version`), a different request body (top-level `system`, content-block
//! `messages`, required `max_tokens`, a distinct `tools` schema), and a different
//! streaming event shape.
//!
//! This module is the single translation seam: it converts an `OpenAI`-shaped request
//! into the Anthropic Messages request and maps Anthropic responses (streaming and
//! non-streaming) back to the `OpenAI` shape, so the rest of the transport — and the
//! whole turn loop — stays protocol-agnostic. Pure, side-effect-free, and unit
//! tested; the HTTP plumbing lives in `ai_chat_backend`.

use serde_json::{json, Map, Value};

/// API version header value. Stable contract version Anthropic recommends pinning.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic requires `max_tokens`; the `OpenAI` turn payload usually omits it. This
/// is the universal-safe ceiling for the Claude models shipped in the presets
/// (Sonnet/Opus 4.x and 3.5 Haiku all allow ≥ 8192 output tokens).
const DEFAULT_MAX_TOKENS: u64 = 8192;

/// True when a provider's protocol selects the Anthropic Messages API.
pub const fn is_anthropic(protocol: &str) -> bool {
    // `eq_ignore_ascii_case` isn't const, so fold to lowercase bytes by hand to
    // keep this a `const fn` while still matching "Anthropic"/"ANTHROPIC".
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

/// Translate an `OpenAI` Chat Completions request body into an Anthropic Messages
/// request body. Handles: system extraction, user/assistant/tool message mapping
/// (incl. `tool_use`/`tool_result` blocks and vision images), the tool schema, and
/// the required `max_tokens`. `OpenAI`-only fields (`reasoning_effort`, `reasoning`,
/// `stream_options`) are dropped — Anthropic would reject them.
pub fn to_anthropic_request(openai: &Value) -> Value {
    let mut out = Map::new();
    out.insert(
        "model".to_string(),
        openai.get("model").cloned().unwrap_or(Value::Null),
    );

    let mut system_blocks: Vec<Value> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    if let Some(arr) = openai.get("messages").and_then(Value::as_array) {
        for msg in arr {
            match msg.get("role").and_then(Value::as_str).unwrap_or("user") {
                "system" => append_system(msg.get("content"), &mut system_blocks),
                "assistant" => {
                    if let Some(message) = convert_assistant(msg) {
                        messages.push(message);
                    }
                }
                "tool" => messages.push(convert_tool_result(msg)),
                _ => messages.push(convert_user(msg)),
            }
        }
    }
    out.insert(
        "messages".to_string(),
        Value::Array(coalesce_messages(messages)),
    );
    if !system_blocks.is_empty() {
        out.insert("system".to_string(), Value::Array(system_blocks));
    }

    // max_tokens is required by Anthropic; honor an explicit cap, else the default.
    let max_tokens = openai
        .get("max_tokens")
        .or_else(|| openai.get("max_completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_TOKENS);
    out.insert("max_tokens".to_string(), json!(max_tokens));

    if let Some(temperature) = openai.get("temperature") {
        out.insert("temperature".to_string(), temperature.clone());
    }

    if let Some(tools) = openai.get("tools").and_then(Value::as_array) {
        let converted: Vec<Value> = tools.iter().filter_map(convert_tool).collect();
        if !converted.is_empty() {
            out.insert("tools".to_string(), Value::Array(converted));
            out.insert(
                "tool_choice".to_string(),
                convert_tool_choice(openai.get("tool_choice")),
            );
        }
    }

    if let Some(stream) = openai.get("stream") {
        out.insert("stream".to_string(), stream.clone());
    }

    Value::Object(out)
}

/// Map an Anthropic non-streaming Messages response back to an `OpenAI` completion
/// body (`choices[0].message` + `usage`), so callers parse it identically.
pub fn from_anthropic_response(body: &Value) -> Value {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    if let Some(blocks) = body.get("content").and_then(Value::as_array) {
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        content.push_str(text);
                    }
                }
                Some("thinking") => {
                    if let Some(text) = block.get("thinking").and_then(Value::as_str) {
                        reasoning.push_str(text);
                    }
                }
                Some("tool_use") => {
                    let id = block.get("id").and_then(Value::as_str).unwrap_or("");
                    let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                    let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                    tool_calls.push(json!({
                        "id": id,
                        "type": "function",
                        "function": { "name": name, "arguments": input.to_string() },
                    }));
                }
                _ => {}
            }
        }
    }

    let has_tools = !tool_calls.is_empty();
    let mut message = json!({ "role": "assistant", "content": content });
    if !reasoning.is_empty() {
        message["reasoning_content"] = Value::String(reasoning);
    }
    if has_tools {
        message["tool_calls"] = Value::Array(tool_calls);
    }

    let usage = body.get("usage");
    let input_tokens = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .and_then(|u| u.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);

    json!({
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason(body.get("stop_reason").and_then(Value::as_str), has_tools),
        }],
        "usage": anthropic_usage(input_tokens, output_tokens, usage),
    })
}

/// Map an Anthropic `stop_reason` (plus whether tool calls were emitted) onto an
/// `OpenAI` `finish_reason`.
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
    json!({
        "prompt_tokens": input_tokens,
        "completion_tokens": output_tokens,
        "total_tokens": input_tokens + output_tokens,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "cache_read_input_tokens": cache_read,
        "cache_creation_input_tokens": cache_creation,
    })
}

// ── Message translation helpers ──

/// Collect `OpenAI` system content into Anthropic system text blocks. A string
/// becomes one text block; the cache-breakpoint array form (`[{type:text,text,
/// cache_control}]`) is already Anthropic-shaped and passes through with its
/// `cache_control` preserved.
fn append_system(content: Option<&Value>, blocks: &mut Vec<Value>) {
    match content {
        Some(Value::String(text)) if !text.is_empty() => {
            blocks.push(json!({ "type": "text", "text": text }));
        }
        Some(Value::Array(parts)) => {
            for part in parts {
                let Some(text) = part.get("text").and_then(Value::as_str) else {
                    continue;
                };
                let mut block = json!({ "type": "text", "text": text });
                if let Some(cache) = part.get("cache_control") {
                    block["cache_control"] = cache.clone();
                }
                blocks.push(block);
            }
        }
        _ => {}
    }
}

fn convert_user(msg: &Value) -> Value {
    json!({ "role": "user", "content": Value::Array(content_blocks(msg.get("content"))) })
}

/// Convert an assistant message (text + `tool_calls`) into an Anthropic assistant
/// message with text and `tool_use` blocks. Returns `None` for an empty message so
/// it is dropped instead of producing an invalid empty-content block.
fn convert_assistant(msg: &Value) -> Option<Value> {
    let mut blocks: Vec<Value> = Vec::new();
    if let Some(text) = msg.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            blocks.push(json!({ "type": "text", "text": text }));
        }
    }
    if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let id = call.get("id").and_then(Value::as_str).unwrap_or("");
            let function = call.get("function");
            let name = function
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = function
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let input = serde_json::from_str::<Value>(args).unwrap_or_else(|_| json!({}));
            blocks.push(json!({ "type": "tool_use", "id": id, "name": name, "input": input }));
        }
    }
    if blocks.is_empty() {
        return None;
    }
    Some(json!({ "role": "assistant", "content": Value::Array(blocks) }))
}

/// Convert an `OpenAI` `tool` result message into an Anthropic user message carrying a
/// `tool_result` block. Consecutive tool results are merged later by `coalesce`.
fn convert_tool_result(msg: &Value) -> Value {
    let id = msg
        .get("tool_call_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let content = match msg.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    json!({
        "role": "user",
        "content": [{ "type": "tool_result", "tool_use_id": id, "content": content }],
    })
}

/// Translate `OpenAI` message content into Anthropic content blocks. A string becomes
/// one text block; a content-part array maps text parts and `image_url` parts
/// (data-URL → base64 source, otherwise a URL source).
fn content_blocks(content: Option<&Value>) -> Vec<Value> {
    match content {
        Some(Value::String(text)) => vec![json!({ "type": "text", "text": text })],
        Some(Value::Array(parts)) => parts.iter().filter_map(convert_content_part).collect(),
        _ => vec![json!({ "type": "text", "text": "" })],
    }
}

fn convert_content_part(part: &Value) -> Option<Value> {
    match part.get("type").and_then(Value::as_str) {
        Some("text") => part
            .get("text")
            .and_then(Value::as_str)
            .map(|text| json!({ "type": "text", "text": text })),
        Some("image_url") => {
            let url = part.pointer("/image_url/url").and_then(Value::as_str)?;
            Some(image_block(url))
        }
        _ => None,
    }
}

fn image_block(url: &str) -> Value {
    if let Some(rest) = url.strip_prefix("data:") {
        if let Some((meta, data)) = rest.split_once(',') {
            let media_type = meta.split(';').next().unwrap_or("image/png");
            return json!({
                "type": "image",
                "source": { "type": "base64", "media_type": media_type, "data": data },
            });
        }
    }
    json!({ "type": "image", "source": { "type": "url", "url": url } })
}

/// Merge adjacent same-role messages by concatenating their content blocks.
/// Anthropic requires strict user/assistant alternation, and the turn loop emits
/// runs of `tool` results (each mapped to a `user` message) that must collapse into
/// a single user turn.
fn coalesce_messages(messages: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for msg in messages {
        if let Some(last) = out.last_mut() {
            if last.get("role") == msg.get("role") {
                let mut merged = last
                    .get("content")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if let Some(extra) = msg.get("content").and_then(Value::as_array) {
                    merged.extend(extra.iter().cloned());
                }
                last["content"] = Value::Array(merged);
                continue;
            }
        }
        out.push(msg);
    }
    out
}

fn convert_tool(tool: &Value) -> Option<Value> {
    let function = tool.get("function")?;
    let name = function.get("name").and_then(Value::as_str)?;
    let mut out = json!({ "name": name });
    if let Some(description) = function.get("description").and_then(Value::as_str) {
        out["description"] = json!(description);
    }
    out["input_schema"] = function
        .get("parameters")
        .cloned()
        .unwrap_or_else(|| json!({ "type": "object" }));
    Some(out)
}

fn convert_tool_choice(choice: Option<&Value>) -> Value {
    match choice {
        Some(Value::String(value)) if value == "none" => json!({ "type": "none" }),
        Some(Value::String(value)) if value == "required" || value == "any" => {
            json!({ "type": "any" })
        }
        Some(Value::Object(object)) => object
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .map_or_else(
                || json!({ "type": "auto" }),
                |name| json!({ "type": "tool", "name": name }),
            ),
        _ => json!({ "type": "auto" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_protocol_case_insensitively() {
        assert!(is_anthropic("anthropic"));
        assert!(is_anthropic("Anthropic"));
        assert!(!is_anthropic("openai-compatible"));
    }

    #[test]
    fn endpoint_resolution() {
        assert_eq!(
            messages_endpoint("https://api.anthropic.com/v1").unwrap(),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            messages_endpoint("https://api.anthropic.com/v1/").unwrap(),
            "https://api.anthropic.com/v1/messages"
        );
        // A base already pointing at chat/completions is rewritten to messages.
        assert_eq!(
            messages_endpoint("https://gateway.example/v1/chat/completions").unwrap(),
            "https://gateway.example/v1/messages"
        );
        // Already-correct messages endpoint is preserved.
        assert_eq!(
            messages_endpoint("https://gateway.example/v1/messages").unwrap(),
            "https://gateway.example/v1/messages"
        );
    }

    #[test]
    fn translates_system_and_user_and_max_tokens() {
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                { "role": "system", "content": "You are helpful." },
                { "role": "user", "content": "Hello" },
            ],
            "temperature": 0.2,
        });
        let out = to_anthropic_request(&openai);
        assert_eq!(out["model"], "claude-sonnet-4-5");
        assert_eq!(out["max_tokens"], json!(DEFAULT_MAX_TOKENS));
        assert_eq!(out["temperature"], json!(0.2));
        assert_eq!(out["system"][0]["type"], "text");
        assert_eq!(out["system"][0]["text"], "You are helpful.");
        // System message is removed from messages; user remains.
        assert_eq!(out["messages"].as_array().unwrap().len(), 1);
        assert_eq!(out["messages"][0]["role"], "user");
        assert_eq!(out["messages"][0]["content"][0]["text"], "Hello");
    }

    #[test]
    fn preserves_system_cache_control_breakpoint() {
        let openai = json!({
            "model": "claude-opus-4-1",
            "messages": [
                { "role": "system", "content": [
                    { "type": "text", "text": "Big prompt", "cache_control": { "type": "ephemeral" } }
                ]},
                { "role": "user", "content": "Hi" },
            ],
        });
        let out = to_anthropic_request(&openai);
        assert_eq!(out["system"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn translates_tools_and_tool_choice() {
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [{ "role": "user", "content": "go" }],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "Read",
                    "description": "Read a file",
                    "parameters": { "type": "object", "properties": { "path": { "type": "string" } } },
                },
            }],
            "tool_choice": "auto",
        });
        let out = to_anthropic_request(&openai);
        assert_eq!(out["tools"][0]["name"], "Read");
        assert_eq!(out["tools"][0]["description"], "Read a file");
        assert_eq!(out["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(out["tool_choice"]["type"], "auto");
    }

    #[test]
    fn tool_choice_none_is_mapped_and_keeps_tools() {
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [{ "role": "user", "content": "go" }],
            "tools": [{ "type": "function", "function": { "name": "Read", "parameters": {} } }],
            "tool_choice": "none",
        });
        let out = to_anthropic_request(&openai);
        assert_eq!(out["tool_choice"]["type"], "none");
        assert!(out["tools"].is_array());
    }

    #[test]
    fn translates_assistant_tool_use_and_tool_results_with_coalescing() {
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                { "role": "user", "content": "do it" },
                { "role": "assistant", "content": null, "tool_calls": [
                    { "id": "c1", "type": "function", "function": { "name": "Read", "arguments": "{\"path\":\"a.rs\"}" } },
                    { "id": "c2", "type": "function", "function": { "name": "Glob", "arguments": "{}" } },
                ]},
                { "role": "tool", "tool_call_id": "c1", "content": "file a" },
                { "role": "tool", "tool_call_id": "c2", "content": "globbed" },
            ],
        });
        let out = to_anthropic_request(&openai);
        let messages = out["messages"].as_array().unwrap();
        // user, assistant(2 tool_use), user(2 merged tool_result)
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][0]["type"], "tool_use");
        assert_eq!(messages[1]["content"][0]["input"]["path"], "a.rs");
        assert_eq!(messages[2]["role"], "user");
        let results = messages[2]["content"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["type"], "tool_result");
        assert_eq!(results[0]["tool_use_id"], "c1");
        assert_eq!(results[1]["tool_use_id"], "c2");
    }

    #[test]
    fn translates_vision_image_data_url() {
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [{ "role": "user", "content": [
                { "type": "text", "text": "look" },
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } },
            ]}],
        });
        let out = to_anthropic_request(&openai);
        let blocks = out["messages"][0]["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[1]["type"], "image");
        assert_eq!(blocks[1]["source"]["type"], "base64");
        assert_eq!(blocks[1]["source"]["media_type"], "image/png");
        assert_eq!(blocks[1]["source"]["data"], "AAAA");
    }

    #[test]
    fn parses_non_stream_response_with_tool_use() {
        let body = json!({
            "type": "message",
            "role": "assistant",
            "content": [
                { "type": "text", "text": "Reading now." },
                { "type": "tool_use", "id": "tu1", "name": "Read", "input": { "path": "a.rs" } },
            ],
            "stop_reason": "tool_use",
            "usage": { "input_tokens": 12, "output_tokens": 5 },
        });
        let out = from_anthropic_response(&body);
        let message = &out["choices"][0]["message"];
        assert_eq!(message["content"], "Reading now.");
        assert_eq!(message["tool_calls"][0]["id"], "tu1");
        assert_eq!(message["tool_calls"][0]["function"]["name"], "Read");
        assert_eq!(
            message["tool_calls"][0]["function"]["arguments"],
            "{\"path\":\"a.rs\"}"
        );
        assert_eq!(out["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(out["usage"]["prompt_tokens"], 12);
        assert_eq!(out["usage"]["completion_tokens"], 5);
        assert_eq!(out["usage"]["total_tokens"], 17);
    }
}
