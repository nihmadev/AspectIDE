use serde_json::Value;

use super::inline_think::{find_tag, is_tag_prefix, partial_tag_tail, prefix_tag, THINK_CLOSE_TAGS, THINK_OPEN_TAGS};
use super::sse::sse_stream_error;
use super::stream_types::{extract_reasoning_field, StreamToolCall};

/// Assembles streamed SSE delta chunks into a single OpenAI-style response body.
#[derive(Default)]
pub struct StreamAccumulator {
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<StreamToolCall>,
    pub usage: Option<Value>,
    pub finish_reason: Option<String>,
    pub in_think: bool,
    pub think_resolved: bool,
    pub think_carry: String,
    pub stream_error: Option<String>,
}

impl StreamAccumulator {
    pub fn ingest<F: FnMut(&str, &str), T: FnMut(&str)>(
        &mut self,
        value: &Value,
        on_delta: &mut F,
        on_tool_start: &mut T,
    ) {
        if self.stream_error.is_none() {
            if let Some(message) = sse_stream_error(value) {
                self.stream_error = Some(message);
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
        let explicit_reasoning = extract_reasoning_field(delta);
        if !explicit_reasoning.is_empty() {
            self.think_resolved = true;
        }

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
                self.merge_tool_call(call, on_tool_start);
            }
        }
    }

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
            let lead_ws = self.think_carry.len() - self.think_carry.trim_start().len();
            let rest = &self.think_carry[lead_ws..];
            if rest.is_empty() {
                break;
            }
            if let Some(len) = prefix_tag(rest, &THINK_OPEN_TAGS) {
                self.think_carry.drain(..lead_ws + len);
                self.in_think = true;
                continue;
            }
            if is_tag_prefix(rest, &THINK_OPEN_TAGS) {
                break;
            }
            self.think_resolved = true;
        }
        (out_content, out_reasoning)
    }

    /// Flush any buffered partial tag at end of stream.
    pub fn flush<F: FnMut(&str, &str)>(&mut self, on_delta: &mut F) {
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

    fn merge_tool_call<T: FnMut(&str)>(&mut self, call: &Value, on_tool_start: &mut T) {
        const MAX_TOOL_CALLS: usize = 256;
        let index = match call.get("index").and_then(Value::as_u64) {
            Some(value) => usize::try_from(value)
                .unwrap_or(usize::MAX)
                .min(MAX_TOOL_CALLS - 1),
            None => self.tool_calls.len().saturating_sub(1),
        };
        let is_new = index >= self.tool_calls.len();
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
        if is_new {
            on_tool_start(&slot.name);
        }
    }

    pub fn into_response_body(self) -> Value {
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
