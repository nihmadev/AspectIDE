use tokio::sync::Mutex;

use crate::types::ParsedToolCall;

pub const MAX_PARALLEL_NATIVE_SUBAGENTS: usize = 4;

pub fn file_edit_lock() -> &'static Mutex<()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub fn is_parallel_safe_read_tool(name: &str) -> bool {
    matches!(
        name,
        "Read"
            | "Grep"
            | "Glob"
            | "SemanticSearch"
            | "RelatedFiles"
            | "RepoMap"
            | "WorkspaceIndex"
            | "SymbolContext"
            | "GitContext"
            | "DiagnosticsContext"
            | "ReadLints"
            | "InspectFile"
            | "TerminalContext"
            | "ActiveContext"
            | "FastContext"
            | "RulesContext"
            | "DocsContext"
            | "MemoryContext"
            | "SecretGuard"
            | "FailureAnalyzer"
            | "BrowserStatus"
            | "BrowserSnapshot"
            | "BrowserScreenshot"
    )
}

pub fn parallel_task_call_ids(calls: &[ParsedToolCall]) -> Vec<String> {
    let mut task_ids: Vec<String> = calls
        .iter()
        .filter(|c| c.name == "Task")
        .map(|c| c.id.clone())
        .collect();
    task_ids.truncate(MAX_PARALLEL_NATIVE_SUBAGENTS);
    task_ids
}
