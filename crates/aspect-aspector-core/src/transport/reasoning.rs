use serde_json::Value;

use super::types::ReasoningEffortFix;

/// Merge a frontend-provided reasoning payload into an outgoing request payload.
pub fn merge_reasoning(payload: &mut Value, reasoning: Option<&Value>) {
    let (Some(Value::Object(extra)), Some(target)) = (reasoning, payload.as_object_mut()) else {
        return;
    };
    for (key, value) in extra {
        target.insert(key.clone(), value.clone());
    }
}

/// True when the frontend sent a non-empty reasoning blob.
pub fn reasoning_present(reasoning: Option<&Value>) -> bool {
    matches!(reasoning, Some(Value::Object(map)) if !map.is_empty())
}

/// Insert `temperature` only for standard models, not reasoning models.
pub fn apply_temperature(payload: &mut Value, reasoning: Option<&Value>, temperature: f64) {
    if reasoning_present(reasoning) {
        return;
    }
    if let Some(target) = payload.as_object_mut() {
        target.insert("temperature".to_string(), serde_json::json!(temperature));
    }
}

/// Strength ranking of the reasoning-effort vocabulary across providers.
fn reasoning_effort_rank(effort: &str) -> i8 {
    match effort.to_ascii_lowercase().as_str() {
        "none" => 0,
        "minimal" => 1,
        "low" => 2,
        "medium" => 3,
        "high" => 4,
        "xhigh" => 5,
        "max" => 6,
        _ => -1,
    }
}

/// Collect every backtick- or single-quote-delimited token from `text`, in order.
fn quoted_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find(['`', '\'']) {
        let quote = rest.as_bytes()[open] as char;
        let after = &rest[open + 1..];
        let Some(close) = after.find(quote) else {
            break;
        };
        let token = &after[..close];
        if !token.is_empty() && token.len() <= 24 && !token.contains(char::is_whitespace) {
            tokens.push(token.to_string());
        }
        rest = &after[close + 1..];
    }
    tokens
}

/// Parse a provider 400 that rejects our reasoning-effort value.
fn parse_rejected_reasoning_effort(error: &str) -> Option<(String, Vec<String>)> {
    let lower = error.to_ascii_lowercase();
    if !lower.contains("reasoning") {
        return None;
    }
    if let Some(at) = lower.find("unknown variant") {
        let rejected = quoted_tokens(&error[at..]).into_iter().next()?;
        let expected = lower[at..].find("expected one of")?;
        let allowed = quoted_tokens(&error[at + expected..]);
        return Some((rejected, allowed));
    }
    if let Some(at) = lower.find("supported values are") {
        let before = quoted_tokens(&error[..at]);
        let rejected = before
            .iter()
            .rev()
            .find(|token| reasoning_effort_rank(token) >= 0)
            .or_else(|| {
                before.iter().rev().find(|token| {
                    !token.contains('.') && !token.eq_ignore_ascii_case("reasoning_effort")
                })
            })
            .cloned()?;
        let allowed = quoted_tokens(&error[at..]);
        return Some((rejected, allowed));
    }
    None
}

/// Auto-recover from a provider rejecting the configured reasoning effort.
pub fn reasoning_effort_fallback(
    reasoning: Option<&mut Value>,
    error: &str,
) -> Option<ReasoningEffortFix> {
    let reasoning = reasoning?;
    let (rejected, allowed) = parse_rejected_reasoning_effort(error)?;
    let applied = allowed
        .iter()
        .filter(|effort| reasoning_effort_rank(effort) >= 0)
        .max_by_key(|effort| reasoning_effort_rank(effort))?
        .clone();
    if applied.eq_ignore_ascii_case(&rejected) {
        return None;
    }
    let target = reasoning.as_object_mut()?;
    let flat_touched = if target.contains_key("reasoning_effort") {
        target.insert(
            "reasoning_effort".to_string(),
            Value::String(applied.clone()),
        );
        true
    } else {
        false
    };
    let nested_touched = if let Some(Value::Object(inner)) = target.get_mut("reasoning") {
        inner.insert("effort".to_string(), Value::String(applied.clone()));
        true
    } else {
        false
    };
    if !flat_touched && !nested_touched {
        target.insert(
            "reasoning_effort".to_string(),
            Value::String(applied.clone()),
        );
    }
    Some(ReasoningEffortFix {
        requested: rejected,
        applied,
    })
}
