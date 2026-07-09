use serde_json::Value;

/// A tool call being accumulated from streamed SSE delta fragments.
#[derive(Default)]
pub struct StreamToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Pull a provider's explicitly-reported reasoning out of a streamed delta.
pub fn extract_reasoning_field(delta: &Value) -> String {
    for key in ["reasoning_content", "reasoning", "thinking"] {
        if let Some(text) = delta.get(key).and_then(Value::as_str) {
            if !text.is_empty() {
                return text.to_string();
            }
        }
    }
    if let Some(object) = delta.get("reasoning").and_then(Value::as_object) {
        for key in ["content", "text"] {
            if let Some(text) = object.get(key).and_then(Value::as_str) {
                if !text.is_empty() {
                    return text.to_string();
                }
            }
        }
    }
    String::new()
}
