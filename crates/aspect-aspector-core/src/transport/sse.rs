use serde_json::Value;

/// Extract the payload from an SSE `data:` line event.
pub fn sse_event_data(event: &str) -> Option<String> {
    let lines = event
        .lines()
        .filter_map(|line| {
            let line = line.trim_end();
            if line.starts_with(':') {
                return None;
            }
            let data = line.strip_prefix("data:")?;
            Some(data.strip_prefix(' ').unwrap_or(data).to_string())
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// Normalize CR/LF/CRLF to LF in an SSE buffer.
pub fn normalize_sse_buffer_newlines(buffer: &mut String) {
    if buffer.contains('\r') {
        *buffer = buffer.replace("\r\n", "\n").replace('\r', "\n");
    }
}

/// Detect a mid-stream error frame on an already-200 SSE stream.
pub fn sse_stream_error(value: &Value) -> Option<String> {
    let msg = value
        .get("error")
        .and_then(|e| {
            e.get("message")
                .and_then(Value::as_str)
                .or_else(|| e.as_str())
        })
        .or_else(|| {
            if value.get("choices").is_none() {
                value.get("message").and_then(Value::as_str)
            } else {
                None
            }
        })?;
    let kind = value
        .get("error")
        .and_then(|e| e.get("code").or_else(|| e.get("type")))
        .and_then(Value::as_str);
    Some(match kind {
        Some(k) => format!("AI provider stream error ({k}): {msg}"),
        None => format!("AI provider stream error: {msg}"),
    })
}
