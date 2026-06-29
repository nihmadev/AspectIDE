#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::{WalkBuilder, WalkState};
use lux_core::{scan_threads, AppResult, SearchHit, SearchOptions, SearchResponse};
use rayon::prelude::*;
use regex::{Regex, RegexBuilder};

const SEARCH_PREVIEW_MAX_CHARS: usize = 240;
const SEARCH_PREVIEW_CONTEXT_BEFORE_CHARS: usize = 80;
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
// Path fragments that mark generated/vendored output: real source ranks above
// these so relevance-blind alphabetical order can't bury a true match.
const LOW_VALUE_PATH_FRAGMENTS: &[&str] = &[
    "node_modules",
    "/target/",
    "/dist/",
    "/build/",
    "/out/",
    "/vendor/",
    "/.next/",
    ".min.",
    ".lock",
    "generated",
];

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

    // Parallel file discovery: worker threads collect candidate paths (after glob
    // filtering) into a shared buffer, fanning directory traversal across cores.
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

    // Rank by relevance BEFORE truncation so alphabetically-early files can't
    // displace exact filename/word matches or real source over generated paths.
    // Path/line/column remain the stable tie-breakers.
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

/// Match `files` in parallel under a global hit budget, returning the collected
/// hits and whether more matches were dropped (so `truncated` stays accurate even
/// when the returned set isn't itself over `max_results`).
///
/// The budget caps collection at a small multiple of `max_results` (hard-capped
/// by [`MAX_GLOBAL_HITS`]) so a broad literal or zero-width/common regex can't
/// collect roughly `files × (max_results + 1)` hits and stall a large workspace.
/// The pool is sized to the scan budget to honor the "reserve a core for the UI"
/// policy rather than grabbing every core via rayon's default global pool.
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
        // Per-file cap (`max_results + 1`) bounds a single pathological file; the
        // global budget bounds the workspace-wide total.
        let per_file_limit = options.max_results.saturating_add(1);
        files
            .par_iter()
            .flat_map_iter(|path| {
                let already = collected.load(Ordering::Relaxed);
                if already >= global_budget {
                    // Budget met: at least this file's matches go uncollected.
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
                // A file that fills its cap signals there were more matches than we
                // return, so the result is genuinely truncated.
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

enum SearchMatcher {
    Literal { needle: String },
    Regex(Regex),
}

impl SearchMatcher {
    fn new(search: &str, options: &SearchOptions) -> AppResult<Self> {
        if options.use_regex {
            let pattern = if options.whole_word {
                format!(r"\b(?:{search})\b")
            } else {
                search.to_string()
            };
            let regex = RegexBuilder::new(&pattern)
                .case_insensitive(!options.case_sensitive)
                .build()?;
            return Ok(Self::Regex(regex));
        }

        if options.whole_word || !options.case_sensitive {
            let pattern = if options.whole_word {
                format!(r"\b{}\b", regex::escape(search))
            } else {
                regex::escape(search)
            };
            let regex = RegexBuilder::new(&pattern)
                .case_insensitive(!options.case_sensitive)
                .build()?;
            return Ok(Self::Regex(regex));
        }

        Ok(Self::Literal {
            needle: search.to_string(),
        })
    }

    fn matches_in_line(&self, line: &str) -> Vec<SearchLineMatch> {
        match self {
            Self::Regex(regex) => regex
                .find_iter(line)
                // Drop zero-width matches (`^`, `$`, lookarounds): they carry no
                // highlightable text and a pattern like `^` would otherwise emit a
                // hit on every line, exploding the global result set.
                .filter(|hit| hit.end() > hit.start())
                .map(|hit| SearchLineMatch {
                    start: hit.start(),
                    length: hit.end() - hit.start(),
                    text: hit.as_str().to_string(),
                })
                .collect(),
            Self::Literal { needle } => literal_matches(line, needle),
        }
    }
}

struct SearchLineMatch {
    start: usize,
    length: usize,
    text: String,
}

impl SearchLineMatch {
    const fn end(&self) -> usize {
        self.start + self.length
    }
}

struct SearchPreview {
    text: String,
    match_start: usize,
    match_length: usize,
}

fn collect_hits(path: &Path, text: &str, matcher: &SearchMatcher, limit: usize) -> Vec<SearchHit> {
    text.lines()
        .enumerate()
        .flat_map(|(line_index, line)| {
            matcher
                .matches_in_line(line)
                .into_iter()
                .map(move |line_match| {
                    let preview = preview_for_line(line, &line_match);
                    SearchHit {
                        path: path.to_path_buf(),
                        line: line_index + 1,
                        column: utf16_column_for_byte_index(line, line_match.start),
                        match_length: utf16_len(&line[line_match.start..line_match.end()]),
                        match_text: line_match.text,
                        preview: preview.text,
                        preview_match_start: preview.match_start,
                        preview_match_length: preview.match_length,
                    }
                })
        })
        .take(limit)
        .collect()
}

fn utf16_column_for_byte_index(line: &str, byte_index: usize) -> usize {
    utf16_len(&line[..byte_index.min(line.len())]) + 1
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn preview_for_line(line: &str, line_match: &SearchLineMatch) -> SearchPreview {
    let leading_whitespace = line.len() - line.trim_start().len();
    let base_start = leading_whitespace.min(line_match.start);
    let chars_before_match = line[base_start..line_match.start].chars().count();
    let (prefix, preview_start) = if chars_before_match > SEARCH_PREVIEW_CONTEXT_BEFORE_CHARS {
        let skipped_chars = chars_before_match - SEARCH_PREVIEW_CONTEXT_BEFORE_CHARS;
        (
            "...",
            base_start + byte_index_after_n_chars(&line[base_start..], skipped_chars),
        )
    } else {
        ("", base_start)
    };

    let max_body_chars = SEARCH_PREVIEW_MAX_CHARS.saturating_sub(prefix.chars().count());
    let body = line[preview_start..]
        .chars()
        .take(max_body_chars)
        .collect::<String>();
    SearchPreview {
        text: format!("{prefix}{body}"),
        match_start: utf16_len(prefix) + utf16_len(&line[preview_start..line_match.start]),
        match_length: utf16_len(&line[line_match.start..line_match.end()]),
    }
}

fn byte_index_after_n_chars(text: &str, count: usize) -> usize {
    if count == 0 {
        return 0;
    }

    text.char_indices()
        .nth(count)
        .map_or(text.len(), |(index, _)| index)
}

fn literal_matches(line: &str, needle: &str) -> Vec<SearchLineMatch> {
    if needle.is_empty() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    let mut search_start = 0;
    while search_start <= line.len() {
        let Some(relative_start) = line[search_start..].find(needle) else {
            break;
        };
        let start = search_start + relative_start;
        let end = start + needle.len();
        matches.push(SearchLineMatch {
            start,
            length: needle.len(),
            text: line[start..end].to_string(),
        });
        search_start = end.max(start + 1);
    }
    matches
}

/// Relevance score for ranking before truncation. Higher ranks earlier. Combines
/// filename/path hits, exact word-boundary matches, source-over-generated bias,
/// shorter paths, and a small line-position nudge. Path/line/column tie-break.
fn relevance_score(hit: &SearchHit, lower_query: &str) -> i64 {
    let mut score = 0_i64;
    // Normalize separators to '/' so the (forward-slash) low-value fragments match
    // on Windows back-slash paths too.
    let path = hit.path.to_string_lossy().to_lowercase().replace('\\', "/");

    // The query appears in the file name → very likely what the user wants.
    if let Some(file_name) = hit.path.file_name().and_then(|name| name.to_str()) {
        let file_name = file_name.to_lowercase();
        if file_name == lower_query {
            score += 1_000;
        } else if file_name.contains(lower_query) {
            score += 400;
        }
    }
    // The query also appears elsewhere in the path.
    if path.contains(lower_query) {
        score += 80;
    }
    // Exact whole-word match in the line beats an in-word substring match.
    if is_word_boundary_match(&hit.match_text.to_lowercase(), lower_query) {
        score += 120;
    }
    // Real source ranks above generated/vendored output.
    if is_low_value_path(&path) {
        score -= 300;
    }
    // Prefer shorter paths (closer to the root) and earlier matches, lightly.
    // These penalties are tiny and bounded, so a saturating cast is exact here.
    score -= i64::try_from(path.len() / 16).unwrap_or(i64::MAX);
    score -= i64::try_from(hit.line.min(10_000) / 200).unwrap_or(i64::MAX);
    score
}

/// Whether `text` equals `query` on word boundaries (a real token match, not an
/// in-word substring like `alpha` inside `alphabet`).
fn is_word_boundary_match(text: &str, query: &str) -> bool {
    text == query
}

/// Whether `path` looks generated/vendored and should rank below real source.
fn is_low_value_path(path: &str) -> bool {
    LOW_VALUE_PATH_FRAGMENTS
        .iter()
        .any(|fragment| path.contains(fragment))
}

fn compile_globs(patterns: &[String]) -> AppResult<Option<GlobSet>> {
    let mut builder = GlobSetBuilder::new();
    let mut has_patterns = false;
    for pattern in patterns
        .iter()
        .map(|pattern| pattern.trim())
        .filter(|pattern| !pattern.is_empty())
    {
        builder.add(Glob::new(pattern)?);
        if !pattern.contains('/') && !pattern.contains('\\') {
            builder.add(Glob::new(&format!("**/{pattern}"))?);
        }
        has_patterns = true;
    }
    Ok(if has_patterns {
        Some(builder.build()?)
    } else {
        None
    })
}

fn matches_glob_filters(
    root: &Path,
    path: &Path,
    include_globs: Option<&GlobSet>,
    exclude_globs: Option<&GlobSet>,
) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    if exclude_globs.is_some_and(|globs| globs.is_match(relative)) {
        return false;
    }
    include_globs.is_none_or(|globs| globs.is_match(relative))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use lux_core::SearchOptions;

    use super::query;

    #[test]
    fn query_supports_case_regex_whole_word_and_glob_filters() {
        let root = test_root();
        fs::create_dir_all(root.join("src")).expect("src should be created");
        fs::create_dir_all(root.join("target")).expect("target should be created");
        fs::write(
            root.join("src/main.rs"),
            "let alpha = 1;\nlet alphabet = 2;\nlet Beta = alpha;\n",
        )
        .expect("main should be written");
        fs::write(root.join("src/readme.md"), "alpha docs\n").expect("readme should be written");
        fs::write(root.join("target/generated.rs"), "alpha generated\n")
            .expect("generated should be written");

        let whole_word = query(
            &root,
            "alpha".to_string(),
            &SearchOptions {
                whole_word: true,
                include_globs: vec!["*.rs".to_string()],
                exclude_globs: vec!["target/**".to_string()],
                max_results: 20,
                ..SearchOptions::default()
            },
        )
        .expect("whole-word search should work");
        assert_eq!(whole_word.hits.len(), 2);
        assert!(whole_word
            .hits
            .iter()
            .all(|hit| hit.path.ends_with("main.rs")));
        assert!(whole_word
            .hits
            .iter()
            .all(|hit| hit.preview != "let alphabet = 2;"));
        assert!(whole_word.hits.iter().all(|hit| hit.match_text == "alpha"));
        assert!(whole_word
            .hits
            .iter()
            .all(|hit| hit.preview_match_length == 5));

        let regex = query(
            &root,
            "b.ta".to_string(),
            &SearchOptions {
                use_regex: true,
                case_sensitive: false,
                max_results: 20,
                ..SearchOptions::default()
            },
        )
        .expect("regex search should work");
        assert_eq!(regex.hits.len(), 1);
        assert_eq!(regex.hits[0].line, 3);

        let case_sensitive = query(
            &root,
            "beta".to_string(),
            &SearchOptions {
                case_sensitive: true,
                max_results: 20,
                ..SearchOptions::default()
            },
        )
        .expect("case-sensitive search should work");
        assert!(case_sensitive.hits.is_empty());

        fs::write(root.join("src/unicode.rs"), "let мир = \"мир\";\n")
            .expect("unicode should be written");
        let unicode = query(
            &root,
            "мир".to_string(),
            &SearchOptions {
                case_sensitive: true,
                include_globs: vec!["unicode.rs".to_string()],
                max_results: 20,
                ..SearchOptions::default()
            },
        )
        .expect("unicode search should work");
        assert_eq!(unicode.hits.len(), 2);
        assert_eq!(unicode.hits[0].column, 5);
        assert_eq!(unicode.hits[0].match_length, 3);
        assert_eq!(unicode.hits[0].preview_match_start, 4);
        assert_eq!(unicode.hits[0].preview_match_length, 3);
        assert_eq!(unicode.hits[0].match_text, "мир");

        fs::remove_dir_all(root).expect("test root should be removed");
    }

    #[test]
    fn ranks_filename_and_source_matches_above_generated() {
        let root = test_root();
        fs::create_dir_all(root.join("src")).expect("src dir");
        fs::create_dir_all(root.join("dist")).expect("dist dir");
        // A file literally named after the query — should rank first.
        fs::write(root.join("src/widget.rs"), "// widget impl\n").expect("widget");
        // A generated/vendored hit — should rank last despite alphabetical order.
        fs::write(root.join("dist/bundle.js"), "var widget = 1;\n").expect("bundle");
        // A plain source mention.
        fs::write(root.join("src/app.rs"), "let widget = make();\n").expect("app");

        let result = query(
            &root,
            "widget".to_string(),
            &SearchOptions {
                max_results: 20,
                ..SearchOptions::default()
            },
        )
        .expect("search should work");

        let first = &result.hits[0];
        assert!(
            first.path.ends_with("widget.rs"),
            "filename match should rank first, got {:?}",
            first.path
        );
        let last = result.hits.last().expect("at least one hit");
        assert!(
            last.path.to_string_lossy().contains("dist"),
            "generated path should rank last, got {:?}",
            last.path
        );

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn zero_width_regex_matches_are_dropped() {
        let root = test_root();
        fs::create_dir_all(&root).ok();
        fs::write(root.join("a.txt"), "one\ntwo\nthree\n").expect("write");

        // `^` matches the start of every line (zero-width); it must yield no hits
        // rather than one-per-line flooding the result set.
        let result = query(
            &root,
            "^".to_string(),
            &SearchOptions {
                use_regex: true,
                max_results: 100,
                ..SearchOptions::default()
            },
        )
        .expect("search should work");
        assert!(
            result.hits.is_empty(),
            "zero-width matches must not produce hits, got {}",
            result.hits.len()
        );

        fs::remove_dir_all(root).expect("cleanup");
    }

    fn test_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("lux-search-test-{}-{suffix}", std::process::id()))
    }
}
