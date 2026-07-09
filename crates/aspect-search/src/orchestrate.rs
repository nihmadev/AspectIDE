use std::collections::BTreeMap;
use std::path::PathBuf;

use aspect_core::FsEntry;

use crate::filter::PathFilter;
use crate::scoring::{collect_file_results, collect_search_results, collect_symbol_results};
use crate::tokenize::tokenize;
use crate::types::{AiSemanticResult, AiSemanticSearchResponse};

pub fn build_ranked_results(
    query: String,
    path_filter_str: Option<String>,
    root_str: String,
    max_results: usize,
    symbols: &[aspect_core::LspWorkspaceSymbol],
    search_hits: Vec<aspect_core::SearchHit>,
    files: &[FsEntry],
    truncated: bool,
    partial: bool,
    partial_reasons: Vec<String>,
) -> AiSemanticSearchResponse {
    let normalized_query = query.to_lowercase();
    let tokens = tokenize(&query);
    let path_matcher = path_filter_str
        .as_deref()
        .map(PathFilter::new);

    let mut results: BTreeMap<String, AiSemanticResult> = BTreeMap::new();

    collect_symbol_results(
        &mut results,
        symbols,
        &root_str,
        &normalized_query,
        &tokens,
        path_matcher.as_ref(),
    );

    collect_search_results(
        &mut results,
        &search_hits,
        &root_str,
        &tokens,
        path_matcher.as_ref(),
    );

    let file_limit = (max_results * 2).min(80);
    collect_file_results(
        &mut results,
        files,
        &root_str,
        &tokens,
        path_matcher.as_ref(),
        file_limit,
    );

    let mut ranked: Vec<AiSemanticResult> = results.into_values().collect();
    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.unwrap_or(0).cmp(&b.line.unwrap_or(0)))
    });
    ranked.truncate(max_results);

    AiSemanticSearchResponse {
        workspace_root: PathBuf::from(&root_str),
        query,
        path_filter: path_filter_str,
        count: ranked.len(),
        truncated,
        partial,
        partial_reasons,
        results: ranked,
    }
}
