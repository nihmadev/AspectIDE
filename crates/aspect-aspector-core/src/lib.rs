//! Native AI turn loop core — types, registries, pure logic utilities, protocol
//! adapters, and AI provider transport.
//!
//! Extracted from `apps/desktop/src-tauri/src/ai_turn.rs` and
//! `apps/desktop/src-tauri/src/aspector/`. Contains types, pure logic, protocol
//! translation (OpenAI ↔ Anthropic), and the full provider HTTP transport (SSE
//! streaming, retry, diagnostics, model listing, embeddings, history persistence).
//! Tauri-specific glue lives in the desktop crate's thin re-export modules.

#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::too_many_lines)]

pub mod types;
pub mod registry;
pub mod usage;
pub use usage::*;
pub mod plan_quality;
pub use plan_quality::*;
pub mod response;
pub use response::*;
pub mod tool_names;
pub use tool_names::*;
pub mod secrets;
pub use secrets::*;
pub mod json_helpers;
pub use json_helpers::*;
pub mod browser;
pub use browser::*;
pub mod parallel;
pub use parallel::*;
pub mod approval;
pub use approval::*;
pub mod subagent;
pub use subagent::*;

pub mod turn;

pub use types::*;
pub use registry::*;

pub mod protocol;
pub mod transport;
