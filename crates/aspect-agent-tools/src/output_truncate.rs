const AI_SHELL_MAX_OUTPUT_CHARS: usize = 24_000;
const AI_SHELL_TRUNCATE_HEAD_CHARS: usize = 12_000;
const AI_SHELL_TRUNCATE_TAIL_CHARS: usize = 12_000;

pub fn truncate_shell_output(value: &str) -> String {
    truncate_shell_output_flagged(value).0
}

pub fn truncate_shell_output_flagged(value: &str) -> (String, bool) {
    let trimmed = value.trim();
    let total_chars = trimmed.chars().count();
    if total_chars <= AI_SHELL_MAX_OUTPUT_CHARS {
        return (trimmed.to_string(), false);
    }
    let widest_marker = format!("\n... [{total_chars} characters omitted] ...\n");
    let content_budget = AI_SHELL_MAX_OUTPUT_CHARS.saturating_sub(widest_marker.chars().count());
    let head_chars = AI_SHELL_TRUNCATE_HEAD_CHARS.min(content_budget);
    let tail_chars = AI_SHELL_TRUNCATE_TAIL_CHARS.min(content_budget - head_chars);
    let omitted = total_chars - head_chars - tail_chars;
    let head: String = trimmed.chars().take(head_chars).collect();
    let tail: String = trimmed.chars().skip(total_chars - tail_chars).collect();
    (
        format!("{head}\n... [{omitted} characters omitted] ...\n{tail}"),
        true,
    )
}
