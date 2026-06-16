//! Native semantic code search (Stage 1 of the TS→Rust migration).
//!
//! Replaces the TypeScript `semanticSearch` tool: instead of 3 separate IPC calls
//! (LSP symbols + text search + file list) followed by ranking in the webview, this
//! composes the `lux-lsp`, `lux-search`, and `lux-fs` services in one native command
//! and ranks everything in Rust. One IPC round-trip, no large file lists crossing the
//! bridge, native scoring.
//!
//! The scoring heuristics are a faithful port of the previous TS implementation
//! (`aiRuntimeFileContext.ts`) so result ordering is preserved — see the parity unit
//! tests at the bottom.

use std::collections::BTreeMap;
use std::path::PathBuf;

use lux_core::{LspWorkspaceSymbol, SearchOptions};
use serde::Serialize;
use tauri::State;

use crate::{workspace_root, SharedState};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSemanticResult {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub source: &'static str,
    pub score: i64,
    pub path: String,
    pub relative_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "kind", skip_serializing_if = "Option::is_none")]
    pub symbol_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_text: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSemanticSearchResponse {
    pub workspace_root: PathBuf,
    pub query: String,
    pub path_filter: Option<String>,
    pub count: usize,
    pub results: Vec<AiSemanticResult>,
}

#[tauri::command]
pub async fn ai_semantic_search(
    state: State<'_, SharedState>,
    query: String,
    path: Option<String>,
    max_results: Option<usize>,
    max_files: Option<usize>,
) -> Result<AiSemanticSearchResponse, String> {
    let root = workspace_root(&state)?;
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err("SemanticSearch requires a non-empty query.".to_string());
    }
    let max_results = max_results.unwrap_or(24).clamp(1, 80);
    let path_filter = path
        .map(|p| normalize_slashes(p.trim()).to_lowercase())
        .filter(|p| !p.is_empty());
    let file_cap = max_files.unwrap_or(5_000).clamp(500, 20_000);
    let search_max = (max_results * 4).clamp(40, 120);
    let tokens = tokenize(&query);
    let root_str = normalize_slashes(&root.to_string_lossy());

    // 1. LSP workspace symbols (best-effort: empty if the server is not ready).
    let mut symbols: Vec<LspWorkspaceSymbol> = Vec::new();
    {
        let mut lsp = state.lsp.lock().await;
        if let Some(manager) = lsp.as_mut() {
            if let Ok(mut found) = manager.workspace_symbols(query.clone()).await {
                found.truncate(max_results.max(40));
                symbols = found;
            }
        }
    }

    // 2. Indexed text search.
    let options = SearchOptions {
        case_sensitive: false,
        whole_word: false,
        use_regex: false,
        include_hidden: false,
        include_globs: Vec::new(),
        exclude_globs: Vec::new(),
        max_results: search_max,
    };
    let search_hits = {
        let root = root.clone();
        let query = query.clone();
        tokio::task::spawn_blocking(move || lux_search::query(root, query, &options))
            .await
            .map_err(|error| error.to_string())?
            .map(|response| response.hits)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "ai_semantic_search: indexed search backend failed");
                Default::default()
            })
    };

    // 3. Workspace file candidates.
    let files = {
        let root = root.clone();
        tokio::task::spawn_blocking(move || lux_fs::list_files(root, file_cap))
            .await
            .map_err(|error| error.to_string())?
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "ai_semantic_search: file listing backend failed");
                Default::default()
            })
    };

    let mut results: BTreeMap<String, AiSemanticResult> = BTreeMap::new();
    let normalized_query = query.to_lowercase();

    for symbol in &symbols {
        let path = normalize_slashes(&symbol.location.path.to_string_lossy());
        if !passes_path_filter(&path, path_filter.as_deref()) {
            continue;
        }
        let descriptor = Descriptor::new(&path, &root_str);
        let score = score_symbol(symbol, &normalized_query, &tokens, &descriptor);
        let symbol_kind = serde_json::to_value(symbol.kind)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string));
        let preview = match &symbol.container_name {
            Some(container) if !container.is_empty() => format!("{container}.{}", symbol.name),
            _ => symbol.name.clone(),
        };
        upsert(
            &mut results,
            AiSemanticResult {
                kind: "symbol",
                source: "lsp-symbols",
                score,
                path: descriptor.path.clone(),
                relative_path: descriptor.relative_path.clone(),
                line: Some(symbol.location.range.start_line + 1),
                column: Some(symbol.location.range.start_column + 1),
                name: Some(symbol.name.clone()),
                symbol_kind,
                container_name: symbol.container_name.clone(),
                preview: Some(preview),
                match_text: None,
            },
        );
    }

    for hit in &search_hits {
        let path = normalize_slashes(&hit.path.to_string_lossy());
        if !passes_path_filter(&path, path_filter.as_deref()) {
            continue;
        }
        let descriptor = Descriptor::new(&path, &root_str);
        let score = score_text_hit(&descriptor, &hit.preview, &hit.match_text, &tokens);
        upsert(
            &mut results,
            AiSemanticResult {
                kind: "text",
                source: "indexed-search",
                score,
                path: descriptor.path.clone(),
                relative_path: descriptor.relative_path.clone(),
                line: Some(u32::try_from(hit.line).unwrap_or(u32::MAX)),
                column: Some(u32::try_from(hit.column).unwrap_or(u32::MAX)),
                name: None,
                symbol_kind: None,
                container_name: None,
                preview: Some(hit.preview.clone()),
                match_text: Some(hit.match_text.clone()),
            },
        );
    }

    let file_limit = (max_results * 2).min(80);
    let mut file_candidates: Vec<(Descriptor, i64)> = files
        .iter()
        .filter(|entry| matches!(entry.kind, lux_core::FsEntryKind::File))
        .map(|entry| normalize_slashes(&entry.path.to_string_lossy()))
        .filter(|path| !is_low_signal_path(path))
        .map(|path| Descriptor::new(&path, &root_str))
        .filter(|descriptor| passes_path_filter(&descriptor.path, path_filter.as_deref()))
        .map(|descriptor| {
            let score = score_file(&descriptor, &tokens);
            (descriptor, score)
        })
        .filter(|(_, score)| *score > 0)
        .collect();
    file_candidates.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.relative_lower.cmp(&b.0.relative_lower))
    });
    file_candidates.truncate(file_limit);
    for (descriptor, score) in file_candidates {
        let kind_label = language_for_path(&descriptor.basename_lower);
        let preview = descriptor.relative_path.clone();
        let name = descriptor.basename.clone();
        upsert(
            &mut results,
            AiSemanticResult {
                kind: "file",
                source: "workspace-index",
                score,
                path: descriptor.path.clone(),
                relative_path: descriptor.relative_path,
                line: None,
                column: None,
                name: Some(name),
                symbol_kind: Some(kind_label),
                container_name: None,
                preview: Some(preview),
                match_text: None,
            },
        );
    }

    let mut ranked: Vec<AiSemanticResult> = results.into_values().collect();
    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.unwrap_or(0).cmp(&b.line.unwrap_or(0)))
    });
    ranked.truncate(max_results);

    Ok(AiSemanticSearchResponse {
        workspace_root: root,
        query,
        path_filter,
        count: ranked.len(),
        results: ranked,
    })
}

// ---------------------------------------------------------------------------
// Ranking — faithful port of aiRuntimeFileContext.ts
// ---------------------------------------------------------------------------

struct Descriptor {
    path: String,
    relative_path: String,
    relative_lower: String,
    basename: String,
    basename_lower: String,
    family_stem_lower: String,
}

impl Descriptor {
    fn new(path: &str, root_lower_root: &str) -> Self {
        let path = normalize_slashes(path);
        let root = normalize_slashes(root_lower_root.trim_end_matches('/'));
        let relative_path = if !root.is_empty()
            && path
                .to_lowercase()
                .starts_with(&format!("{}/", root.to_lowercase()))
        {
            path.get(root.len() + 1..).unwrap_or(&path).to_string()
        } else {
            path.clone()
        };
        let basename = path.rsplit('/').next().unwrap_or(&path).to_string();
        let family_stem = family_stem(&basename);
        Self {
            path,
            relative_lower: relative_path.to_lowercase(),
            relative_path,
            basename_lower: basename.to_lowercase(),
            basename,
            family_stem_lower: family_stem.to_lowercase(),
        }
    }
}

fn normalize_slashes(value: &str) -> String {
    value.replace('\\', "/")
}

pub fn normalize_slashes_pub(value: &str) -> String {
    normalize_slashes(value)
}
pub fn file_extension_pub(basename_lower: &str) -> String {
    file_extension(basename_lower)
}
pub fn family_stem_pub(basename: &str) -> String {
    family_stem(basename)
}
pub fn tokenize_pub(query: &str) -> Vec<String> {
    tokenize(query)
}
pub fn is_low_signal_path_pub(path: &str) -> bool {
    is_low_signal_path(path)
}
pub fn score_path_pub(path: &str) -> i64 {
    score_path(path)
}
pub fn language_for_path_pub(basename_lower: &str) -> String {
    language_for_path(basename_lower)
}

fn passes_path_filter(path: &str, filter: Option<&str>) -> bool {
    filter.is_none_or(|filter| normalize_slashes(path).to_lowercase().contains(filter))
}

fn score_symbol(
    symbol: &LspWorkspaceSymbol,
    normalized_query: &str,
    tokens: &[String],
    file: &Descriptor,
) -> i64 {
    let name = symbol.name.to_lowercase();
    let container = symbol
        .container_name
        .as_deref()
        .unwrap_or("")
        .to_lowercase();
    let mut score = 80 + score_path(&file.relative_path);
    if name == normalized_query {
        score += 90;
    } else if name.contains(normalized_query) {
        score += 55;
    }
    if container.contains(normalized_query) {
        score += 25;
    }
    for token in tokens {
        if name.contains(token) {
            score += if token.len() >= 6 { 24 } else { 16 };
        }
        if container.contains(token) {
            score += 12;
        }
        if file.relative_lower.contains(token) {
            score += 10;
        }
    }
    if is_test_file(file) {
        score -= 10;
    }
    if is_important_project_file(file) {
        score += 8;
    }
    score
}

fn score_text_hit(file: &Descriptor, preview: &str, match_text: &str, tokens: &[String]) -> i64 {
    let haystack = format!("{}\n{}\n{}", file.relative_lower, preview, match_text).to_lowercase();
    let mut score = 50 + score_path(&file.relative_path);
    for token in tokens {
        if haystack.contains(token) {
            score += if token.len() >= 6 { 18 } else { 11 };
        }
        if file.basename_lower.contains(token) {
            score += 10;
        }
    }
    let lower_preview = preview.to_lowercase();
    if [
        "function",
        "class",
        "interface",
        "type",
        "struct",
        "enum",
        "impl",
        "export",
        "const",
        "async",
    ]
    .iter()
    .any(|kw| lower_preview.contains(kw))
    {
        score += 12;
    }
    if is_test_file(file) {
        score -= 8;
    }
    if is_important_project_file(file) {
        score += 6;
    }
    score
}

fn score_file(file: &Descriptor, tokens: &[String]) -> i64 {
    let mut score = 0;
    for token in tokens {
        if file.basename_lower.contains(token) {
            score += if token.len() >= 6 { 34 } else { 22 };
        }
        if file.family_stem_lower.contains(token) {
            score += 16;
        }
        if file.relative_lower.contains(token) {
            score += 10;
        }
    }
    if score == 0 {
        return 0;
    }
    score += score_path(&file.relative_path).min(30);
    if is_important_project_file(file) {
        score += 16;
    }
    if is_test_file(file) {
        score -= 6;
    }
    score
}

fn score_path(path: &str) -> i64 {
    let lower = path.to_lowercase().replace('\\', "/");
    let mut score = 0;
    if lower.ends_with("package.json")
        || lower.ends_with("cargo.toml")
        || lower.contains("vite.config.")
        || lower.contains("tsconfig.")
        || lower.contains("readme")
        || lower.contains("src/app.")
        || lower.contains("src/main.")
        || lower.contains("src-tauri/src/lib.rs")
    {
        score += 100;
    }
    if lower.contains("/src/") {
        score += 25;
    }
    if lower.contains("/components/") {
        score += 10;
    }
    if lower.contains("/node_modules/") || lower.contains("/target/") || lower.contains("/dist/") {
        score -= 200;
    }
    score
}

fn upsert(results: &mut BTreeMap<String, AiSemanticResult>, result: AiSemanticResult) {
    let detail = result
        .name
        .clone()
        .or_else(|| result.match_text.clone())
        .unwrap_or_default();
    let key = format!(
        "{}:{}:{}:{}",
        result.kind,
        result.path.to_lowercase(),
        result.line.unwrap_or(0),
        detail.to_lowercase(),
    );
    match results.get(&key) {
        Some(existing) if existing.score >= result.score => {}
        _ => {
            results.insert(key, result);
        }
    }
}

const TEST_SEGMENT_WORDS: &[&str] = &["test", "spec", "tests", "specs"];
const TEST_DIR_WORDS: &[&str] = &["__tests__", "test", "tests", "spec", "specs"];

fn is_test_file(file: &Descriptor) -> bool {
    let base_parts = split_delims(&file.basename_lower);
    if base_parts
        .iter()
        .any(|p| TEST_SEGMENT_WORDS.contains(&p.as_str()))
    {
        return true;
    }
    file.relative_lower
        .split('/')
        .any(|segment| TEST_DIR_WORDS.contains(&segment))
}

fn is_important_project_file(file: &Descriptor) -> bool {
    const NAMES: &[&str] = &[
        "package.json",
        "cargo.toml",
        "pyproject.toml",
        "go.mod",
        "pom.xml",
        "build.gradle",
        "dockerfile",
        "makefile",
        ".env.example",
    ];
    const PREFIXES: &[&str] = &["vite.config.", "tsconfig.", "jsconfig."];
    let rel = &file.relative_lower;
    for name in NAMES {
        if rel == name || rel.ends_with(&format!("/{name}")) {
            return true;
        }
    }
    for prefix in PREFIXES {
        if rel.starts_with(prefix) || rel.contains(&format!("/{prefix}")) {
            return true;
        }
    }
    rel.contains("readme")
}

fn is_low_signal_path(path: &str) -> bool {
    const IGNORED_DIRS: &[&str] = &[
        "node_modules",
        "target",
        "dist",
        "build",
        "out",
        "coverage",
        ".git",
        ".next",
        ".turbo",
        "vendor",
        "venv",
        ".venv",
        "__pycache__",
    ];
    const BINARY_EXTS: &[&str] = &[
        ".7z", ".avi", ".bmp", ".class", ".db", ".dll", ".dmg", ".exe", ".gif", ".gz", ".ico",
        ".jar", ".jpeg", ".jpg", ".lockb", ".mov", ".mp3", ".mp4", ".o", ".obj", ".pdf", ".png",
        ".rar", ".so", ".tar", ".ttf", ".webm", ".webp", ".woff", ".woff2", ".zip",
    ];
    let lower = normalize_slashes(path).to_lowercase();
    if lower
        .split('/')
        .any(|segment| IGNORED_DIRS.contains(&segment))
    {
        return true;
    }
    if BINARY_EXTS.iter().any(|ext| lower.ends_with(ext)) {
        return true;
    }
    !is_source_path(&lower) && !is_extensionless_project_file(&lower)
}

const SOURCE_EXTS: &[&str] = &[
    ".astro", ".c", ".cc", ".cpp", ".cs", ".css", ".cxx", ".go", ".graphql", ".gql", ".h", ".hpp",
    ".html", ".java", ".js", ".json", ".jsx", ".kt", ".kts", ".less", ".md", ".mdx", ".mjs",
    ".mts", ".php", ".proto", ".py", ".rb", ".rs", ".sass", ".scss", ".sql", ".svelte", ".swift",
    ".toml", ".ts", ".tsx", ".vue", ".xml", ".yaml", ".yml",
];

fn is_source_path(lower: &str) -> bool {
    SOURCE_EXTS.iter().any(|ext| lower.ends_with(ext))
}

fn is_extensionless_project_file(lower_path: &str) -> bool {
    let basename = lower_path.rsplit('/').next().unwrap_or(lower_path);
    matches!(
        basename,
        "dockerfile"
            | "makefile"
            | "readme"
            | "license"
            | "notice"
            | "procfile"
            | "gemfile"
            | "rakefile"
    )
}

fn language_for_path(lower: &str) -> String {
    let lang = if ends_with_any(lower, &[".tsx", ".ts", ".mts", ".cts"]) {
        "typescript"
    } else if ends_with_any(lower, &[".jsx", ".js", ".mjs", ".cjs"]) {
        "javascript"
    } else if ends_with_any(lower, &[".rs"]) {
        "rust"
    } else if ends_with_any(lower, &[".py"]) {
        "python"
    } else if ends_with_any(lower, &[".go"]) {
        "go"
    } else if ends_with_any(lower, &[".java", ".kt", ".kts"]) {
        "jvm"
    } else if ends_with_any(lower, &[".cs"]) {
        "csharp"
    } else if ends_with_any(lower, &[".css", ".scss", ".sass", ".less"]) {
        "styles"
    } else if ends_with_any(lower, &[".json", ".yaml", ".yml", ".toml", ".xml"]) {
        "config-data"
    } else if ends_with_any(lower, &[".md", ".mdx"])
        || lower.contains("readme")
        || lower.contains("license")
        || lower.contains("notice")
    {
        "docs"
    } else if ends_with_any(lower, &[".html", ".vue", ".svelte", ".astro"]) {
        "web"
    } else if ends_with_any(lower, &[".sql", ".graphql", ".gql", ".proto"]) {
        "schema"
    } else {
        "other"
    };
    lang.to_string()
}

fn ends_with_any(value: &str, exts: &[&str]) -> bool {
    exts.iter().any(|ext| value.ends_with(ext))
}

fn file_extension(basename_lower: &str) -> String {
    for special in [".d.ts", ".d.mts", ".d.cts"] {
        if basename_lower.ends_with(special) {
            return special.to_string();
        }
    }
    match basename_lower.rfind('.') {
        Some(dot) if dot > 0 => basename_lower[dot..].to_string(),
        _ => String::new(),
    }
}

const FAMILY_SUFFIXES: &[&str] = &[
    "test",
    "spec",
    "stories",
    "story",
    "module",
    "types",
    "schema",
    "route",
    "routes",
    "model",
    "models",
    "entity",
    "entities",
    "service",
    "controller",
    "view",
    "styles",
    "style",
    "component",
    "page",
    "layout",
    "hook",
    "hooks",
    "util",
    "utils",
    "helper",
    "helpers",
];

fn family_stem(basename: &str) -> String {
    // Strip a trailing `.d`-aware extension, then up to two known `[-_.]suffix` segments.
    let lower = basename.to_lowercase();
    let ext = file_extension(&lower);
    // Preserve the original-case stem (TS parity: `familyStemFromBasename` does not
    // lowercase its result) by slicing `basename` when its byte length matches `lower`
    // — the common ASCII case, where `ext` (a suffix of `lower`) maps to a valid char
    // boundary in `basename`. When lowercasing changes byte length (e.g. 'İ' -> 'i' +
    // combining dot) fall back to slicing `lower`, which is always boundary-safe.
    let src: &str = if basename.len() == lower.len() {
        basename
    } else {
        &lower
    };
    let mut stem = src[..src.len().saturating_sub(ext.len())].to_string();
    for _ in 0..2 {
        let stem_lower = stem.to_lowercase();
        let mut stripped = false;
        for suffix in FAMILY_SUFFIXES {
            for delim in ['.', '-', '_'] {
                let tail = format!("{delim}{suffix}");
                if stem_lower.ends_with(&tail) {
                    stem = stem[..stem.len() - tail.len()].to_string();
                    stripped = true;
                    break;
                }
            }
            if stripped {
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    stem
}

fn split_delims(value: &str) -> Vec<String> {
    value
        .split(['.', '_', '-'])
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

const STOP_WORDS: &[&str] = &[
    "about", "after", "also", "and", "any", "are", "bug", "can", "code", "create", "default",
    "edit", "file", "files", "fix", "for", "from", "get", "has", "have", "into", "make", "need",
    "new", "not", "now", "please", "set", "that", "the", "this", "tool", "tools", "use", "with",
    "work",
];
const SHORT_USEFUL: &[&str] = &["ai", "api", "ci", "db", "fs", "gh", "ui", "ux"];

fn tokenize(query: &str) -> Vec<String> {
    // Insert a space at lower/digit -> upper boundaries (camelCase split).
    let mut spaced = String::with_capacity(query.len() + 8);
    let chars: Vec<char> = query.chars().collect();
    for (index, ch) in chars.iter().enumerate() {
        if index > 0 {
            let prev = chars[index - 1];
            if (prev.is_ascii_lowercase() || prev.is_ascii_digit()) && ch.is_ascii_uppercase() {
                spaced.push(' ');
            }
        }
        spaced.push(*ch);
    }
    let lowered = spaced.to_lowercase();

    let mut seen: Vec<String> = Vec::new();
    for raw in lowered.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
        let token = raw.trim_matches(|c| c == '-' || c == '_');
        if token.is_empty() {
            continue;
        }
        let owned = token.to_string();
        if owned.len() < 3 && !SHORT_USEFUL.contains(&owned.as_str()) {
            continue;
        }
        if STOP_WORDS.contains(&owned.as_str()) {
            continue;
        }
        if !seen.iter().any(|t| t == &owned) {
            seen.push(owned);
        }
        if seen.len() >= 12 {
            break;
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_splits_camelcase_and_filters() {
        assert_eq!(tokenize("parseUserInput"), vec!["parse", "user", "input"]);
        // stop words and too-short tokens dropped; short-useful kept.
        assert_eq!(tokenize("fix the api db"), vec!["api", "db"]);
        // dedupe + max 12.
        assert_eq!(tokenize("render render render"), vec!["render"]);
    }

    #[test]
    fn score_path_priorities() {
        assert!(score_path("apps/desktop/src/lib/store.ts") >= 25);
        assert!(score_path("package.json") >= 100);
        assert!(score_path("/root/node_modules/x/index.js") <= -100);
    }

    #[test]
    fn test_and_important_detection() {
        let test = Descriptor::new("/root/src/foo.test.ts", "/root");
        assert!(is_test_file(&test));
        let pkg = Descriptor::new("/root/package.json", "/root");
        assert!(is_important_project_file(&pkg));
        let plain = Descriptor::new("/root/src/app.ts", "/root");
        assert!(!is_test_file(&plain));
    }

    #[test]
    fn low_signal_paths() {
        assert!(is_low_signal_path("/root/node_modules/x/a.js"));
        assert!(is_low_signal_path("/root/assets/logo.png"));
        assert!(!is_low_signal_path("/root/src/app.ts"));
        assert!(!is_low_signal_path("/root/Dockerfile"));
    }

    #[test]
    fn descriptor_relative_and_family() {
        let d = Descriptor::new("/root/src/userProfile.service.ts", "/root");
        assert_eq!(d.relative_path, "src/userProfile.service.ts");
        assert_eq!(d.basename, "userProfile.service.ts");
        // family stem strips extension + `.service` suffix.
        assert_eq!(d.family_stem_lower, "userprofile");
    }

    #[test]
    fn file_scoring_requires_token_hit() {
        let d = Descriptor::new("/root/src/auth/login.ts", "/root");
        assert!(score_file(&d, &tokenize("login")) > 0);
        assert_eq!(score_file(&d, &tokenize("zzzzzz")), 0);
    }
}
