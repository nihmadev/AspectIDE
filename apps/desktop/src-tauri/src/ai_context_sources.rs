//! Native `RulesContext`, `DocsContext`, `MemoryContext` tools — Stage 4.
//!
//! All three follow the same pattern: list workspace files → filter by path
//! pattern → score by query tokens → read top-N files → return JSON.

use std::path::PathBuf;

use serde::Serialize;
use tauri::State;
use tokio::io::AsyncReadExt;

use crate::ai_semantic;
use crate::{workspace_root, SharedState};

const MAX_FILE_BYTES: u64 = 12_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextFile {
    pub path: String,
    pub relative_path: String,
    pub size: u64,
    pub truncated: bool,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiContextSourceResponse {
    pub tool: String,
    pub workspace_root: PathBuf,
    pub query: String,
    pub count: usize,
    pub files: Vec<ContextFile>,
}

#[tauri::command]
pub async fn ai_rules_context(
    state: State<'_, SharedState>,
    query: Option<String>,
    max_files: Option<usize>,
    max_scan: Option<usize>,
) -> Result<AiContextSourceResponse, String> {
    context_source_tool(
        &state,
        "RulesContext",
        query,
        max_files.unwrap_or(12),
        max_scan.unwrap_or(5000),
        is_rules_path,
    )
    .await
}

#[tauri::command]
pub async fn ai_docs_context(
    state: State<'_, SharedState>,
    query: Option<String>,
    max_files: Option<usize>,
    max_scan: Option<usize>,
) -> Result<AiContextSourceResponse, String> {
    context_source_tool(
        &state,
        "DocsContext",
        query,
        max_files.unwrap_or(12),
        max_scan.unwrap_or(5000),
        is_docs_path,
    )
    .await
}

#[tauri::command]
pub async fn ai_memory_context(
    state: State<'_, SharedState>,
    query: Option<String>,
    max_files: Option<usize>,
    max_scan: Option<usize>,
) -> Result<AiContextSourceResponse, String> {
    context_source_tool(
        &state,
        "MemoryContext",
        query,
        max_files.unwrap_or(14),
        max_scan.unwrap_or(5000),
        is_memory_path,
    )
    .await
}

async fn context_source_tool(
    state: &State<'_, SharedState>,
    tool_name: &str,
    query: Option<String>,
    max_files: usize,
    max_scan: usize,
    filter: fn(&str, &str) -> bool,
) -> Result<AiContextSourceResponse, String> {
    let root = workspace_root(state)?;
    let root_str = ai_semantic::normalize_slashes_pub(&root.to_string_lossy());
    let query_str = query.unwrap_or_default().trim().to_string();
    let tokens = ai_semantic::tokenize_pub(&query_str);
    let max_files = max_files.clamp(1, 40);
    let max_scan = max_scan.clamp(500, 20_000);

    let entries = {
        let root = root.clone();
        tokio::task::spawn_blocking(move || lux_fs::list_files(root, max_scan))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?
    };

    let mut candidates: Vec<(String, String, i64)> = entries
        .iter()
        .filter(|e| matches!(e.kind, lux_core::FsEntryKind::File))
        .map(|e| {
            let path = ai_semantic::normalize_slashes_pub(&e.path.to_string_lossy());
            let rel = relative_path(&path, &root_str);
            (path, rel)
        })
        .filter(|(_, rel)| filter(rel, &root_str))
        .map(|(path, rel)| {
            let score = score_context_file(&rel, &tokens);
            (path, rel, score)
        })
        .collect();
    candidates.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.cmp(&b.1)));
    candidates.truncate(max_files);

    let mut files = Vec::with_capacity(candidates.len());
    for (path, rel, _) in &candidates {
        // Bounded read: cap the bytes pulled off disk at `MAX_FILE_BYTES` so a
        // huge matching file can't be fully buffered into memory. `size` is the
        // real on-disk length (from metadata), so truncation semantics are kept.
        let read = async {
            let file = tokio::fs::File::open(path).await?;
            let size = file.metadata().await?.len();
            let mut buf = Vec::new();
            file.take(MAX_FILE_BYTES).read_to_end(&mut buf).await?;
            std::io::Result::Ok((buf, size))
        }
        .await;
        match read {
            Ok((buf, size)) => {
                let limit = usize::try_from(MAX_FILE_BYTES).unwrap_or(usize::MAX);
                let truncated = size > MAX_FILE_BYTES;
                let text = String::from_utf8_lossy(&buf).into_owned();
                let clamped = if text.len() > limit {
                    let mut end = limit;
                    while end > 0 && !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    text[..end].to_string()
                } else {
                    text
                };
                files.push(ContextFile {
                    path: path.clone(),
                    relative_path: rel.clone(),
                    size,
                    truncated,
                    text: clamped,
                    error: None,
                });
            }
            Err(err) => {
                files.push(ContextFile {
                    path: path.clone(),
                    relative_path: rel.clone(),
                    size: 0,
                    truncated: false,
                    text: String::new(),
                    error: Some(err.to_string()),
                });
            }
        }
    }

    Ok(AiContextSourceResponse {
        tool: tool_name.to_string(),
        workspace_root: root,
        query: query_str,
        count: files.len(),
        files,
    })
}

fn relative_path(path: &str, root: &str) -> String {
    let root_lower = root.to_lowercase();
    if !root_lower.is_empty() && path.to_lowercase().starts_with(&format!("{root_lower}/")) {
        path[root.len() + 1..].to_string()
    } else {
        path.to_string()
    }
}

fn score_context_file(relative_lower: &str, tokens: &[String]) -> i64 {
    let lower = relative_lower.to_lowercase();
    let mut score: i64 = 10;
    for token in tokens {
        if lower.contains(token) {
            score += if token.len() >= 6 { 20 } else { 12 };
        }
    }
    // Boost well-known files.
    if lower.ends_with("agents.md")
        || lower.ends_with("claude.md")
        || lower.ends_with(".cursorrules")
    {
        score += 50;
    }
    if lower.contains("readme") {
        score += 30;
    }
    if lower.ends_with("package.json") || lower.ends_with("cargo.toml") {
        score += 25;
    }
    score
}

// ── Path classification (ports isRulesContextPath, isDocsContextPath, isMemoryContextPath) ──

/// Returns `true` when `path`'s file extension matches any of `exts` (ASCII case-insensitive).
fn has_ext(path: &str, exts: &[&str]) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|ext| {
            exts.iter()
                .any(|&candidate| ext.eq_ignore_ascii_case(candidate))
        })
}

const RULES_FILENAMES: &[&str] = &[
    "agents.md",
    "claude.md",
    ".cursorrules",
    "cursor_rules.md",
    "cursor-rules.md",
    "codex.md",
];

fn is_rules_path(rel: &str, _root: &str) -> bool {
    let lower = rel.to_lowercase();
    let basename = lower.rsplit('/').next().unwrap_or(&lower);
    RULES_FILENAMES.contains(&basename)
        || lower.starts_with(".cursor/rules/")
        || lower.contains("/.cursor/rules/")
        || (lower.contains("/rules/") && has_ext(&lower, &["md", "mdx", "txt"]))
}

fn is_docs_path(rel: &str, _root: &str) -> bool {
    let lower = rel.to_lowercase();
    if ai_semantic::is_low_signal_path_pub(rel) {
        return false;
    }
    lower.contains("readme")
        || lower.contains("contributing")
        || lower.contains("changelog")
        || lower.contains("architecture")
        || lower.starts_with("docs/")
        || lower.contains("/docs/")
        || lower.ends_with("package.json")
        || lower.ends_with("cargo.toml")
        || lower.ends_with("pyproject.toml")
        || lower.ends_with("go.mod")
        || lower.contains("vite.config.")
        || lower.contains("tsconfig.")
}

const MEMORY_FILENAMES: &[&str] = &[
    "memory.md",
    "memories.md",
    "project-memory.md",
    "decisions.md",
    "decision-log.md",
    "preferences.md",
    "notes.md",
    "todo.md",
    "todos.md",
    "roadmap.md",
    "agents.md",
    "claude.md",
    "codex.md",
    ".cursorrules",
];

fn is_memory_path(rel: &str, _root: &str) -> bool {
    let lower = rel.to_lowercase();
    if ai_semantic::is_low_signal_path_pub(rel) {
        return false;
    }
    let basename = lower.rsplit('/').next().unwrap_or(&lower);
    if MEMORY_FILENAMES.contains(&basename) {
        return true;
    }
    let ext_ok = has_ext(&lower, &["md", "mdx", "txt", "json", "yaml", "yml", "toml"]);
    if !ext_ok {
        return false;
    }
    lower.split('/').any(|seg| {
        matches!(
            seg,
            "adr"
                | "adrs"
                | "decisions"
                | "decision"
                | "memory"
                | "notes"
                | "roadmap"
                | "todos"
                | "todo"
                | ".codex"
                | ".cursor"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_path_detection() {
        assert!(is_rules_path("AGENTS.md", ""));
        assert!(is_rules_path(".cursor/rules/my-rule.md", ""));
        assert!(!is_rules_path("src/app.ts", ""));
    }

    #[test]
    fn docs_path_detection() {
        assert!(is_docs_path("README.md", ""));
        assert!(is_docs_path("docs/architecture.md", ""));
        assert!(is_docs_path("package.json", ""));
        assert!(!is_docs_path("node_modules/x/README.md", ""));
    }

    #[test]
    fn memory_path_detection() {
        assert!(is_memory_path("memory.md", ""));
        assert!(is_memory_path("adr/0001-use-rust.md", ""));
        assert!(!is_memory_path("src/app.ts", ""));
    }
}
