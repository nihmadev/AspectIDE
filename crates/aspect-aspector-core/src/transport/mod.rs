//! AI provider transport: HTTP clients, SSE streaming, retry logic, history I/O.
//!
//! Extracted from `apps/desktop/src-tauri/src/aspector/transport.rs`. This module
//! owns all provider-facing communication: non-streaming completions, SSE streaming,
//! model listing, embeddings, provider diagnostics, reasoning-effort fallback, and
//! chat history persistence. Tauri-specific glue (AppHandle, #[tauri::command])
//! lives in the desktop crate's `aspector/transport.rs` re-export module.

pub mod types;
pub mod reasoning;
pub mod endpoints;
pub mod retry;
pub mod race;
pub mod sse;
pub mod inline_think;
pub mod stream_types;
pub mod stream_acc;
pub mod anthropic_acc;
pub mod stream_mode;
pub mod auth;
pub mod completion;
pub mod streaming;
pub mod models;
pub mod diagnostic;
pub mod history;
pub mod stream_feed;

pub use types::*;
pub use reasoning::*;
pub use endpoints::*;
pub use retry::*;
pub use race::*;
pub use sse::*;
pub use inline_think::*;
pub use stream_types::*;
pub use stream_acc::*;
pub use anthropic_acc::AnthropicStreamAccumulator;
pub use stream_mode::StreamMode;
pub use auth::*;
pub use completion::*;
pub use streaming::*;
pub use models::*;
pub use diagnostic::*;
pub use history::*;
pub use stream_feed::*;
