//! Text utilities for the reranking pipeline: host extraction, authority signal,
//! tokenization, character/word-boundary trimming.

use std::collections::{HashMap, HashSet};

/// Cap on distinct query terms actually scored.
const MAX_QUERY_TERMS: usize = 64;

/// Registrable-ish host of a URL.
#[must_use]
pub(crate) fn host_of(url: &str) -> String {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = if host.starts_with('[') {
        host.split(']').next().map_or(host, |h| &h[1..])
    } else {
        host.split(':').next().unwrap_or(host)
    };
    host.to_ascii_lowercase()
}

/// A small, capped authority/junk signal so official docs and known-good hosts
/// edge out content farms at equal lexical relevance.
pub(crate) fn authority_signal(url: &str) -> f64 {
    const TRUSTED: &[&str] = &[
        "docs.rs",
        "doc.rust-lang.org",
        "rust-lang.org",
        "developer.mozilla.org",
        "wikipedia.org",
        "github.com",
        "stackoverflow.com",
        "arxiv.org",
        "docs.python.org",
        "python.org",
        "nodejs.org",
        "go.dev",
        "learn.microsoft.com",
        "developer.apple.com",
        "kubernetes.io",
        "postgresql.org",
        "ietf.org",
        "rfc-editor.org",
        "w3.org",
        "whatwg.org",
        "crates.io",
        "npmjs.com",
        "pypi.org",
    ];
    const JUNK: &[&str] = &[
        "pinterest.com",
        "scribd.com",
        "slideshare.net",
        "coursehero.com",
        "chegg.com",
        "quizlet.com",
        "brainly.com",
        "studocu.com",
    ];
    let host = host_of(url);
    let in_list = |list: &[&str]| {
        list.iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
    };

    let mut signal: f64 = 0.0;
    if url.starts_with("https://") {
        signal += 0.01;
    }
    if in_list(TRUSTED) {
        signal += 0.04;
    }
    if in_list(JUNK) {
        signal -= 0.08;
    }
    let path = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.find('/').map(|slash| &rest[slash..]))
        .unwrap_or("");
    let path_lower = path.to_ascii_lowercase();
    if ["/docs", "/documentation", "/reference", "/manual", "/rfc"]
        .iter()
        .any(|prefix| path_lower.starts_with(prefix))
    {
        signal += 0.02;
    }
    signal.clamp(-0.08, 0.06)
}

/// Take at most `max_chars` characters from `text` on a char boundary.
pub(crate) fn take_chars(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((byte_index, _)) => &text[..byte_index],
        None => text,
    }
}

/// Count occurrences of each ≥2-char alphanumeric token in one pass.
pub(crate) fn term_counts(text: &str) -> HashMap<&str, usize> {
    let mut counts = HashMap::new();
    for token in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
    {
        *counts.entry(token).or_insert(0) += 1;
    }
    counts
}

/// Tokenize query text into unique lowercased ≥2-char alphanumeric tokens.
pub(crate) fn tokenize_unique(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
        .map(str::to_string)
        .take(MAX_QUERY_TERMS)
        .collect()
}

/// Trim to at most `max_chars` on a word boundary, appending an ellipsis when truncated.
pub(crate) fn trim_to_chars(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut end = 0;
    for (count, (byte_index, _)) in trimmed.char_indices().enumerate() {
        if count >= max_chars {
            break;
        }
        end = byte_index;
    }
    let cut = trimmed[..end].rfind(char::is_whitespace).unwrap_or(end);
    format!("{}…", trimmed[..cut].trim_end())
}
