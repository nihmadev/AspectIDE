use serde_json::{json, Value};

use crate::types::{McpToolInfo, MAX_RESULT_CHARS};

pub fn parse_tools(result: &Value) -> Vec<McpToolInfo> {
    result
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| {
                    let name = tool.get("name").and_then(Value::as_str)?.to_string();
                    Some(McpToolInfo {
                        name,
                        description: tool
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        input_schema: tool
                            .get("inputSchema")
                            .cloned()
                            .unwrap_or_else(|| json!({ "type": "object" })),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn flatten_tool_result(result: &Value) -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for part in content {
            if part.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    lines.push(text.to_string());
                }
            } else {
                lines.push(part.to_string());
            }
        }
    }
    if let Some(structured) = result.get("structuredContent") {
        if !structured.is_null() {
            lines.push(format!("[structuredContent] {structured}"));
        }
    }

    let mut out = if lines.is_empty() {
        result.to_string()
    } else {
        lines.join("\n")
    };

    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        out = format!("[tool error] {out}");
    }

    clamp_result_body(out)
}

fn clamp_result_body(body: String) -> String {
    let total = body.chars().count();
    if total <= MAX_RESULT_CHARS {
        return body;
    }
    let head: String = body.chars().take(MAX_RESULT_CHARS).collect();
    let kept = match head.rfind('\n') {
        Some(newline) if newline > 0 => &head[..newline],
        _ => head.as_str(),
    };
    format!(
        "{kept}\n[truncated: output exceeded {MAX_RESULT_CHARS} chars ({total} total); \
         re-run the tool with narrower arguments to see the rest.]"
    )
}

