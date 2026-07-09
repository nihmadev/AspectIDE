use aspect_ai_core::*;

/// Read-before-edit guard.
pub fn require_file_read_before_edit(
    state: &tauri::State<'_, crate::SharedState>,
    session_id: &str,
    tool: &str,
    raw_path: &str,
) -> Result<(), String> {
    let Ok(resolved) = crate::resolve_workspace_path(state, std::path::Path::new(raw_path)) else {
        return Ok(());
    };
    if !resolved.is_file() {
        return Ok(());
    }
    if crate::aspector::session::store::was_file_read(session_id, &resolved) {
        return Ok(());
    }
    Err(format!(
        "{tool} blocked: read {raw_path} before editing it. Call Read (or InspectFile) on this file first, then retry the edit so the change is based on its current contents."
    ))
}

/// Best-effort pre-edit checkpoint capture.
pub async fn augment_checkpoint_before_edit(
    state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    raw_path: &str,
) {
    let Some(checkpoint_id) = input
        .file_checkpoint_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    let Ok(root) = crate::workspace_root(state) else {
        return;
    };
    let root_str = crate::aspector::context::semantic::normalize_slashes_pub(&root.to_string_lossy());
    if let Err(error) = crate::aspector::session::checkpoint::augment_checkpoint(
        state,
        &root_str,
        checkpoint_id,
        None,
        vec![raw_path.to_string()],
    )
    .await
    {
        tracing::debug!(
            %error, checkpoint_id, path = raw_path,
            "augment_checkpoint_before_edit: pre-edit checkpoint capture failed (non-fatal)"
        );
    }
}

/// Check if content is effectively empty.
pub fn is_empty_message_content(content: &serde_json::Value) -> bool {
    match content {
        serde_json::Value::String(s) => s.trim().is_empty(),
        serde_json::Value::Array(arr) => arr.iter().all(|part| {
            part.get("text").and_then(|t| t.as_str()).is_some_and(|t| t.trim().is_empty())
        }),
        _ => true,
    }
}

#[allow(dead_code)]
/// Best-effort embedding for memory tools.
pub async fn maybe_embed(input: &TurnInput, text: &str) -> Option<Vec<f32>> {
    let model = input.embedding_model.as_deref()?.trim();
    if model.is_empty() {
        return None;
    }
    if crate::aspector::anthropic::is_anthropic(&input.prompt_input.provider_protocol) {
        return None;
    }
    match crate::aspector::transport::embeddings(&input.base_url, input.api_key.as_deref(), model, text).await {
        Ok(vector) => Some(vector),
        Err(error) => {
            tracing::debug!("embedding generation skipped: {error}");
            None
        }
    }
}

/// Emit error event and usage on turn failure.
pub fn handle_turn_error(_app: &tauri::AppHandle, error: &str) {
    tracing::error!("Turn error: {error}");
}

#[allow(dead_code)]
/// Ensure path strings use forward slashes.
pub fn normalize_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
