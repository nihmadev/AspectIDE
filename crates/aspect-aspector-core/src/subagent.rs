pub const MAX_SUBAGENT_SUMMARY_CHARS: usize = 12_000;

const MAIN_AGENT_ONLY_TOOLS: &[&str] = &[
    "AgentMessage", "PresentPlan", "AskUser", "Goal", "TodoWrite", "McpManage",
];

pub fn subagent_agent_id(subagent_type: &str) -> String {
    match subagent_type {
        "code" | "full" | "architect" | "ask" | "plan" => format!("{subagent_type}/subagent"),
        _ => format!("{subagent_type}/subagent"),
    }
}

pub fn clamp_subagent_summary(summary: String) -> String {
    if summary.len() > MAX_SUBAGENT_SUMMARY_CHARS {
        let mut truncated = summary;
        truncated.truncate(MAX_SUBAGENT_SUMMARY_CHARS);
        truncated.push_str("\n\n[summary truncated]");
        truncated
    } else {
        summary
    }
}

pub fn is_main_agent_only_tool(name: &str) -> bool {
    MAIN_AGENT_ONLY_TOOLS.contains(&name)
}
