use serde_json::{json, Value};

use crate::types::{McpToolInfo, MAX_TOOL_NAME_LEN};
use crate::registry;

pub async fn agent_tool_definitions() -> Vec<Value> {
    let registry_guard = registry().lock().await;
    let mut defs = Vec::new();
    for connection in registry_guard.values() {
        if connection.state() != "connected" {
            continue;
        }
        for tool in connection.tools() {
            let plain = format!("mcp__{}__{}", connection.config().id, tool.name);
            let (namespaced, renamed) = if is_provider_safe_tool_name(&plain) {
                (plain, false)
            } else {
                let alias = format!(
                    "mcp__{}__{}",
                    connection.config().id,
                    sanitize_tool_segment(&connection.config().id, &tool.name)
                );
                if !is_provider_safe_tool_name(&alias) {
                    tracing::warn!(
                        server = %connection.config().id,
                        tool = %tool.name,
                        "MCP tool name cannot be made provider-safe even after sanitizing; omitting"
                    );
                    continue;
                }
                tracing::info!(
                    server = %connection.config().id,
                    tool = %tool.name,
                    alias = %alias,
                    "exposing MCP tool under a sanitized alias (original name is not provider-safe)"
                );
                (alias, true)
            };
            let base_description = if tool.description.is_empty() {
                format!(
                    "MCP tool '{}' from server '{}'.",
                    tool.name, connection.config().name
                )
            } else {
                format!("[{}] {}", connection.config().name, tool.description)
            };
            let description = if renamed {
                format!(
                    "{base_description} (Exposed under a sanitized name; the server's real tool \
                     name is '{}'.)",
                    tool.name
                )
            } else {
                base_description
            };
            defs.push(json!({
                "type": "function",
                "function": {
                    "name": namespaced,
                    "description": description,
                    "parameters": tool.input_schema,
                },
            }));
        }
    }
    defs
}

pub fn is_provider_safe_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_TOOL_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn tool_name_tag(real: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in real.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:06x}", hash & 0xff_ffff)
}

pub fn sanitize_tool_segment(server_id: &str, real: &str) -> String {
    let mut cleaned = String::with_capacity(real.len());
    let mut prev_underscore = false;
    for ch in real.chars() {
        let mapped = if ch.is_ascii_alphanumeric() || ch == '-' {
            prev_underscore = false;
            ch
        } else {
            if prev_underscore {
                continue;
            }
            prev_underscore = true;
            '_'
        };
        cleaned.push(mapped);
    }
    let cleaned = cleaned.trim_matches('_');
    let stem = if cleaned.is_empty() { "tool" } else { cleaned };

    let tag = tool_name_tag(real);
    let fixed = "mcp__".len() + server_id.len() + "__".len() + 1 + tag.len();
    let stem_budget = MAX_TOOL_NAME_LEN.saturating_sub(fixed);
    let clipped_stem: String = stem.chars().take(stem_budget).collect();
    let clipped_stem = clipped_stem.trim_end_matches('_');
    if clipped_stem.is_empty() {
        tag
    } else {
        format!("{clipped_stem}_{tag}")
    }
}

pub fn resolve_tool_name(server_id: &str, tools: &[McpToolInfo], requested: &str) -> String {
    if tools.iter().any(|tool| tool.name == requested) {
        return requested.to_string();
    }
    for tool in tools {
        if sanitize_tool_segment(server_id, &tool.name) == requested {
            return tool.name.clone();
        }
    }
    requested.to_string()
}

