#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

pub(crate) mod glob;
pub(crate) mod matcher;
pub(crate) mod rank;

pub mod path;
pub mod tokenize;
pub mod classify;
pub mod filter;
pub mod types;
pub mod scoring;
pub mod graph;
pub mod orchestrate;

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

use aspect_core::{scan_threads, AppResult, SearchHit, SearchOptions, SearchResponse};
use ignore::{WalkBuilder, WalkState};
use rayon::prelude::*;

use crate::glob::{compile_globs, matches_glob_filters};
use crate::matcher::{collect_hits, SearchMatcher};
use crate::rank::relevance_score;

// Skip files larger than this cap so a handful of multi-GB files can't be read
// fully into memory at once across the worker pool (matches ripgrep's default).
const MAX_SEARCH_FILE_BYTES: u64 = 8 * 1024 * 1024;
// How many candidates beyond `max_results` to collect so ranking has room to
// float the most relevant hits up before truncation. Bounds the global hit set
// to a small multiple of what we return rather than `files × (max_results + 1)`.
const RANK_OVERSCAN: usize = 20;
// Absolute ceiling on collected hits regardless of `max_results`, so a broad
// literal or common/zero-width regex over a large workspace can't keep search
// and the AI search tool busy/memory-heavy collecting millions of hits.
const MAX_GLOBAL_HITS: usize = 50_000;

pub fn query(
    root: impl AsRef<Path>,
    search: String,
    options: &SearchOptions,
) -> AppResult<SearchResponse> {
    let started = Instant::now();
    if search.trim().is_empty() {
        return Ok(SearchResponse {
            query: search,
            hits: Vec::new(),
            truncated: false,
            elapsed_ms: started.elapsed().as_millis(),
        });
    }

    let root = root.as_ref().to_path_buf();
    let include_globs = compile_globs(&options.include_globs)?;
    let exclude_globs = compile_globs(&options.exclude_globs)?;
    let matcher = SearchMatcher::new(&search, options)?;

    let threads = scan_threads();

    let mut builder = WalkBuilder::new(&root);
    builder.hidden(!options.include_hidden);
    builder
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .threads(threads);

    let collected: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    builder.build_parallel().run(|| {
        let collected = Arc::clone(&collected);
        let root = root.clone();
        let include = include_globs.clone();
        let exclude = exclude_globs.clone();
        Box::new(move |result| {
            let Ok(entry) = result else {
                return WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                return WalkState::Continue;
            }
            let path = entry.into_path();
            if matches_glob_filters(&root, &path, include.as_ref(), exclude.as_ref()) {
                if let Ok(mut buffer) = collected.lock() {
                    buffer.push(path);
                }
            }
            WalkState::Continue
        })
    });
    let files = Arc::try_unwrap(collected)
        .ok()
        .and_then(|mutex| mutex.into_inner().ok())
        .unwrap_or_default();

    let (mut hits, more_available) = match_files_bounded(&files, &matcher, options, threads);

    let lower_query = search.to_lowercase();
    hits.sort_by_cached_key(|hit| {
        (
            std::cmp::Reverse(relevance_score(hit, &lower_query)),
            hit.path.clone(),
            hit.line,
            hit.column,
        )
    });
    let truncated = more_available || hits.len() > options.max_results;
    hits.truncate(options.max_results);

    Ok(SearchResponse {
        query: search,
        hits,
        truncated,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn match_files_bounded(
    files: &[PathBuf],
    matcher: &SearchMatcher,
    options: &SearchOptions,
    threads: usize,
) -> (Vec<SearchHit>, bool) {
    let global_budget = options
        .max_results
        .saturating_mul(RANK_OVERSCAN)
        .clamp(options.max_results.max(1), MAX_GLOBAL_HITS);
    let collected = Arc::new(AtomicUsize::new(0));
    let more_available = Arc::new(AtomicBool::new(false));

    let run = |files: &[PathBuf]| -> Vec<SearchHit> {
        let per_file_limit = options.max_results.saturating_add(1);
        files
            .par_iter()
            .flat_map_iter(|path| {
                let already = collected.load(Ordering::Relaxed);
                if already >= global_budget {
                    more_available.store(true, Ordering::Relaxed);
                    return Vec::new();
                }
                if std::fs::metadata(path).is_ok_and(|meta| meta.len() > MAX_SEARCH_FILE_BYTES) {
                    return Vec::new();
                }
                let file_limit = per_file_limit.min(global_budget - already);
                let file_hits = std::fs::read_to_string(path).map_or_else(
                    |_| Vec::new(),
                    |text| collect_hits(path, &text, matcher, file_limit),
                );
                if file_hits.len() >= per_file_limit {
                    more_available.store(true, Ordering::Relaxed);
                }
                collected.fetch_add(file_hits.len(), Ordering::Relaxed);
                file_hits
            })
            .collect()
    };

    let hits = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_or_else(|_| run(files), |pool| pool.install(|| run(files)));
    (hits, more_available.load(Ordering::Relaxed))
}

