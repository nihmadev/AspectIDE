use std::{path::Path, time::Instant};

use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use lux_core::{AppResult, SearchHit, SearchOptions, SearchResponse};
use rayon::prelude::*;
use regex::{Regex, RegexBuilder};

const SEARCH_PREVIEW_MAX_CHARS: usize = 240;
const SEARCH_PREVIEW_CONTEXT_BEFORE_CHARS: usize = 80;

pub fn query(
    root: impl AsRef<Path>,
    search: String,
    options: SearchOptions,
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
    let matcher = SearchMatcher::new(&search, &options)?;

    let mut builder = WalkBuilder::new(&root);
    builder.hidden(!options.include_hidden);
    builder.git_ignore(true).git_exclude(true).parents(true);

    let files: Vec<_> = builder
        .build()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        })
        .map(|entry| entry.into_path())
        .filter(|path| {
            matches_glob_filters(&root, path, include_globs.as_ref(), exclude_globs.as_ref())
        })
        .collect();

    let mut hits: Vec<SearchHit> = files
        .par_iter()
        .flat_map_iter(|path| match std::fs::read_to_string(path) {
            Ok(text) => collect_hits(path, &text, &matcher),
            Err(_) => Vec::new(),
        })
        .collect();

    hits.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.column.cmp(&right.column))
    });
    let truncated = hits.len() > options.max_results;
    hits.truncate(options.max_results);

    Ok(SearchResponse {
        query: search,
        hits,
        truncated,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

enum SearchMatcher {
    Literal { needle: String },
    Regex(Regex),
}

impl SearchMatcher {
    fn new(search: &str, options: &SearchOptions) -> AppResult<Self> {
        if options.use_regex {
            let pattern = if options.whole_word {
                format!(r"\b(?:{})\b", search)
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
    fn end(&self) -> usize {
        self.start + self.length
    }
}

struct SearchPreview {
    text: String,
    match_start: usize,
    match_length: usize,
}

fn collect_hits(path: &Path, text: &str, matcher: &SearchMatcher) -> Vec<SearchHit> {
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
        .map(|(index, _)| index)
        .unwrap_or(text.len())
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
    include_globs.map_or(true, |globs| globs.is_match(relative))
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
            SearchOptions {
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
            SearchOptions {
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
            SearchOptions {
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
            SearchOptions {
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

    fn test_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("lux-search-test-{}-{suffix}", std::process::id()))
    }
}
