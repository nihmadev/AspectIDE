use aspect_research::{FocusMode, ResearchDepth, ResearchOptions};
use aspect_ssh::TransferDirection;

pub fn json_str(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

pub fn json_str_opt(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

pub fn json_usize(value: &serde_json::Value, key: &str, default: usize) -> usize {
    value
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(default)
}

pub fn json_str_array(value: &serde_json::Value, key: &str, max: usize) -> Vec<String> {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .take(max)
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_research_focus(
    args: &serde_json::Value,
) -> (FocusMode, Option<String>) {
    let mode_str = json_str(args, "focus");
    let focus = match mode_str.as_str() {
        "web" | "general" => FocusMode::Web,
        "academic" | "scholarly" | "papers" => FocusMode::Academic,
        "news" | "recent" => FocusMode::News,
        "social" | "forums" | "discussion" => FocusMode::Social,
        "video" => FocusMode::Video,
        "code" | "developer" | "dev" => FocusMode::Code,
        _ => FocusMode::Web,
    };
    let focus_hint = json_str_opt(args, "query")
        .or_else(|| json_str_opt(args, "question"));
    (focus, focus_hint)
}

pub fn parse_transfer_direction(value: &str) -> TransferDirection {
    match value.to_lowercase().as_str() {
        "upload" | "to_remote" | "send" => TransferDirection::Upload,
        "download" | "from_remote" | "receive" => TransferDirection::Download,
        _ => TransferDirection::Upload,
    }
}

pub fn parse_research_depth(value: &str) -> ResearchDepth {
    match value.to_lowercase().as_str() {
        "deep" | "thorough" | "comprehensive" => ResearchDepth::Deep,
        _ => ResearchDepth::Standard,
    }
}

pub fn parse_research_options(args: &serde_json::Value) -> ResearchOptions {
    ResearchOptions {
        max_sources: json_usize(args, "max_sources", 6),
        depth: parse_research_depth(&json_str(args, "depth")),
        focus: parse_research_focus(args).0,
        max_chars_per_source: json_usize(args, "max_chars_per_source", 2_400),
    }
}
