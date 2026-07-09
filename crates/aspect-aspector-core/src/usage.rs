pub fn build_system_message(system: &str, anthropic_cache: bool) -> serde_json::Value {
    if anthropic_cache {
        serde_json::json!({
            "role": "system",
            "content": [{
                "type": "text",
                "text": system,
                "cache_control": { "type": "ephemeral" },
            }],
        })
    } else {
        serde_json::json!({ "role": "system", "content": system })
    }
}

pub fn parse_cached_prompt_tokens(usage: &serde_json::Value) -> u64 {
    let direct = usage
        .get("cache_read_input_tokens")
        .or_else(|| usage.get("cached_tokens"))
        .and_then(serde_json::Value::as_u64);
    if let Some(value) = direct {
        return value;
    }
    usage
        .get("prompt_tokens_details")
        .or_else(|| usage.get("input_tokens_details"))
        .and_then(|details| details.get("cached_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

pub fn accumulate_usage(
    usage: &serde_json::Value,
    prompt: &mut u64,
    completion: &mut u64,
    total: &mut u64,
    cached: &mut u64,
) {
    let round_prompt = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let round_completion = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    *prompt += round_prompt;
    *completion += round_completion;
    *total += usage
        .get("total_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(round_prompt + round_completion);
    *cached += parse_cached_prompt_tokens(usage);
}
