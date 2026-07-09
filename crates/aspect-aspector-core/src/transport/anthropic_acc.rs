use serde_json::Value;

use super::stream_types::StreamToolCall;

/// One position in the Anthropic response's ordered content-block list.
#[derive(Default, Clone)]
pub enum PendingBlock {
    #[default]
    Empty,
    Thinking {
        thinking: String,
        signature: String,
    },
    RedactedThinking {
        data: String,
    },
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: String,
    },
}

/// Reassembles an Anthropic Messages SSE stream into OpenAI-style response body.
#[derive(Default)]
pub struct AnthropicStreamAccumulator {
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<StreamToolCall>,
    pub content_blocks: Vec<PendingBlock>,
    pub stop_reason: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read: u64,
    pub cache_creation: u64,
    pub error: Option<String>,
}

impl AnthropicStreamAccumulator {
    pub fn ingest<F: FnMut(&str, &str), T: FnMut(&str)>(
        &mut self,
        value: &Value,
        on_delta: &mut F,
        on_tool_start: &mut T,
    ) {
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
                let index = block_index(value);
                match block.and_then(|b| b.get("type")).and_then(Value::as_str) {
                    Some("tool_use") => {
                        self.ensure_slot(index);
                        let (id, name, arguments) = {
                            let slot = &mut self.tool_calls[index];
                            if let Some(id_val) = block.and_then(|b| b.get("id")).and_then(Value::as_str) {
                                slot.id = id_val.to_string();
                            }
                            if let Some(name_val) = block.and_then(|b| b.get("name")).and_then(Value::as_str) {
                                slot.name = name_val.to_string();
                            }
                            if let Some(input) = block.and_then(|b| b.get("input")) {
                                if input.as_object().is_some_and(|object| !object.is_empty()) {
                                    slot.arguments = input.to_string();
                                }
                            }
                            (slot.id.clone(), slot.name.clone(), slot.arguments.clone())
                        };
                        on_tool_start(&name);
                        self.ensure_block_slot(index);
                        self.content_blocks[index] = PendingBlock::ToolUse { id, name, input: arguments };
                    }
                    Some("thinking") => {
                        self.ensure_block_slot(index);
                        self.content_blocks[index] = PendingBlock::Thinking {
                            thinking: String::new(),
                            signature: String::new(),
                        };
                    }
                    Some("redacted_thinking") => {
                        let data = block
                            .and_then(|b| b.get("data"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        self.ensure_block_slot(index);
                        self.content_blocks[index] = PendingBlock::RedactedThinking { data };
                    }
                    Some("text") => {
                        self.ensure_block_slot(index);
                        self.content_blocks[index] = PendingBlock::Text { text: String::new() };
                    }
                    _ => {}
                }
            }
            "content_block_delta" => {
                let delta = value.get("delta");
                let index = block_index(value);
                match delta.and_then(|d| d.get("type")).and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(text) = delta.and_then(|d| d.get("text")).and_then(Value::as_str) {
                            self.content.push_str(text);
                            on_delta(text, "");
                            self.ensure_block_slot(index);
                            if let PendingBlock::Text { text: block_text } = &mut self.content_blocks[index] {
                                block_text.push_str(text);
                            }
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = delta.and_then(|d| d.get("thinking")).and_then(Value::as_str) {
                            self.reasoning.push_str(text);
                            on_delta("", text);
                            self.ensure_block_slot(index);
                            if let PendingBlock::Thinking { thinking, .. } = &mut self.content_blocks[index] {
                                thinking.push_str(text);
                            }
                        }
                    }
                    Some("signature_delta") => {
                        if let Some(sig) = delta.and_then(|d| d.get("signature")).and_then(Value::as_str) {
                            self.ensure_block_slot(index);
                            if let PendingBlock::Thinking { signature, .. } = &mut self.content_blocks[index] {
                                signature.push_str(sig);
                            }
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(fragment) = delta.and_then(|d| d.get("partial_json")).and_then(Value::as_str) {
                            self.ensure_slot(index);
                            self.tool_calls[index].arguments.push_str(fragment);
                            self.ensure_block_slot(index);
                            if let PendingBlock::ToolUse { input, .. } = &mut self.content_blocks[index] {
                                input.push_str(fragment);
                            }
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(reason) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = Some(reason.to_string());
                }
                if let Some(output) = value.pointer("/usage/output_tokens").and_then(Value::as_u64) {
                    self.output_tokens += output;
                }
            }
            "error" if self.error.is_none() => {
                let detail = value.pointer("/error/message").and_then(Value::as_str);
                let kind = value.pointer("/error/type").and_then(Value::as_str);
                self.error = Some(match (kind, detail) {
                    (Some(kind), Some(detail)) => format!("AI provider stream error ({kind}): {detail}"),
                    (Some(kind), None) => format!("AI provider stream error: {kind}"),
                    (None, Some(detail)) => format!("AI provider stream error: {detail}"),
                    (None, None) => "AI provider stream error".to_string(),
                });
            }
            _ => {}
        }
    }

    fn ensure_slot(&mut self, index: usize) {
        const MAX_TOOL_CALLS: usize = 256;
        let index = index.min(MAX_TOOL_CALLS - 1);
        while self.tool_calls.len() <= index {
            self.tool_calls.push(StreamToolCall::default());
        }
    }

    fn ensure_block_slot(&mut self, index: usize) {
        const MAX_BLOCKS: usize = 256;
        let index = index.min(MAX_BLOCKS - 1);
        while self.content_blocks.len() <= index {
            self.content_blocks.push(PendingBlock::Empty);
        }
    }

    pub fn into_response_body(self) -> Value {
        use crate::protocol;

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
        let anthropic_content = anthropic_content_blocks(&self.content_blocks);
        if !anthropic_content.is_empty() {
            message["anthropic_content"] = Value::Array(anthropic_content);
        }
        let mut body = serde_json::json!({
            "choices": [{
                "index": 0,
                "message": message,
                "finish_reason": protocol::finish_reason(self.stop_reason.as_deref(), has_tools),
            }],
            "usage": protocol::anthropic_usage(self.input_tokens, self.output_tokens, Some(&serde_json::json!({
                "cache_read_input_tokens": self.cache_read,
                "cache_creation_input_tokens": self.cache_creation,
            }))),
        });
        if protocol::stop_reason_needs_marker(self.stop_reason.as_deref()) {
            body["anthropic_stop_reason"] = serde_json::json!(self.stop_reason.unwrap_or_default());
        }
        body
    }
}

/// Render the ordered Anthropic content blocks into the exact block shapes.
fn anthropic_content_blocks(blocks: &[PendingBlock]) -> Vec<Value> {
    blocks
        .iter()
        .filter_map(|block| match block {
            PendingBlock::Empty => None,
            PendingBlock::Thinking { thinking, signature } => {
                if thinking.is_empty() || signature.is_empty() {
                    None
                } else {
                    Some(serde_json::json!({
                        "type": "thinking",
                        "thinking": thinking,
                        "signature": signature,
                    }))
                }
            }
            PendingBlock::RedactedThinking { data } => {
                if data.is_empty() { None } else { Some(serde_json::json!({ "type": "redacted_thinking", "data": data })) }
            }
            PendingBlock::Text { text } => {
                if text.is_empty() { None } else { Some(serde_json::json!({ "type": "text", "text": text })) }
            }
            PendingBlock::ToolUse { id, name, input } => {
                if id.is_empty() && name.is_empty() {
                    None
                } else {
                    let parsed = serde_json::from_str::<Value>(input)
                        .unwrap_or_else(|_| serde_json::json!({}));
                    Some(serde_json::json!({ "type": "tool_use", "id": id, "name": name, "input": parsed }))
                }
            }
        })
        .collect()
}

/// `index` field of an Anthropic content-block event, defaulting to 0.
fn block_index(value: &Value) -> usize {
    value
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(0)
}
