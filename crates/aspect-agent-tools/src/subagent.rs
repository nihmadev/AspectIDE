pub fn subagent_agent_id(subagent_type: &str) -> String {
    let uuid = uuid::Uuid::new_v4().simple().to_string();
    let short = &uuid[..8];
    let safe_type: String = subagent_type
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(24)
        .collect();
    if safe_type.is_empty() {
        format!("agent-{short}")
    } else {
        format!("{safe_type}-{short}")
    }
}

pub const MAX_SUBAGENT_SUMMARY_CHARS: usize = 12_000;

pub fn clamp_subagent_summary(summary: String) -> String {
    let total = summary.chars().count();
    if total <= MAX_SUBAGENT_SUMMARY_CHARS {
        return summary;
    }
    let truncated: String = summary.chars().take(MAX_SUBAGENT_SUMMARY_CHARS).collect();
    format!(
        "{truncated}\n\n[Subagent summary truncated: {total} chars total, showing first {MAX_SUBAGENT_SUMMARY_CHARS}.]"
    )
}
