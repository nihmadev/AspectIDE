//! Re-export of the Anthropic protocol adapter from `aspect-ai-core`.
//!
//! Kept as `crate::aspector::anthropic` for backwards compatibility with existing
//! callers. New code should import from `aspect_ai_core::protocol` directly.

pub use aspect_ai_core::protocol::*;
