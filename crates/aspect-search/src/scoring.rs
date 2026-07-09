use std::collections::BTreeMap;

use aspect_core::LspWorkspaceSymbol;

use crate::classify::{is_important_project_file, is_test_file};
use crate::path::{family_stem, language_for_path, normalize_slashes, score_path};
use crate::types::AiSemanticResult;
use crate::filter::passes_path_filter;
use crate::filter::PathFilter;

pub(crate) struct Descriptor {
    pub path: String,
    pub relative_path: String,
    pub relative_lower: String,
    pub basename: String,
    pub basename_lower: String,
    pub family_stem_lower: String,
}

impl Descriptor {
    pub(crate) fn new(path: &str, root_lower_root: &str) -> Self {
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
    if is_test_file(&file.basename_lower, &file.relative_lower) {
        score -= 10;
    }
    if is_important_project_file(&file.relative_lower) {
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
        "function", "class", "interface", "type", "struct", "enum", "impl",
        "export", "const", "async",
    ]
    .iter()
    .any(|kw| lower_preview.contains(kw))
    {
        score += 12;
    }
    if is_test_file(&file.basename_lower, &file.relative_lower) {
        score -= 8;
    }
    if is_important_project_file(&file.relative_lower) {
        score += 6;
    }
    score
}

fn score_file(file: &Descriptor, tokens: &[String]) -> i64 {
    let mut score = 0i64;
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
    if is_important_project_file(&file.relative_lower) {
        score += 16;
    }
    if is_test_file(&file.basename_lower, &file.relative_lower) {
        score -= 6;
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

pub(crate) fn collect_symbol_results(
    results: &mut BTreeMap<String, AiSemanticResult>,
    symbols: &[LspWorkspaceSymbol],
    root_str: &str,
    normalized_query: &str,
    tokens: &[String],
    path_matcher: Option<&PathFilter>,
) {
    for symbol in symbols {
        let path = normalize_slashes(&symbol.location.path.to_string_lossy());
        if crate::path::is_low_signal_path(&path) {
            continue;
        }
        if !passes_path_filter(&path, path_matcher) {
            continue;
        }
        let descriptor = Descriptor::new(&path, root_str);
        let score = score_symbol(symbol, normalized_query, tokens, &descriptor);
        let symbol_kind = serde_json::to_value(symbol.kind)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string));
        let preview = match &symbol.container_name {
            Some(container) if !container.is_empty() => {
                format!("{container}.{}", symbol.name)
            }
            _ => symbol.name.clone(),
        };
        upsert(
            results,
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
}

pub(crate) fn collect_search_results(
    results: &mut BTreeMap<String, AiSemanticResult>,
    search_hits: &[aspect_core::SearchHit],
    root_str: &str,
    tokens: &[String],
    path_matcher: Option<&PathFilter>,
) {
    for hit in search_hits {
        let path = normalize_slashes(&hit.path.to_string_lossy());
        if crate::path::is_low_signal_path(&path) {
            continue;
        }
        if !passes_path_filter(&path, path_matcher) {
            continue;
        }
        let descriptor = Descriptor::new(&path, root_str);
        let score = score_text_hit(&descriptor, &hit.preview, &hit.match_text, tokens);
        upsert(
            results,
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
}

pub(crate) fn collect_file_results(
    results: &mut BTreeMap<String, AiSemanticResult>,
    files: &[aspect_core::FsEntry],
    root_str: &str,
    tokens: &[String],
    path_matcher: Option<&PathFilter>,
    file_limit: usize,
) {
    let mut file_candidates: Vec<(Descriptor, i64)> = files
        .iter()
        .filter(|entry| matches!(entry.kind, aspect_core::FsEntryKind::File))
        .map(|entry| normalize_slashes(&entry.path.to_string_lossy()))
        .filter(|path| !crate::path::is_low_signal_path(path))
        .map(|path| Descriptor::new(&path, root_str))
        .filter(|descriptor| passes_path_filter(&descriptor.path, path_matcher))
        .map(|descriptor| {
            let score = score_file(&descriptor, tokens);
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
            results,
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
}
