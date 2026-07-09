//! AI provider transport — re-exported from `aspect-ai-core` with Tauri glue.
//!
//! All core logic (HTTP clients, SSE streaming, retry, history, diagnostics,
//! embeddings, model listing, reasoning helpers) lives in `aspect_ai_core::transport`.
//! This module re-exports everything and adds Tauri-specific trait implementations
//! (`EventEmitter` for `AppHandle`, `PathProvider` for `AppHandle`) plus the
//! `#[tauri::command]` function wrappers.

pub use aspect_ai_core::transport::*;

use std::path::PathBuf;

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use aspect_ai_core::transport::{AiChatStreamEvent, EventEmitter, PathProvider};

// EventEmitter and PathProvider are implemented via the Tauri AppHandle
// through wrapper functions, not direct trait impls (avoids orphan rules).
// See `emit_stream_event` and `get_data_dir` free functions below.

pub fn emit_stream_event(
    app: &AppHandle,
    stream_id: &str,
    kind: &str,
    data: Option<Value>,
    error: Option<String>,
) -> Result<(), String> {
    app.emit(
        "aspect://ai-chat-stream",
        AiChatStreamEvent {
            stream_id: stream_id.to_string(),
            kind: kind.to_string(),
            data,
            error,
        },
    )
    .map_err(|e| e.to_string())
}

pub fn get_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|e| e.to_string())
}

/// Tauri command wrapper for listing provider models.
#[tauri::command]
pub async fn ai_list_provider_models(
    base_url: String,
    api_key: Option<String>,
) -> Result<Vec<String>, String> {
    list_provider_models(base_url, api_key).await
}

// ── Wrapper types to bridge AppHandle → EventEmitter / PathProvider ──
// These avoid orphan-rule violations (foreign traits + foreign type).
use tokio::sync::oneshot;

struct AppEventEmitter(AppHandle);
impl EventEmitter for AppEventEmitter {
    fn emit_stream(&self, stream_id: &str, kind: &str, data: Option<Value>, error: Option<String>) -> Result<(), String> {
        emit_stream_event(&self.0, stream_id, kind, data, error)
    }
}

struct AppPathProvider<'a>(&'a AppHandle);
impl PathProvider for AppPathProvider<'_> {
    fn data_dir(&self) -> Result<PathBuf, String> {
        get_data_dir(self.0)
    }
}

/// Run a streaming completion with a Tauri `AppHandle`.
pub async fn run_completion_stream_app(
    app: AppHandle,
    stream_id: String,
    request: AiChatCompletionStreamRequest,
    cancel_rx: oneshot::Receiver<()>,
) {
    run_completion_stream(AppEventEmitter(app), stream_id, request, cancel_rx).await
}

/// Load chat history with a Tauri `AppHandle`.
pub fn history_load_app(app: &AppHandle) -> Result<AiChatHistoryResponse, String> {
    history_load(&AppPathProvider(app))
}

/// Save chat history with a Tauri `AppHandle`.
pub fn history_save_app(
    app: &AppHandle,
    request: AiChatHistorySaveRequest,
) -> Result<AiChatHistoryResponse, String> {
    history_save(&AppPathProvider(app), request)
}
