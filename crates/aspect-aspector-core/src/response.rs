use crate::types::{ParsedAssistant, ParsedToolCall};

pub fn is_empty_message_content(content: &serde_json::Value) -> bool {
    match content {
        serde_json::Value::String(s) => s.trim().is_empty(),
        serde_json::Value::Array(arr) => arr.iter().all(|part| {
            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                text.trim().is_empty()
            } else {
                false
            }
        }),
        _ => true,
    }
}

pub fn parse_assistant_message(body: &serde_json::Value) -> ParsedAssistant {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();

    if let Some(content_val) = body.get("content") {
        match content_val {
            serde_json::Value::String(s) => {
                content = s.clone();
            }
            serde_json::Value::Array(parts) => {
                for part in parts {
                    let part_type = part
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("");
                    match part_type {
                        "text" => {
                            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                content.push_str(text);
                            }
                        }
                        "reasoning" | "thinking" => {
                            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                if !reasoning.is_empty() {
                                    reasoning.push('\n');
                                }
                                reasoning.push_str(text);
                            }
                        }
                        "tool_use" | "tool-call" | "function" => {
                            if let Some(tc) = parse_tool_call(part) {
                                tool_calls.push(tc);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(tool_calls_val) = body.get("tool_calls") {
        if let Some(calls) = tool_calls_val.as_array() {
            for call in calls {
                if let Some(tc) = parse_tool_call(call) {
                    tool_calls.push(tc);
                }
            }
        }
    }

    if let Some(reasoning_val) = body.get("reasoning") {
        if let Some(text) = reasoning_val.as_str() {
            if !reasoning.is_empty() {
                reasoning.push('\n');
            }
            reasoning.push_str(text);
        }
    }

    ParsedAssistant {
        content,
        reasoning,
        tool_calls,
    }
}

pub fn parse_tool_call(value: &serde_json::Value) -> Option<ParsedToolCall> {
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("name").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .or_else(|| {
            value
                .get("function")
                .and_then(|f| f.get("name").and_then(|n| n.as_str()))
        })
        .unwrap_or("")
        .to_string();

    let args = value
        .get("input")
        .or_else(|| value.get("arguments"))
        .or_else(|| {
            value
                .get("function")
                .and_then(|f| f.get("arguments"))
        })
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    if name.is_empty() {
        return None;
    }

    Some(ParsedToolCall { id, name, args })
}
