use std::collections::HashSet;

use super::normalize::normalize_tool_name;
use super::aliases::harness_alias_hint;
use super::suggest::closest_tool_names;

pub fn tool_names_from_defs(tools: &[serde_json::Value]) -> HashSet<String> {
    tools
        .iter()
        .filter_map(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .or_else(|| t.get("name"))
                .and_then(|n| n.as_str())
        })
        .map(str::to_string)
        .collect()
}

pub fn tool_rejection_error(
    name: &str,
    agent_mode: &str,
    browser_enabled: bool,
    allowed: &HashSet<String>,
    universe: &HashSet<String>,
    mode_defs_with_browser: &HashSet<String>,
    mode_defs_no_browser: &HashSet<String>,
) -> String {
    if let Some((server, _)) = name
        .strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
    {
        return format!(
            "{name} is not attached to this request: MCP tools need Agent or Automatic mode and a connected `{server}` MCP server (see McpManage). Use the tools in this request's tools array instead."
        );
    }
    if universe.contains(name) {
        if mode_defs_with_browser.contains(name) {
            if !browser_enabled
                && !mode_defs_no_browser.contains(name)
            {
                return format!(
                    "{name} is blocked: browser automation is disabled in Aspect settings (Settings в†’ AI в†’ Browser). Do not retry Browser tools; continue without the browser or ask the user to enable it."
                );
            }
            return format!(
                "{name} is not available in this read-only context and was blocked by the tool allowlist. Do not retry it; finish with the read-only tools attached to this request."
            );
        }
        return format!(
            "{name} is not available in {agent_mode} mode and was blocked by the tool allowlist. It exists only in Agent/Automatic mode вЂ” do not retry it here; finish with the read-only tools attached to this request or ask the user to switch modes."
        );
    }
    let mut message = format!(
        "Unknown tool {name} вЂ” it does not exist in Aspect in ANY mode (that name comes from a different assistant's harness). "
    );
    if let Some(hint) = harness_alias_hint(&normalize_tool_name(name)) {
        message.push_str(hint);
    } else {
        let suggestions = closest_tool_names(name, allowed);
        if suggestions.is_empty() {
            message.push_str(
                "Pick a tool from this request's tools array вЂ” that list is complete and final.",
            );
        } else {
            use std::fmt::Write as _;
            let _ = write!(message, "Did you mean: {}?", suggestions.join(", "));
        }
    }
    message
}
