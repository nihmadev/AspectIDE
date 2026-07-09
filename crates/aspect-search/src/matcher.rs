use std::path::Path;

use aspect_core::SearchHit;
use regex::{Regex, RegexBuilder};
use aspect_core::AppResult;
use aspect_core::SearchOptions;

const SEARCH_PREVIEW_MAX_CHARS: usize = 240;
const SEARCH_PREVIEW_CONTEXT_BEFORE_CHARS: usize = 80;

pub enum SearchMatcher {
    Literal { needle: String },
    Regex(Regex),
}

impl SearchMatcher {
    pub fn new(search: &str, options: &SearchOptions) -> AppResult<Self> {
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

    pub fn matches_in_line(&self, line: &str) -> Vec<SearchLineMatch> {
        match self {
            Self::Regex(regex) => regex
                .find_iter(line)
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

pub struct SearchLineMatch {
    pub start: usize,
    pub length: usize,
    pub text: String,
}

impl SearchLineMatch {
    pub const fn end(&self) -> usize {
        self.start + self.length
    }
}

pub struct SearchPreview {
    pub text: String,
    pub match_start: usize,
    pub match_length: usize,
}

pub fn collect_hits(path: &Path, text: &str, matcher: &SearchMatcher, limit: usize) -> Vec<SearchHit> {
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
