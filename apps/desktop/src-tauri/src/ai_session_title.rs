//! Native chat session title generation — Stage 5.
//!
//! Picks a fast model, asks it for an ultra-short title, normalizes the result.
//! The LLM call reuses the native `ai_chat_backend::completion` transport.
//! TS keeps only the debounce/inflight/rename orchestration (UI state).

use serde::Deserialize;

const DEFAULT_TITLE: &str = "New chat";
const MAX_TITLE_CHARS: usize = 42;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleModel {
    pub id: String,
    pub alias: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateTitleInput {
    pub first_user_message: String,
    pub base_url: String,
    pub api_key: Option<String>,
    /// Candidate models — Rust picks the fastest (haiku/mini/flash/…).
    pub models: Vec<TitleModel>,
    pub active_model_alias: String,
}

/// Generate a short session title. Falls back to a heuristic title on any failure.
#[tauri::command]
pub async fn ai_generate_session_title(input: GenerateTitleInput) -> Result<String, String> {
    let snippet: String = input
        .first_user_message
        .trim()
        .chars()
        .take(1_200)
        .collect();
    let fallback = heuristic_title(&input.first_user_message);
    if snippet.is_empty() {
        return Ok(fallback);
    }

    let model = pick_title_model(&input.models, &input.active_model_alias);
    let payload = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": "You generate ultra-short IDE chat session titles. Return only the title (max 6 words), same language as the user message. No quotes, no markdown, no trailing punctuation." },
            { "role": "user", "content": format!("First user message:\n{snippet}") },
        ],
        "temperature": 0.3,
        "stream": false,
    });

    let request = crate::ai_chat_backend::AiChatCompletionRequest::new(
        input.base_url.clone(),
        input.api_key.clone(),
        payload,
    );

    match crate::ai_chat_backend::completion(request).await {
        Ok(response) => {
            let raw = response
                .body
                .pointer("/choices/0/message/content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let parsed = parse_generated_title(raw);
            if parsed.is_empty() || parsed == DEFAULT_TITLE {
                Ok(fallback)
            } else {
                Ok(parsed)
            }
        }
        Err(_) => Ok(fallback),
    }
}

/// Pick the fastest title-generation model from the provider's catalog.
fn pick_title_model(models: &[TitleModel], active_alias: &str) -> String {
    let mut best: Option<(&TitleModel, i32)> = None;
    for model in models {
        let haystack = format!("{} {} {}", model.id, model.alias, model.name).to_lowercase();
        let mut score = 0;
        if haystack.contains("haiku")
            || haystack.contains("mini")
            || haystack.contains("nano")
            || haystack.contains("flash")
            || haystack.contains("small")
            || haystack.contains("fast")
            || haystack.contains("lite")
            || haystack.contains("8b")
        {
            score += 40;
        }
        if haystack.contains("haiku") || haystack.contains("flash") {
            score += 20;
        }
        if haystack.contains("mini") || haystack.contains("nano") {
            score += 15;
        }
        if model.alias == active_alias || model.id == active_alias {
            score += 5;
        }
        if score > 0 && best.as_ref().is_none_or(|(_, s)| score > *s) {
            best = Some((model, score));
        }
    }
    best.map_or_else(
        || active_alias.to_string(),
        |(m, _)| {
            if m.alias.is_empty() {
                m.id.clone()
            } else {
                m.alias.clone()
            }
        },
    )
}

fn parse_generated_title(raw: &str) -> String {
    let line = raw
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let without_prefix = line
        .strip_prefix("title:")
        .or_else(|| line.strip_prefix("Title:"))
        .or_else(|| line.strip_prefix("chat title:"))
        .or_else(|| line.strip_prefix("Chat title:"))
        .unwrap_or(line)
        .trim();
    normalize_title(without_prefix)
}

fn heuristic_title(first_user_message: &str) -> String {
    normalize_title(first_user_message)
}

fn normalize_title(value: &str) -> String {
    let normalized: String = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return DEFAULT_TITLE.to_string();
    }
    let dequoted = normalized.trim_matches(|c| matches!(c, '"' | '\'' | '`'));
    let mut emph: &str = dequoted;
    for pair in ["**", "__", "*", "_"] {
        if emph.len() > 2 * pair.len() && emph.starts_with(pair) && emph.ends_with(pair) {
            emph = &emph[pair.len()..emph.len() - pair.len()];
            break;
        }
    }
    let stripped = emph.trim_end_matches(['.', '!', '?']).trim();
    if stripped.is_empty() {
        return DEFAULT_TITLE.to_string();
    }
    if stripped.chars().count() > MAX_TITLE_CHARS {
        let truncated: String = stripped.chars().take(MAX_TITLE_CHARS).collect();
        format!("{}...", truncated.trim_end())
    } else {
        stripped.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_quotes_and_punctuation() {
        assert_eq!(normalize_title("\"Fix the bug.\""), "Fix the bug");
        assert_eq!(normalize_title("  multiple   spaces  "), "multiple spaces");
        assert_eq!(normalize_title(""), DEFAULT_TITLE);
    }

    #[test]
    fn normalize_truncates_long_titles() {
        let long = "a".repeat(60);
        let result = normalize_title(&long);
        assert!(result.ends_with("..."));
        assert!(result.chars().count() <= MAX_TITLE_CHARS + 3);
    }

    #[test]
    fn parse_strips_title_prefix() {
        assert_eq!(
            parse_generated_title("Title: Build auth flow"),
            "Build auth flow"
        );
        assert_eq!(
            parse_generated_title("chat title: Refactor parser"),
            "Refactor parser"
        );
    }

    #[test]
    fn pick_fastest_model() {
        let models = vec![
            TitleModel {
                id: "gpt-5".into(),
                alias: "gpt-5".into(),
                name: "GPT-5".into(),
            },
            TitleModel {
                id: "claude-haiku".into(),
                alias: "haiku".into(),
                name: "Claude Haiku".into(),
            },
        ];
        assert_eq!(pick_title_model(&models, "gpt-5"), "haiku");
    }

    #[test]
    fn pick_model_falls_back_to_active() {
        let models = vec![TitleModel {
            id: "gpt-5".into(),
            alias: "gpt-5".into(),
            name: "GPT-5".into(),
        }];
        assert_eq!(pick_title_model(&models, "gpt-5"), "gpt-5");
    }
}
