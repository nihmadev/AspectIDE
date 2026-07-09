use serde_json::{json, Value};

use super::finish_reason;

/// Map an Anthropic non-streaming Messages response back to an `OpenAI` completion
/// body (`choices[0].message` + `usage`), so callers parse it identically.
pub fn from_anthropic_response(body: &Value) -> Value {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_calls: Vec<Value> = Vec::new();
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
        "usage": super::anthropic_usage(input_tokens, output_tokens, usage),
    });
    if super::stop_reason_needs_marker(stop_reason) {
        out["anthropic_stop_reason"] = json!(stop_reason.unwrap_or_default());
    }
    out
}
