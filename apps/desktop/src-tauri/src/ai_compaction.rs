//! Native context-compaction summarization — Stage 5.
//!
//! Compresses an IDE chat transcript into a durable checkpoint summary via the
//! native transport. TS keeps the message-store management (pruning, checkpoint
//! insertion, fingerprinting) since that mutates the React-rendered history.

use serde::Deserialize;

const MAX_SUMMARY_CHARS: usize = 12_000;
const MAX_TRANSCRIPT_CHARS: usize = 48_000;

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
    /// Provider reasoning payload (reasoning_effort + reasoning.effort), or absent/`{}`
    /// when the active model has no effort levels. Merged into the request so the
    /// checkpoint summary is produced with the selected reasoning depth.
    #[serde(default)]
    pub reasoning: Option<serde_json::Value>,
}

/// Summarize a transcript into a checkpoint. Errors if the model returns nothing
/// (caller falls back to a deterministic TS summary).
#[tauri::command]
pub async fn ai_compaction_summary(input: CompactionSummaryInput) -> Result<String, String> {
    let max_tokens = MAX_SUMMARY_CHARS / 4;
    let system = format!(
        "You compress IDE pair-programming chat history into a durable checkpoint.\n\
         Preserve: task goal, constraints, decisions, files/paths, tool outcomes, errors, and the exact next step.\n\
         Do not invent facts. Do not add filler. Use markdown headings.\n\
         Required sections: ## Task goal, ## Progress, ## Key decisions, ## Files and tools, ## Open items / next step\n\
         Stay under {max_tokens} tokens."
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
    user_parts.push(format!(
        "Transcript ({} chars):\n{}",
        input.transcript.len(),
        truncate(&input.transcript, MAX_TRANSCRIPT_CHARS),
    ));

    let mut payload = serde_json::json!({
        "model": input.model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user_parts.join("\n\n") },
        ],
        "temperature": 0.2,
        "stream": false,
        "max_tokens": max_tokens,
    });
    crate::ai_chat_backend::merge_reasoning(&mut payload, input.reasoning.as_ref());

    let request = crate::ai_chat_backend::AiChatCompletionRequest::new(
        input.base_url.clone(),
        input.api_key.clone(),
        payload,
    );

    let response = crate::ai_chat_backend::completion(request).await?;
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
    Ok(truncate(&content, MAX_SUMMARY_CHARS))
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
}
