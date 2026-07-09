use serde_json::Value;

use super::anthropic_acc::AnthropicStreamAccumulator;
use super::stream_acc::StreamAccumulator;

/// Streaming accumulator that adapts to the provider protocol.
pub enum StreamMode {
    OpenAi(StreamAccumulator),
    Anthropic(AnthropicStreamAccumulator),
}

impl StreamMode {
    pub fn ingest<F: FnMut(&str, &str), T: FnMut(&str)>(
        &mut self,
        value: &Value,
        on_delta: &mut F,
        on_tool_start: &mut T,
    ) {
        match self {
            Self::OpenAi(acc) => acc.ingest(value, on_delta, on_tool_start),
            Self::Anthropic(acc) => acc.ingest(value, on_delta, on_tool_start),
        }
    }

    pub fn flush<F: FnMut(&str, &str)>(&mut self, on_delta: &mut F) {
        if let Self::OpenAi(acc) = self {
            acc.flush(on_delta);
        }
    }

    pub fn stream_error(&self) -> Option<&str> {
        match self {
            Self::Anthropic(acc) => acc.error.as_deref(),
            Self::OpenAi(acc) => acc.stream_error.as_deref(),
        }
    }

    pub fn into_response_body(self) -> Value {
        match self {
            Self::OpenAi(acc) => acc.into_response_body(),
            Self::Anthropic(acc) => acc.into_response_body(),
        }
    }
}
