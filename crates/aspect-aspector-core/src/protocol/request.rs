use serde_json::{json, Map, Value};

use super::tools;

/// Anthropic requires `max_tokens`; the `OpenAI` turn payload usually omits it.
const DEFAULT_MAX_TOKENS: u64 = 8192;

/// Output-token ceiling for a Claude model id when the caller didn't set an
/// explicit `max_tokens`/`max_completion_tokens`.
fn default_max_tokens_for_model(model: &str) -> u64 {
    if model.starts_with("claude-3-5") {
        DEFAULT_MAX_TOKENS
    } else if model.starts_with("claude-sonnet-4")
        || model.starts_with("claude-opus-4")
        || model.contains("-4-")
    {
        32_000
    } else {
        DEFAULT_MAX_TOKENS
    }
}

/// Extended-thinking token budget for a reasoning-effort level, mirroring the
/// low/medium/high tiers the rest of the stack already uses for reasoning models.
fn thinking_budget_for_effort(effort: &str) -> Option<u64> {
    match effort {
        "low" => Some(4096),
        "medium" => Some(8192),
        "high" => Some(16384),
        _ => None,
    }
}

/// Read the reasoning-effort string folded into the outgoing
/// payload, tolerating both shapes: a top-level `reasoning_effort`
/// string, or a `reasoning: { effort }` object.
fn reasoning_effort(openai: &Value) -> Option<&str> {
    openai
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .or_else(|| openai.pointer("/reasoning/effort").and_then(Value::as_str))
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
                _ => {
                    if let Some(message) = convert_user(msg) {
                        messages.push(message);
                    }
                }
            }
        }
    }
    let mut messages = coalesce_messages(messages);
    apply_conversation_cache_breakpoints(&mut messages);
    out.insert("messages".to_string(), Value::Array(messages));
    if !system_blocks.is_empty() {
        out.insert("system".to_string(), Value::Array(system_blocks));
    }

    let model = openai.get("model").and_then(Value::as_str).unwrap_or("");
    let mut max_tokens = openai
        .get("max_tokens")
        .or_else(|| openai.get("max_completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| default_max_tokens_for_model(model));

    let thinking_budget = reasoning_effort(openai)
        .and_then(thinking_budget_for_effort)
        .filter(|_| !tools::tool_choice_forces_tool_use(openai.get("tool_choice")))
        .filter(|_| max_tokens > 1024)
        .map(|budget| budget.min(max_tokens.saturating_sub(1024)))
        .filter(|budget| *budget >= 1024);
    if let Some(budget_tokens) = thinking_budget {
        out.insert(
            "thinking".to_string(),
            json!({ "type": "enabled", "budget_tokens": budget_tokens }),
        );
        max_tokens = max_tokens.max(budget_tokens + 1024);
    }
    out.insert("max_tokens".to_string(), json!(max_tokens));

    if thinking_budget.is_none() {
        if let Some(temperature) = openai.get("temperature") {
            out.insert("temperature".to_string(), temperature.clone());
        }
        if let Some(top_p) = openai.get("top_p") {
            out.insert("top_p".to_string(), top_p.clone());
        }
    }

    let tool_choice = openai.get("tool_choice");
    if let Some(tools) = openai.get("tools").and_then(Value::as_array) {
        let converted: Vec<Value> = tools.iter().filter_map(tools::convert_tool).collect();
        if !converted.is_empty() {
            out.insert("tools".to_string(), Value::Array(converted));
            let disable_parallel =
                openai.get("parallel_tool_calls").and_then(Value::as_bool) == Some(false);
            out.insert(
                "tool_choice".to_string(),
                tools::convert_tool_choice(tool_choice, disable_parallel),
            );
        }
    }

    if let Some(stream) = openai.get("stream") {
        out.insert("stream".to_string(), stream.clone());
    }

    Value::Object(out)
}

// ── Message translation helpers ──

/// Collect `OpenAI` system content into Anthropic system text blocks.
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

/// Convert an `OpenAI` user message into an Anthropic one.
fn convert_user(msg: &Value) -> Option<Value> {
    let blocks = content_blocks(msg.get("content"));
    if blocks.is_empty() {
        return None;
    }
    Some(json!({ "role": "user", "content": Value::Array(blocks) }))
}

/// Convert an assistant message (text + `tool_calls`) into an Anthropic assistant
/// message with text and `tool_use` blocks.
fn convert_assistant(msg: &Value) -> Option<Value> {
    if let Some(anthropic_blocks) = msg.get("anthropic_content").and_then(Value::as_array) {
        if let Some(reconstructed) = convert_assistant_from_anthropic_content(anthropic_blocks) {
            return Some(reconstructed);
        }
    }

    let mut blocks: Vec<Value> = Vec::new();
    match msg.get("content") {
        Some(Value::String(text)) if !text.is_empty() => {
            blocks.push(json!({ "type": "text", "text": text }));
        }
        Some(Value::Array(parts)) => {
            for part in parts {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        blocks.push(json!({ "type": "text", "text": text }));
                    }
                }
            }
        }
        _ => {}
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

/// Reconstruct an assistant message's Anthropic content blocks from the
/// `anthropic_content` vendor field.
fn convert_assistant_from_anthropic_content(blocks: &[Value]) -> Option<Value> {
    let mut thinking_blocks: Vec<Value> = Vec::new();
    let mut text_blocks: Vec<Value> = Vec::new();
    let mut tool_blocks: Vec<Value> = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("thinking") => {
                let thinking = block.get("thinking").and_then(Value::as_str).unwrap_or("");
                let signature = block.get("signature").and_then(Value::as_str).unwrap_or("");
                if !thinking.is_empty() && !signature.is_empty() {
                    thinking_blocks.push(json!({
                        "type": "thinking", "thinking": thinking, "signature": signature,
                    }));
                }
            }
            Some("redacted_thinking") => {
                if let Some(data) = block.get("data").and_then(Value::as_str) {
                    if !data.is_empty() {
                        thinking_blocks.push(json!({ "type": "redacted_thinking", "data": data }));
                    }
                }
            }
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        text_blocks.push(json!({ "type": "text", "text": text }));
                    }
                }
            }
            Some("tool_use") => {
                let id = block.get("id").and_then(Value::as_str).unwrap_or("");
                let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                tool_blocks
                    .push(json!({ "type": "tool_use", "id": id, "name": name, "input": input }));
            }
            _ => {}
        }
    }
    let mut out = Vec::with_capacity(thinking_blocks.len() + text_blocks.len() + tool_blocks.len());
    out.extend(thinking_blocks);
    out.extend(text_blocks);
    out.extend(tool_blocks);
    if out.is_empty() {
        return None;
    }
    Some(json!({ "role": "assistant", "content": Value::Array(out) }))
}

/// Convert an `OpenAI` `tool` result message into an Anthropic user message carrying a
/// `tool_result` block.
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
    let is_error = serde_json::from_str::<Value>(&content)
        .ok()
        .is_some_and(|value| value.get("error").is_some());
    let mut block = json!({ "type": "tool_result", "tool_use_id": id, "content": content });
    if is_error {
        block["is_error"] = Value::Bool(true);
    }
    json!({ "role": "user", "content": [block] })
}

/// Translate `OpenAI` message content into Anthropic content blocks.
fn content_blocks(content: Option<&Value>) -> Vec<Value> {
    match content {
        Some(Value::String(text)) if !text.is_empty() => {
            vec![json!({ "type": "text", "text": text })]
        }
        Some(Value::Array(parts)) => parts.iter().filter_map(convert_content_part).collect(),
        _ => vec![],
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

/// Number of trailing messages that receive a rolling `cache_control` breakpoint.
const CONVERSATION_CACHE_BREAKPOINTS: usize = 2;

fn block_accepts_cache_control(block: &Value) -> bool {
    matches!(
        block.get("type").and_then(Value::as_str),
        Some("text" | "image" | "tool_use" | "tool_result")
    )
}

/// Attach rolling `cache_control: ephemeral` breakpoints to the last cacheable
/// content block of the final [`CONVERSATION_CACHE_BREAKPOINTS`] messages.
fn apply_conversation_cache_breakpoints(messages: &mut [Value]) {
    for message in messages
        .iter_mut()
        .rev()
        .take(CONVERSATION_CACHE_BREAKPOINTS)
    {
        let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        if let Some(block) = blocks
            .iter_mut()
            .rev()
            .find(|block| block_accepts_cache_control(block))
        {
            if block.get("cache_control").is_none() {
                block["cache_control"] = json!({ "type": "ephemeral" });
            }
        }
    }
}

/// Merge adjacent same-role messages by concatenating their content blocks.
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
