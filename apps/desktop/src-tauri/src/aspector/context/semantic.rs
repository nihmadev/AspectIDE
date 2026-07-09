use aspect_core::SearchOptions;
use tauri::State;

use crate::{workspace_root, SharedState};

// Compatibility re-exports for existing callers (workspace, related, sources, checkpoint)
pub use aspect_search::path::normalize_slashes as normalize_slashes_pub;
pub use aspect_search::path::file_extension as file_extension_pub;
pub use aspect_search::path::family_stem as family_stem_pub;
pub use aspect_search::tokenize::tokenize as tokenize_pub;
pub use aspect_search::path::is_low_signal_path as is_low_signal_path_pub;
pub use aspect_search::path::score_path as score_path_pub;
pub use aspect_search::path::language_for_path as language_for_path_pub;

fn compute_path_filter(path: Option<String>) -> Option<String> {
    path.map(|p| {
        let normalized = p.trim().replace('\\', "/");
        normalized.to_lowercase()
    })
    .filter(|p| !p.is_empty())
}

#[tauri::command]
pub async fn ai_semantic_search(
    state: State<'_, SharedState>,
    query: String,
    path: Option<String>,
    max_results: Option<usize>,
    max_files: Option<usize>,
) -> Result<aspect_search::types::AiSemanticSearchResponse, String> {
    let root = workspace_root(&state)?;
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err("SemanticSearch requires a non-empty query.".to_string());
    }
    let max_results = max_results.unwrap_or(24).clamp(1, 80);
    let path_filter = compute_path_filter(path);
    let file_cap = max_files.unwrap_or(5_000).clamp(500, 20_000);
    let search_max = (max_results * 4).clamp(40, 120);

    let root_str = aspect_search::path::normalize_slashes(&root.to_string_lossy());

    // 1. LSP workspace symbols
    let symbols_future = async {
        let mut lsp = state.lsp.lock().await;
        if let Some(manager) = lsp.as_mut() {
            if let Ok(mut found) = manager.workspace_symbols(query.clone()).await {
                found.truncate(max_results.max(40));
                return found;
            }
        }
        Vec::<aspect_core::LspWorkspaceSymbol>::new()
    };

    // 2. Indexed text search
    let search_future = {
        let root = root.clone();
        let q = query.clone();
        let options = SearchOptions {
            case_sensitive: false,
            whole_word: false,
            use_regex: false,
            include_hidden: false,
            include_globs: Vec::new(),
            exclude_globs: vec![
                "node_modules/**".to_string(),
                "target/**".to_string(),
                "dist/**".to_string(),
                "build/**".to_string(),
                ".git/**".to_string(),
                "vendor/**".to_string(),
                "venv/**".to_string(),
                ".venv/**".to_string(),
                "coverage/**".to_string(),
            ],
            max_results: search_max,
        };
        tokio::task::spawn_blocking(move || aspect_search::query(root, q, &options))
    };

    // 3. Workspace file candidates
    let files_future = {
        let root = root.clone();
        tokio::task::spawn_blocking(move || aspect_fs::list_files_scanned(root, file_cap))
    };

    let (symbols, search_result, files_result) =
        tokio::join!(symbols_future, search_future, files_future);

    let mut partial_reasons: Vec<&'static str> = Vec::new();
    let search_hits = search_result
        .map_err(|error| error.to_string())?
        .map_or_else(
            |error| {
                tracing::warn!(%error, "ai_semantic_search: indexed search backend failed");
                partial_reasons.push("content-search backend failed; results omit text matches");
                Vec::default()
            },
            |response| response.hits,
        );

    let listing = files_result.map_err(|error| error.to_string())?;
    let truncated = listing.truncated;
    let files = listing.entries;

    let mut response = aspect_search::orchestrate::build_ranked_results(
        query,
        path_filter,
        root_str,
        max_results,
        &symbols,
        search_hits,
        &files,
        truncated,
        !partial_reasons.is_empty(),
        partial_reasons.into_iter().map(str::to_string).collect(),
    );

    // Best-effort structural boost: lift results that map to a well-connected
    // code-graph node.
    {
        let guard = state.code_graph.lock().await;
        if let Some(index) = guard.as_ref() {
            aspect_search::graph::apply_graph_boost(index.graph(), &mut response.results);
            response
                .results
                .sort_by(|a, b| {
                    b.score
                        .cmp(&a.score)
                        .then_with(|| a.path.cmp(&b.path))
                        .then_with(|| a.line.unwrap_or(0).cmp(&b.line.unwrap_or(0)))
                });
            response.results.truncate(max_results);
            response.count = response.results.len();
        }
    }

    Ok(response)
}
