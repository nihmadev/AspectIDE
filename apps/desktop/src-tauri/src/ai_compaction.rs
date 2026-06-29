//! Native context-compaction summarization — Stage 5.
//!
//! Compresses an IDE chat transcript into a durable checkpoint summary via the
//! native transport. TS keeps the message-store management (pruning, checkpoint
//! insertion, fingerprinting) since that mutates the React-rendered history.

use serde::Deserialize;

const MAX_SUMMARY_CHARS: usize = 18_000;
const MAX_TRANSCRIPT_CHARS: usize = 84_000;

/// Required markdown headings in the order the prompt emits them. Used to
/// validate that a model-generated summary covers all critical sections before
/// storage.
const REQUIRED_SECTIONS: &[&str] = &[
    "## Task goal",
    "## Latest user direction",
    "## Open tasks",
    "## Progress",
    "## Key decisions / constraints",
    "## Files and tools",
    "## Errors / blockers",
    "## Critical preserved facts",
    "## Open items / next step",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionSummaryInput {
    pub transcript: String,
    #[serde(default)]
    pub previous_summary: String,
    #[serde(default)]
    pub pinned_goal: String,
    #[serde(default)]
    pub open_tasks: Vec<String>,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    /// Provider wire protocol (`openai-compatible` or `anthropic`); selects the
    /// transport so checkpoint summaries work on Anthropic providers too.
    #[serde(default)]
    pub protocol: String,
    /// Provider reasoning payload (`reasoning_effort` + reasoning.effort), or absent/`{}`
    /// when the active model has no effort levels. Merged into the request so the
    /// checkpoint summary is produced with the selected reasoning depth.
    #[serde(default)]
    pub reasoning: Option<serde_json::Value>,
}

/// Summarize a transcript into a checkpoint. Errors if the model returns nothing
/// (caller falls back to a deterministic TS summary) or if the output truncation
/// would drop critical required sections.
#[tauri::command]
pub async fn ai_compaction_summary(input: CompactionSummaryInput) -> Result<String, String> {
    let max_tokens = MAX_SUMMARY_CHARS / 4;
    let system = format!(
        "You compress IDE pair-programming chat history into a durable checkpoint for a coding agent.\n\
         This is not a casual summary. It is the ONLY memory the agent will have for older turns, so quality after compaction must be no lower than before it.\n\
         Preserve exactly: the task goal, the latest user direction, every active task with its status, constraints, decisions, files/paths touched, tool outcomes, errors/blockers, verification results, and the precise next step.\n\
         Never replace concrete facts with vague prose. Do not invent facts. Do not omit unresolved bugs or tasks. Do not say 'see above'.\n\
         If the transcript and the pinned goal/open tasks conflict, include both and mark the conflict.\n\
         Required markdown sections, in this exact order: ## Task goal, ## Latest user direction, ## Open tasks, ## Progress, ## Key decisions / constraints, ## Files and tools, ## Errors / blockers, ## Critical preserved facts, ## Open items / next step.\n\
         Prefer preserving facts over being short, but stay under {max_tokens} tokens."
    );

    let mut user_parts: Vec<String> = Vec::new();
    if !input.pinned_goal.trim().is_empty() {
        user_parts.push(format!(
            "Pinned session goal:\n{}",
            truncate(&input.pinned_goal, 2_000)
        ));
    }
    if !input.open_tasks.is_empty() {
        let tasks = input
            .open_tasks
            .iter()
            .map(|t| format!("- {t}"))
            .collect::<Vec<_>>()
            .join("\n");
        user_parts.push(format!("Open tasks:\n{tasks}"));
    }
    if !input.previous_summary.trim().is_empty() {
        user_parts.push(format!(
            "Previous checkpoint to merge:\n{}",
            truncate(&input.previous_summary, 4_000)
        ));
    }
    // Use tail-preserving compaction for the ordered transcript so the latest
    // user direction, tool output, and errors are always visible to the summary
    // model instead of only the oldest prefix.
    user_parts.push(format!(
        "Transcript ({} chars):\n{}",
        input.transcript.len(),
        transcript_tail(&input.transcript, MAX_TRANSCRIPT_CHARS),
    ));

    let mut payload = serde_json::json!({
        "model": input.model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user_parts.join("\n\n") },
        ],
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    crate::ai_chat_backend::merge_reasoning(&mut payload, input.reasoning.as_ref());
    crate::ai_chat_backend::apply_temperature(&mut payload, input.reasoning.as_ref(), 0.2);
    // `max_tokens` is the legacy OpenAI name that reasoning models reject; the
    // prompt already bounds summary length, so cap only standard models.
    if !crate::ai_chat_backend::reasoning_present(input.reasoning.as_ref()) {
        if let Some(target) = payload.as_object_mut() {
            target.insert("max_tokens".to_string(), serde_json::json!(max_tokens));
        }
    }

    let request = crate::ai_chat_backend::AiChatCompletionRequest::with_protocol(
        input.base_url.clone(),
        input.api_key.clone(),
        payload,
        input.protocol.clone(),
    );

    // Stream: a non-streaming request hangs against SSE-only providers/proxies, and
    // compaction fires automatically mid-conversation, so a stall reads as the chat
    // freezing. The summary itself isn't surfaced token-by-token, so on_delta is a
    // no-op.
    let response =
        crate::ai_chat_backend::completion_streaming(request, |_, _| {}, || false, |_| {}).await?;
    let content = response
        .body
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Err("Compaction summary was empty.".to_string());
    }

    // Validate the stored summary always has every required section present and
    // non-empty — not only when truncation clips it. A blind clip can silently
    // drop "Errors / blockers", "Critical preserved facts", or "Open items /
    // next step" (the sections compaction exists to preserve), but a short yet
    // malformed generation that fits under the cap is equally unusable as the
    // agent's only memory of older turns. Either way we reject so the caller
    // falls back to its deterministic summary instead of storing partial state.
    let result = truncate(&content, MAX_SUMMARY_CHARS);
    let clipped = result != content;
    if let Err(missing) = validate_summary_sections(&result) {
        return Err(if clipped {
            format!(
                "Compaction summary exceeds {MAX_SUMMARY_CHARS} chars and {missing}. Regenerate with more concise output."
            )
        } else {
            format!("Compaction summary {missing}. Regenerate covering all required sections.")
        });
    }

    Ok(result)
}

/// Validate that all required markdown sections are present and non-empty.
///
/// For each heading in `REQUIRED_SECTIONS`, locates its position, determines the
/// body boundary by searching for the next required heading that follows it, then
/// checks the body contains non-whitespace content.
fn validate_summary_sections(content: &str) -> Result<(), String> {
    for (i, &heading) in REQUIRED_SECTIONS.iter().enumerate() {
        let pos = content
            .find(heading)
            .ok_or_else(|| format!("required section '{heading}' is missing"))?;
        let start = pos + heading.len();
        let end = REQUIRED_SECTIONS[i + 1..]
            .iter()
            .filter_map(|&next| content[start..].find(next))
            .min()
            .map_or(content.len(), |offset| start + offset);
        let body = content[start..end].trim();
        if body.is_empty() {
            return Err(format!("required section '{heading}' is empty"));
        }
    }
    Ok(())
}

/// Keep the last `max` characters of `value`, preserving the message order that
/// matters most for continuity (tool output, errors, latest user direction).
fn transcript_tail(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let tail_start = value.len()
        - value
            .chars()
            .rev()
            .take(max)
            .map(char::len_utf8)
            .sum::<usize>();
    let tail: String = value[tail_start..].chars().collect();
    format!("…{tail}")
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        let truncated: String = value.chars().take(max).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_respects_char_boundary() {
        assert_eq!(truncate("short", 100), "short");
        let long = "a".repeat(200);
        let result = truncate(&long, 50);
        assert_eq!(result.chars().count(), 51); // 50 + ellipsis
        assert!(result.ends_with('…'));
    }

    #[test]
    fn transcript_tail_keeps_suffix() {
        assert_eq!(transcript_tail("short", 100), "short");
        let long = "abcdefghij";
        let result = transcript_tail(long, 4);
        assert!(result.starts_with('…'));
        assert!(result.contains("ghij"));
        assert_eq!(result.chars().count(), 5); // 4 + ellipsis
    }

    #[test]
    fn validate_sections_passes_complete_summary() {
        let content = "\
## Task goal
Fix the login flow.

## Latest user direction
Add two-factor auth.

## Open tasks
- Implement TOTP
- Add backup codes

## Progress
50% complete

## Key decisions / constraints
Use time-based OTP

## Files and tools
login.rs, auth.rs

## Errors / blockers
None so far

## Critical preserved facts
User wants TOTP

## Open items / next step
PR review
";
        assert!(validate_summary_sections(content).is_ok());
    }

    #[test]
    fn validate_sections_rejects_missing_heading() {
        let content = "\
## Task goal
Fix the login flow.
## Latest user direction
Add two-factor auth.
";
        assert!(validate_summary_sections(content).is_err());
    }

    #[test]
    fn validate_sections_rejects_empty_section() {
        let content = "\
## Task goal
Fix the login flow.

## Latest user direction

## Open tasks
- Implement TOTP
"
        .to_string();
        // Need all sections present; for brevity just check the empty section is caught.
        let mut full = content;
        for rest in &REQUIRED_SECTIONS[3..] {
            full.push('\n');
            full.push_str(rest);
            full.push_str("\ncontent\n");
        }
        assert!(validate_summary_sections(&full).is_err());
    }
}
