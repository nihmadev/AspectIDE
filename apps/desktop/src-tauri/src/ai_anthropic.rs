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
/// is the conservative fallback for older/unrecognized Claude models; see
/// `default_max_tokens_for_model` for the model-aware ceiling actually used.
const DEFAULT_MAX_TOKENS: u64 = 8192;

/// Output-token ceiling for a Claude model id when the caller didn't set an
/// explicit `max_tokens`/`max_completion_tokens`. `DEFAULT_MAX_TOKENS` (8192)
/// previously applied to every model, which truncates a large tool call (e.g.
/// writing a whole file) mid-JSON on the Sonnet/Opus 4.x family, which allows a
/// much larger output window.
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

/// Read the reasoning-effort string `merge_reasoning` folded into the outgoing
/// payload, tolerating both shapes it inserts: a top-level `reasoning_effort`
/// string, or a `reasoning: { effort }` object.
fn reasoning_effort(openai: &Value) -> Option<&str> {
    openai
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .or_else(|| openai.pointer("/reasoning/effort").and_then(Value::as_str))
}

/// Anthropic `stop_reason` values that must be surfaced to the caller even though
/// `finish_reason` normalizes them to `OpenAI`'s "stop" for model-agnostic
/// consumers. Currently just `refusal` (Anthropic's safety-refusal signal) —
/// silently normalizing it away would hide a distinction a caller may want to log
/// or act on.
pub fn stop_reason_needs_marker(stop_reason: Option<&str>) -> bool {
    stop_reason == Some("refusal")
}

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
                // An all-empty user message (e.g. a staged injection whose text was
                // blank) would otherwise become a `{"content":[]}` block Anthropic
                // rejects — `convert_user` already yields `None` for it; simply
                // don't push it rather than sending an empty content array.
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

    // max_tokens is required by Anthropic; honor an explicit cap, else the
    // model-aware default (a flat 8192 truncates a large tool call mid-JSON on
    // the Sonnet/Opus 4.x family, which allows a much larger output window).
    let model = openai.get("model").and_then(Value::as_str).unwrap_or("");
    let mut max_tokens = openai
        .get("max_tokens")
        .or_else(|| openai.get("max_completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| default_max_tokens_for_model(model));

    // Extended thinking: the frontend's selected reasoning effort (folded into the
    // payload by `merge_reasoning` as `reasoning_effort` / `reasoning.effort`) maps
    // to a thinking token budget. The API requires `max_tokens > budget_tokens` and
    // rejects an explicit `temperature`/`top_p` while thinking is enabled, so both
    // are enforced here instead of at every call site.
    // The API also rejects thinking combined with a FORCING tool_choice
    // ("required"/named tool → Anthropic "any"/"tool"); the native loop only ever
    // sends auto/none, but this translator serves arbitrary callers.
    let thinking_budget = reasoning_effort(openai)
        .and_then(thinking_budget_for_effort)
        .filter(|_| !tool_choice_forces_tool_use(openai.get("tool_choice")))
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

    // Tools + tool choice. `tool_choice: "none"` maps to Anthropic's
    // `{"type": "none"}` and the tools array is STILL sent: the Messages API
    // rejects any request whose messages contain `tool_use`/`tool_result` blocks
    // without a `tools` definition, and the recovery-synthesis call ("answer
    // without tools now") always carries prior tool rounds. Omitting the array
    // here used to 400 exactly those recovery calls. The caller's sequencing
    // constraint (`parallel_tool_calls: false`) is carried into Anthropic's
    // `disable_parallel_tool_use` flag on the choice (not valid on `none`).
    let tool_choice = openai.get("tool_choice");
    if let Some(tools) = openai.get("tools").and_then(Value::as_array) {
        let converted: Vec<Value> = tools.iter().filter_map(convert_tool).collect();
        if !converted.is_empty() {
            out.insert("tools".to_string(), Value::Array(converted));
            let disable_parallel =
                openai.get("parallel_tool_calls").and_then(Value::as_bool) == Some(false);
            out.insert(
                "tool_choice".to_string(),
                convert_tool_choice(tool_choice, disable_parallel),
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
    // The exact block sequence, preserved verbatim (including thinking signatures
    // and redacted_thinking blobs the flattened fields above don't carry) so a
    // later replay of this turn can reconstruct it — see `convert_assistant`.
    let mut anthropic_content: Vec<Value> = Vec::new();
    if let Some(blocks) = body.get("content").and_then(Value::as_array) {
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        content.push_str(text);
                        if !text.is_empty() {
                            anthropic_content.push(json!({ "type": "text", "text": text }));
                        }
                    }
                }
                Some("thinking") => {
                    let text = block.get("thinking").and_then(Value::as_str).unwrap_or("");
                    reasoning.push_str(text);
                    let signature = block.get("signature").and_then(Value::as_str).unwrap_or("");
                    // Drop a broken thinking block (missing text or signature) from the
                    // replay-able sequence rather than round-tripping a partial one.
                    if !text.is_empty() && !signature.is_empty() {
                        anthropic_content.push(json!({
                            "type": "thinking", "thinking": text, "signature": signature,
                        }));
                    }
                }
                Some("redacted_thinking") => {
                    if let Some(data) = block.get("data").and_then(Value::as_str) {
                        if !data.is_empty() {
                            anthropic_content
                                .push(json!({ "type": "redacted_thinking", "data": data }));
                        }
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
                    anthropic_content.push(
                        json!({ "type": "tool_use", "id": id, "name": name, "input": input }),
                    );
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
    if !anthropic_content.is_empty() {
        message["anthropic_content"] = Value::Array(anthropic_content);
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

    let stop_reason = body.get("stop_reason").and_then(Value::as_str);
    let mut out = json!({
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason(stop_reason, has_tools),
        }],
        "usage": anthropic_usage(input_tokens, output_tokens, usage),
    });
    if stop_reason_needs_marker(stop_reason) {
        out["anthropic_stop_reason"] = json!(stop_reason.unwrap_or_default());
    }
    out
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

/// Convert an `OpenAI` user message into an Anthropic one. Returns `None` for an
/// all-empty message (empty/absent string content) instead of producing an
/// invalid `{"content":[]}` block — the Messages API rejects empty content arrays.
fn convert_user(msg: &Value) -> Option<Value> {
    let blocks = content_blocks(msg.get("content"));
    if blocks.is_empty() {
        return None;
    }
    Some(json!({ "role": "user", "content": Value::Array(blocks) }))
}

/// Convert an assistant message (text + `tool_calls`) into an Anthropic assistant
/// message with text and `tool_use` blocks. Returns `None` for an empty message so
/// it is dropped instead of producing an invalid empty-content block.
fn convert_assistant(msg: &Value) -> Option<Value> {
    // Replayed history carries the exact original Anthropic block sequence — thinking
    // (with signature) / redacted_thinking, then text, then tool_use — in the
    // `anthropic_content` vendor field. Reconstructing FROM it (rather than the
    // flattened `content`/`tool_calls` fields) is what fixes the 400 "Expected
    // thinking or redacted_thinking block" error the Messages API raises when a
    // thinking-enabled turn that also used tools is replayed without its thinking
    // blocks reappearing first.
    if let Some(anthropic_blocks) = msg.get("anthropic_content").and_then(Value::as_array) {
        if let Some(reconstructed) = convert_assistant_from_anthropic_content(anthropic_blocks) {
            return Some(reconstructed);
        }
        // Every captured block was empty/broken (e.g. a thinking block that never
        // got a signature) — fall through to the plain text/tool_calls path so a
        // turn that still has real content isn't dropped outright.
    }

    let mut blocks: Vec<Value> = Vec::new();
    match msg.get("content") {
        // Plain string content → a single text block (the common case).
        Some(Value::String(text)) if !text.is_empty() => {
            blocks.push(json!({ "type": "text", "text": text }));
        }
        // OpenAI content-part array (e.g. replayed transcript history): preserve text
        // blocks via the same conversion path as user messages instead of dropping
        // the whole message when there are no tool_calls.
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
/// `anthropic_content` vendor field, restoring the exact block sequence the API
/// requires: `thinking`/`redacted_thinking` blocks first, then `text`, then `tool_use`.
/// A `thinking` block missing its text or signature is dropped entirely — the API
/// rejects a broken/partial thinking block, whereas dropping ALL thinking blocks
/// for a turn is valid. Returns `None` when nothing survives the filter.
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
    // ai_turn.rs pushes a failed tool call as a JSON object string with a
    // top-level "error" key (`{"error": "..."}`). Surface that as Anthropic's
    // `is_error` flag on the tool_result block so the model is told the call
    // failed instead of reading the error text as an ordinary result.
    let is_error = serde_json::from_str::<Value>(&content)
        .ok()
        .is_some_and(|value| value.get("error").is_some());
    let mut block = json!({ "type": "tool_result", "tool_use_id": id, "content": content });
    if is_error {
        block["is_error"] = Value::Bool(true);
    }
    json!({ "role": "user", "content": [block] })
}

/// Translate `OpenAI` message content into Anthropic content blocks. A string becomes
/// one text block (dropped if empty); a content-part array maps text parts and
/// `image_url` parts (data-URL → base64 source, otherwise a URL source). Absent or
/// empty content yields an EMPTY vec rather than a synthetic empty text block — the
/// Messages API rejects an empty `content` array, so callers must skip the whole
/// message rather than send one.
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
/// Two (not one) so that when the newest message changes next round, the lookup
/// still hits the breakpoint that was on the previous round's tail — the whole
/// conversation prefix is then a cache READ instead of being re-billed at full
/// input price on every tool round. With the system-prompt breakpoint this uses
/// 3 of Anthropic's 4 allowed `cache_control` blocks.
const CONVERSATION_CACHE_BREAKPOINTS: usize = 2;

/// Block types `cache_control` may be attached to; a `thinking`/
/// `redacted_thinking` block rejects it, so the marker goes on the last
/// cacheable block of a message instead.
fn block_accepts_cache_control(block: &Value) -> bool {
    matches!(
        block.get("type").and_then(Value::as_str),
        Some("text" | "image" | "tool_use" | "tool_result")
    )
}

/// Attach rolling `cache_control: ephemeral` breakpoints to the last cacheable
/// content block of the final [`CONVERSATION_CACHE_BREAKPOINTS`] messages, so
/// each round's request caches the conversation built so far (see the constant
/// for why two). Existing caller-set markers are respected, not duplicated.
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
    // An explicit `"parameters": null` (some callers send this for a no-arg tool)
    // must fall back to the empty-object schema exactly like a missing field —
    // `.get()` returns `Some(Value::Null)` for it, which would otherwise pass
    // `null` straight through as `input_schema` and 400 the request.
    out["input_schema"] = function
        .get("parameters")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!({ "type": "object" }));
    Some(out)
}

/// Map an `OpenAI` `tool_choice` onto an Anthropic one. `"none"` becomes
/// `{"type": "none"}` (tools stay defined so prior `tool_use`/`tool_result`
/// blocks remain valid). `disable_parallel` (from `parallel_tool_calls: false`)
/// adds `disable_parallel_tool_use: true` so the caller's one-tool-per-turn
/// constraint survives the protocol hop — except on `none`, which accepts no
/// extra fields.
/// True when the OpenAI-shaped `tool_choice` would translate to Anthropic's
/// `any`/`tool` (forcing) shapes, which the API rejects alongside `thinking`.
fn tool_choice_forces_tool_use(choice: Option<&Value>) -> bool {
    match choice {
        Some(Value::String(value)) => value == "required" || value == "any",
        Some(Value::Object(object)) => object
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .is_some(),
        _ => false,
    }
}

fn convert_tool_choice(choice: Option<&Value>, disable_parallel: bool) -> Value {
    let mut out = match choice {
        Some(Value::String(value)) if value == "none" => {
            return json!({ "type": "none" });
        }
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
    };
    if disable_parallel {
        out["disable_parallel_tool_use"] = Value::Bool(true);
    }
    out
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
        // claude-sonnet-4-5 gets the model-aware 32000 default, not the flat
        // legacy 8192 ceiling (see `model_aware_max_tokens_table`).
        assert_eq!(out["max_tokens"], json!(32_000));
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
    fn tool_choice_none_keeps_tools_and_maps_to_none_type() {
        // Recovery-synthesis regression: `tool_choice: "none"` must keep the tools
        // array (messages with tool_use/tool_result blocks REQUIRE `tools`) and map
        // to Anthropic's `{"type": "none"}` — omitting them 400s every recovery call
        // after a tool round. `none` accepts no extra fields, so parallel_tool_calls
        // must not attach disable_parallel_tool_use here.
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [{ "role": "user", "content": "go" }],
            "tools": [{ "type": "function", "function": { "name": "Read", "parameters": {} } }],
            "tool_choice": "none",
            "parallel_tool_calls": false,
        });
        let out = to_anthropic_request(&openai);
        assert_eq!(
            out["tools"][0]["name"], "Read",
            "tools stay defined for none"
        );
        assert_eq!(out["tool_choice"]["type"], "none");
        assert!(
            out["tool_choice"]
                .get("disable_parallel_tool_use")
                .is_none(),
            "none accepts no extra fields"
        );
    }

    #[test]
    fn parallel_tool_calls_false_disables_parallel_use() {
        // `parallel_tool_calls: false` enforces one tool per turn; that constraint
        // must survive as Anthropic's `disable_parallel_tool_use`.
        let base = json!({
            "model": "claude-sonnet-4-5",
            "messages": [{ "role": "user", "content": "go" }],
            "tools": [{ "type": "function", "function": { "name": "Read", "parameters": {} } }],
            "parallel_tool_calls": false,
        });

        // auto (default) choice
        let out = to_anthropic_request(&base);
        assert_eq!(out["tool_choice"]["type"], "auto");
        assert_eq!(out["tool_choice"]["disable_parallel_tool_use"], json!(true));

        // required → any
        let mut required = base.clone();
        required["tool_choice"] = json!("required");
        let out = to_anthropic_request(&required);
        assert_eq!(out["tool_choice"]["type"], "any");
        assert_eq!(out["tool_choice"]["disable_parallel_tool_use"], json!(true));

        // named tool choice
        let mut named = base.clone();
        named["tool_choice"] = json!({ "type": "function", "function": { "name": "Read" } });
        let out = to_anthropic_request(&named);
        assert_eq!(out["tool_choice"]["type"], "tool");
        assert_eq!(out["tool_choice"]["name"], "Read");
        assert_eq!(out["tool_choice"]["disable_parallel_tool_use"], json!(true));

        // parallel_tool_calls: true (or absent) leaves the flag off.
        let mut parallel = base;
        parallel["parallel_tool_calls"] = json!(true);
        let out = to_anthropic_request(&parallel);
        assert!(out["tool_choice"]
            .get("disable_parallel_tool_use")
            .is_none());
    }

    #[test]
    fn assistant_content_array_is_preserved_without_tool_calls() {
        // Replayed transcript history can arrive as an OpenAI content-part array on
        // an assistant turn; its text must survive instead of the message dropping.
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                { "role": "user", "content": "hi" },
                { "role": "assistant", "content": [
                    { "type": "text", "text": "previous answer" },
                ]},
                { "role": "user", "content": "continue" },
            ],
        });
        let out = to_anthropic_request(&openai);
        let messages = out["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][0]["type"], "text");
        assert_eq!(messages[1]["content"][0]["text"], "previous answer");
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

    // ── Extended thinking (Cluster 2a/2d) ──

    #[test]
    fn thinking_effort_maps_to_budget_and_strips_temperature() {
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [{ "role": "user", "content": "go" }],
            "temperature": 0.7,
            "top_p": 0.9,
            "reasoning_effort": "high",
        });
        let out = to_anthropic_request(&openai);
        assert_eq!(out["thinking"]["type"], "enabled");
        assert_eq!(out["thinking"]["budget_tokens"], 16384);
        // Thinking requires temperature/top_p unset (API mandates default/1).
        assert!(out.get("temperature").is_none());
        assert!(out.get("top_p").is_none());
        // budget_tokens must stay below max_tokens.
        assert!(out["max_tokens"].as_u64().unwrap() > 16384);
    }

    #[test]
    fn thinking_effort_via_reasoning_object_shape() {
        // merge_reasoning also folds in a `reasoning: { effort }` object shape.
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [{ "role": "user", "content": "go" }],
            "reasoning": { "effort": "low" },
        });
        let out = to_anthropic_request(&openai);
        assert_eq!(out["thinking"]["budget_tokens"], 4096);
    }

    #[test]
    fn thinking_is_skipped_when_tool_choice_forces_tool_use() {
        // The API rejects thinking + tool_choice any/tool; the translator must
        // drop thinking (and keep temperature passthrough) for such callers.
        for choice in [
            json!("required"),
            json!({ "type": "function", "function": { "name": "Write" } }),
        ] {
            let openai = json!({
                "model": "claude-sonnet-4-5",
                "messages": [{ "role": "user", "content": "go" }],
                "reasoning_effort": "high",
                "temperature": 0.4,
                "tools": [{ "type": "function", "function": { "name": "Write", "parameters": { "type": "object" } } }],
                "tool_choice": choice,
            });
            let out = to_anthropic_request(&openai);
            assert!(out.get("thinking").is_none());
            assert_eq!(out["temperature"], 0.4);
        }
    }

    #[test]
    fn thinking_budget_medium_effort() {
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [{ "role": "user", "content": "go" }],
            "reasoning_effort": "medium",
        });
        let out = to_anthropic_request(&openai);
        assert_eq!(out["thinking"]["budget_tokens"], 8192);
    }

    #[test]
    fn thinking_budget_is_capped_below_an_explicit_small_max_tokens() {
        // An explicit small max_tokens must still leave the API-required headroom
        // (budget_tokens < max_tokens) rather than requesting an impossible budget.
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [{ "role": "user", "content": "go" }],
            "max_tokens": 2048,
            "reasoning_effort": "high",
        });
        let out = to_anthropic_request(&openai);
        let budget = out["thinking"]["budget_tokens"].as_u64().unwrap();
        let max_tokens = out["max_tokens"].as_u64().unwrap();
        assert!(budget < max_tokens);
        assert_eq!(budget, 1024, "capped to max_tokens(2048) - 1024");
    }

    #[test]
    fn no_reasoning_effort_leaves_temperature_and_thinking_absent() {
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [{ "role": "user", "content": "go" }],
            "temperature": 0.5,
        });
        let out = to_anthropic_request(&openai);
        assert!(out.get("thinking").is_none());
        assert_eq!(out["temperature"], 0.5);
    }

    // ── Model-aware max_tokens (Cluster 2d) ──

    #[test]
    fn model_aware_max_tokens_table() {
        assert_eq!(
            default_max_tokens_for_model("claude-3-5-sonnet-20241022"),
            8192
        );
        assert_eq!(
            default_max_tokens_for_model("claude-3-5-haiku-20241022"),
            8192
        );
        assert_eq!(default_max_tokens_for_model("claude-sonnet-4-5"), 32_000);
        assert_eq!(default_max_tokens_for_model("claude-opus-4-1"), 32_000);
        assert_eq!(default_max_tokens_for_model("claude-haiku-4-5"), 32_000);
        assert_eq!(default_max_tokens_for_model("claude-2.1"), 8192);
        assert_eq!(default_max_tokens_for_model("claude-instant-1.2"), 8192);
    }

    #[test]
    fn to_anthropic_request_uses_model_aware_default_max_tokens() {
        let sonnet4 = json!({
            "model": "claude-sonnet-4-5",
            "messages": [{ "role": "user", "content": "go" }],
        });
        assert_eq!(to_anthropic_request(&sonnet4)["max_tokens"], 32_000);

        let haiku35 = json!({
            "model": "claude-3-5-haiku-20241022",
            "messages": [{ "role": "user", "content": "go" }],
        });
        assert_eq!(to_anthropic_request(&haiku35)["max_tokens"], 8192);

        // An explicit max_tokens always wins over the model-aware default.
        let explicit = json!({
            "model": "claude-sonnet-4-5",
            "messages": [{ "role": "user", "content": "go" }],
            "max_tokens": 4096,
        });
        assert_eq!(to_anthropic_request(&explicit)["max_tokens"], 4096);
    }

    // ── anthropic_content replay reconstruction (Cluster 2c) ──

    #[test]
    fn convert_assistant_reconstructs_thinking_then_text_then_tool_use() {
        // Deliberately out-of-order in the source array — the API requires
        // thinking/redacted_thinking FIRST regardless of capture order.
        let msg = json!({
            "role": "assistant",
            "content": null,
            "anthropic_content": [
                { "type": "tool_use", "id": "tu1", "name": "Read", "input": { "path": "a.rs" } },
                { "type": "thinking", "thinking": "let me plan", "signature": "sig-1" },
                { "type": "text", "text": "Reading now." },
            ],
        });
        let out = convert_assistant(&msg).expect("message should convert");
        let blocks = out["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0]["type"], "thinking");
        assert_eq!(blocks[0]["thinking"], "let me plan");
        assert_eq!(blocks[0]["signature"], "sig-1");
        assert_eq!(blocks[1]["type"], "text");
        assert_eq!(blocks[1]["text"], "Reading now.");
        assert_eq!(blocks[2]["type"], "tool_use");
        assert_eq!(blocks[2]["id"], "tu1");
    }

    #[test]
    fn convert_assistant_preserves_redacted_thinking_before_tool_use() {
        let msg = json!({
            "role": "assistant",
            "content": null,
            "anthropic_content": [
                { "type": "redacted_thinking", "data": "opaque-blob" },
                { "type": "tool_use", "id": "tu1", "name": "Read", "input": {} },
            ],
        });
        let out = convert_assistant(&msg).expect("message should convert");
        let blocks = out["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "redacted_thinking");
        assert_eq!(blocks[0]["data"], "opaque-blob");
        assert_eq!(blocks[1]["type"], "tool_use");
    }

    #[test]
    fn convert_assistant_drops_thinking_block_missing_signature() {
        // Replaying a thinking block without its signature 400s — dropping it
        // (while keeping the rest of the turn) is the only valid recovery.
        let msg = json!({
            "role": "assistant",
            "content": null,
            "anthropic_content": [
                { "type": "thinking", "thinking": "no signature" },
                { "type": "text", "text": "answer" },
            ],
        });
        let out = convert_assistant(&msg).expect("message should convert");
        let blocks = out["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1, "the broken thinking block must be dropped");
        assert_eq!(blocks[0]["type"], "text");
    }

    #[test]
    fn convert_assistant_falls_back_when_anthropic_content_all_empty() {
        // Every captured block is broken/empty — fall back to the flattened
        // content/tool_calls fields instead of dropping the message outright.
        let msg = json!({
            "role": "assistant",
            "content": "plain answer",
            "anthropic_content": [
                { "type": "thinking", "thinking": "no signature" },
            ],
        });
        let out = convert_assistant(&msg).expect("message should convert via fallback");
        let blocks = out["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], "plain answer");
    }

    #[test]
    fn convert_assistant_without_anthropic_content_uses_legacy_path() {
        // Older history (or cross-provider) with no vendor field still works via
        // the pre-existing text + tool_calls conversion.
        let msg = json!({
            "role": "assistant",
            "content": "hello",
            "tool_calls": [
                { "id": "c1", "type": "function", "function": { "name": "Read", "arguments": "{}" } },
            ],
        });
        let out = convert_assistant(&msg).expect("message should convert");
        let blocks = out["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[1]["type"], "tool_use");
    }

    // ── Empty-content filtering (Cluster 2e) ──

    #[test]
    fn content_blocks_filters_empty_or_absent_text() {
        assert!(content_blocks(Some(&json!(""))).is_empty());
        assert!(content_blocks(None).is_empty());
        assert!(content_blocks(Some(&Value::Null)).is_empty());
        assert_eq!(content_blocks(Some(&json!("hi"))).len(), 1);
    }

    #[test]
    fn to_anthropic_request_drops_all_empty_user_message() {
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                { "role": "user", "content": "" },
                { "role": "user", "content": "real question" },
            ],
        });
        let out = to_anthropic_request(&openai);
        let messages = out["messages"].as_array().unwrap();
        // The empty message is dropped entirely rather than sent as `content: []`.
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["content"][0]["text"], "real question");
    }

    // ── convert_tool parameters:null (Cluster 2f) ──

    #[test]
    fn convert_tool_treats_null_parameters_like_missing() {
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [{ "role": "user", "content": "go" }],
            "tools": [{
                "type": "function",
                "function": { "name": "NoArgs", "parameters": Value::Null },
            }],
        });
        let out = to_anthropic_request(&openai);
        assert_eq!(out["tools"][0]["input_schema"], json!({ "type": "object" }));
    }

    // ── tool_result is_error propagation (Cluster 2g) ──

    #[test]
    fn convert_tool_result_sets_is_error_for_error_object_content() {
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                { "role": "user", "content": "go" },
                { "role": "assistant", "content": null, "tool_calls": [
                    { "id": "c1", "type": "function", "function": { "name": "Read", "arguments": "{}" } },
                ]},
                { "role": "tool", "tool_call_id": "c1", "content": "{\"error\":\"file not found\"}" },
            ],
        });
        let out = to_anthropic_request(&openai);
        let messages = out["messages"].as_array().unwrap();
        let tool_result = &messages[2]["content"][0];
        assert_eq!(tool_result["type"], "tool_result");
        assert_eq!(tool_result["is_error"], true);
    }

    #[test]
    fn convert_tool_result_omits_is_error_for_success_content() {
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                { "role": "user", "content": "go" },
                { "role": "assistant", "content": null, "tool_calls": [
                    { "id": "c1", "type": "function", "function": { "name": "Read", "arguments": "{}" } },
                ]},
                { "role": "tool", "tool_call_id": "c1", "content": "file contents here" },
            ],
        });
        let out = to_anthropic_request(&openai);
        let messages = out["messages"].as_array().unwrap();
        let tool_result = &messages[2]["content"][0];
        assert!(tool_result.get("is_error").is_none());
    }

    // ── finish_reason / anthropic_stop_reason marker (Cluster 2h) ──

    #[test]
    fn refusal_maps_to_stop_and_surfaces_marker() {
        assert_eq!(finish_reason(Some("refusal"), false), "stop");
        assert!(stop_reason_needs_marker(Some("refusal")));
        assert!(!stop_reason_needs_marker(Some("end_turn")));
        assert!(!stop_reason_needs_marker(None));

        let body = json!({
            "content": [{ "type": "text", "text": "I can't help with that." }],
            "stop_reason": "refusal",
        });
        let out = from_anthropic_response(&body);
        assert_eq!(out["choices"][0]["finish_reason"], "stop");
        assert_eq!(out["anthropic_stop_reason"], "refusal");
    }

    #[test]
    fn non_refusal_stop_reasons_carry_no_marker() {
        let body = json!({
            "content": [{ "type": "text", "text": "done" }],
            "stop_reason": "end_turn",
        });
        let out = from_anthropic_response(&body);
        assert!(out.get("anthropic_stop_reason").is_none());
    }

    // ── Conversation prompt caching (rolling breakpoints) ──

    #[test]
    fn rolling_cache_breakpoints_on_last_two_messages() {
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                { "role": "user", "content": "round 1" },
                { "role": "assistant", "content": "answer 1" },
                { "role": "user", "content": "round 2" },
            ],
        });
        let out = to_anthropic_request(&openai);
        let messages = out["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        // Oldest message: no marker (only the trailing two carry breakpoints).
        assert!(messages[0]["content"][0].get("cache_control").is_none());
        assert_eq!(
            messages[1]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert_eq!(
            messages[2]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn cache_breakpoint_lands_on_last_cacheable_block_of_each_message() {
        // A trailing tool round: assistant(thinking+tool_use) then user(tool_result).
        // thinking rejects cache_control → the assistant marker must land on the
        // tool_use block; the tool_result user message takes the second marker.
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                { "role": "user", "content": "go" },
                { "role": "assistant", "content": null,
                  "anthropic_content": [
                    { "type": "thinking", "thinking": "plan", "signature": "sig" },
                    { "type": "tool_use", "id": "c1", "name": "Read", "input": {} },
                  ]},
                { "role": "tool", "tool_call_id": "c1", "content": "file body" },
            ],
        });
        let out = to_anthropic_request(&openai);
        let messages = out["messages"].as_array().unwrap();
        let assistant = messages[1]["content"].as_array().unwrap();
        assert!(
            assistant[0].get("cache_control").is_none(),
            "not on thinking"
        );
        assert_eq!(assistant[1]["cache_control"]["type"], "ephemeral");
        assert_eq!(
            messages[2]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn cache_breakpoint_single_message_conversation() {
        let openai = json!({
            "model": "claude-sonnet-4-5",
            "messages": [{ "role": "user", "content": "hello" }],
        });
        let out = to_anthropic_request(&openai);
        assert_eq!(
            out["messages"][0]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    // ── non-stream anthropic_content assembly ──

    #[test]
    fn from_anthropic_response_assembles_ordered_anthropic_content() {
        let body = json!({
            "content": [
                { "type": "thinking", "thinking": "plan", "signature": "sig" },
                { "type": "text", "text": "Reading now." },
                { "type": "tool_use", "id": "tu1", "name": "Read", "input": { "path": "a.rs" } },
            ],
            "stop_reason": "tool_use",
            "usage": { "input_tokens": 1, "output_tokens": 1 },
        });
        let out = from_anthropic_response(&body);
        let anthropic_content = out["choices"][0]["message"]["anthropic_content"]
            .as_array()
            .expect("anthropic_content array");
        assert_eq!(anthropic_content.len(), 3);
        assert_eq!(anthropic_content[0]["type"], "thinking");
        assert_eq!(anthropic_content[0]["signature"], "sig");
        assert_eq!(anthropic_content[1]["type"], "text");
        assert_eq!(anthropic_content[2]["type"], "tool_use");
    }

    #[test]
    fn from_anthropic_response_drops_broken_thinking_from_anthropic_content() {
        let body = json!({
            "content": [
                { "type": "thinking", "thinking": "plan" },
                { "type": "text", "text": "answer" },
            ],
            "stop_reason": "end_turn",
        });
        let out = from_anthropic_response(&body);
        let anthropic_content = out["choices"][0]["message"]["anthropic_content"]
            .as_array()
            .expect("anthropic_content array");
        assert_eq!(anthropic_content.len(), 1);
        assert_eq!(anthropic_content[0]["type"], "text");
    }
}
