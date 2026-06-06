//! Native goal-run continuation verdict — Stage 5.
//!
//! Judges whether a /goal completion condition is satisfied from transcript
//! evidence. The LLM call reuses the native transport. The goal-run state machine
//! (rounds, history, UI listeners) stays in TS; only this stateless verdict moves.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalEvalInput {
    pub condition: String,
    /// Pre-built transcript (TS builds it from message store — UI-coupled).
    pub transcript: String,
    pub open_todo_summaries: Vec<String>,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalEvalVerdict {
    pub satisfied: bool,
    pub blocked: bool,
    pub reason: String,
    pub source: String,
}

/// Ask the model whether the completion condition is satisfied. Returns null on failure.
#[tauri::command]
pub async fn ai_goal_eval_verdict(input: GoalEvalInput) -> Result<Option<GoalEvalVerdict>, String> {
    let todo_block = if input.open_todo_summaries.is_empty() {
        "Open TodoWrite tasks: none".to_string()
    } else {
        let lines: Vec<String> = input.open_todo_summaries.iter().take(8).map(|l| format!("- {l}")).collect();
        format!("Open TodoWrite tasks:\n{}", lines.join("\n"))
    };

    let system = [
        "You evaluate whether a software agent completion condition is satisfied.",
        "You do NOT execute tools. Judge only from the transcript evidence the agent already surfaced.",
        "Return strict JSON with keys: satisfied (boolean), blocked (boolean), reason (string).",
        "satisfied=true only when the condition is fully met with evidence in the transcript.",
        "For smoke/test goals (words like test, smoke, demo, тест, демо): satisfied=true once the agent ran tools and reported verification — not full product delivery.",
        "blocked=true when user credentials, product decisions, or external input is required before continuing.",
        "If satisfied and blocked are both false, the worker should continue.",
        "Keep reason under 220 characters.",
    ].join("\n");

    let user = format!(
        "Completion condition:\n{}\n\n{}\n\nTranscript:\n{}",
        input.condition.trim(),
        todo_block,
        input.transcript,
    );

    let payload = serde_json::json!({
        "model": input.model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
        "temperature": 0.1,
        "stream": false,
    });

    let request = crate::ai_chat_backend::AiChatCompletionRequest::new(
        input.base_url.clone(),
        input.api_key.clone(),
        payload,
    );

    match crate::ai_chat_backend::completion(request).await {
        Ok(response) => {
            let content = response.body.pointer("/choices/0/message/content").and_then(|v| v.as_str()).unwrap_or("");
            Ok(parse_verdict(content))
        }
        Err(_) => Ok(None),
    }
}

/// Extract the first JSON object from the model output and parse the verdict.
fn parse_verdict(content: &str) -> Option<GoalEvalVerdict> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if end < start { return None; }
    let json_slice = &content[start..=end];
    let value: serde_json::Value = serde_json::from_str(json_slice).ok()?;
    let satisfied = value.get("satisfied")?.as_bool().unwrap_or(false);
    let blocked = value.get("blocked").and_then(|v| v.as_bool()).unwrap_or(false);
    let reason: String = value.get("reason").and_then(|v| v.as_str()).unwrap_or("").chars().take(220).collect();
    Some(GoalEvalVerdict { satisfied, blocked, reason, source: "model".to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clean_json() {
        let v = parse_verdict(r#"{"satisfied": true, "blocked": false, "reason": "done"}"#).unwrap();
        assert!(v.satisfied);
        assert!(!v.blocked);
        assert_eq!(v.reason, "done");
    }

    #[test]
    fn parse_json_with_surrounding_text() {
        let v = parse_verdict("Here is my verdict:\n{\"satisfied\": false, \"blocked\": true, \"reason\": \"needs API key\"}\nDone.").unwrap();
        assert!(!v.satisfied);
        assert!(v.blocked);
        assert_eq!(v.reason, "needs API key");
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert!(parse_verdict("no json here").is_none());
        assert!(parse_verdict("").is_none());
    }

    #[test]
    fn reason_truncated_to_220() {
        let long_reason = "x".repeat(300);
        let input = format!(r#"{{"satisfied": true, "blocked": false, "reason": "{long_reason}"}}"#);
        let v = parse_verdict(&input).unwrap();
        assert_eq!(v.reason.chars().count(), 220);
    }
}
