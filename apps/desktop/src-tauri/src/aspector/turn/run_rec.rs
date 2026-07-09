use aspect_ai_core::*;

#[allow(clippy::too_many_arguments)]
pub async fn run_recovery_synthesis(
    _app: &tauri::AppHandle,
    _state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    _turn_id: &str,
    _messages: &mut Vec<serde_json::Value>,
    _tools: &[serde_json::Value],
    final_content: &mut String,
    _usage_prompt: u64,
    _usage_completion: u64,
    _usage_total: u64,
    _usage_cached: u64,
    _model_calls: u64,
) -> Option<String> {
    if !final_content.trim().is_empty() {
        return None;
    }
    let goal = crate::aspector::session::store::get_goal(&input.session_id);
    let recovery_prompt = serde_json::json!({
        "model": input.model,
        "messages": vec![
            serde_json::json!({ "role": "user", "content": format!(
                "The previous attempt did not produce a complete answer. Based on the conversation, provide a helpful response to the user's request. User goal: {}", goal
            )}),
        ],
        "max_tokens": 500,
        "stream": false,
    });
    let request = crate::aspector::transport::AiChatCompletionRequest::with_protocol(
        input.base_url.clone(), input.api_key.clone(), recovery_prompt,
        input.prompt_input.provider_protocol.clone(),
    );
    match crate::aspector::transport::completion(request, |_notice| {}).await {
        Ok(response) => {
            let text = response.body.get("choices")
                .and_then(|c| c.as_array())
                .and_then(|c| c.first())
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default();
            if text.is_empty() { None } else { Some(text) }
        }
        Err(_) => None,
    }
}
