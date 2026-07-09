//! Search URL builders + URL utilities (canonical dedup key, query expansion,
//! focus bias, follow-up link extraction, SSRF-safe href resolution).

use crate::model::FocusMode;
use super::{percent_encode, validate_source_url, extract_attr};

/// Build a SearxNG JSON search URL for `query` under `focus`.
#[must_use]
pub fn searxng_search_url(base: &str, query: &str, focus: FocusMode) -> String {
    let base = base.trim().trim_end_matches('/');
    format!(
        "{base}/search?q={}&format=json&safesearch=0&categories={}",
        percent_encode(query),
        percent_encode(focus.searxng_category()),
    )
}

/// Build a keyless DuckDuckGo HTML search URL for `query` (full results page).
#[must_use]
pub fn duckduckgo_search_url(query: &str) -> String {
    format!(
        "https://html.duckduckgo.com/html/?q={}",
        percent_encode(query)
    )
}

/// Build the DuckDuckGo *lite* search URL — a minimal, JS-free results table.
#[must_use]
pub fn duckduckgo_lite_search_url(query: &str) -> String {
    format!(
        "https://lite.duckduckgo.com/lite/?q={}",
        percent_encode(query)
    )
}

/// Build a keyless Brave Search HTML URL for `query` (secondary fallback engine).
#[must_use]
pub fn brave_search_url(query: &str) -> String {
    format!(
        "https://search.brave.com/search?q={}&source=web",
        percent_encode(query)
    )
}

/// Build a keyless Mojeek HTML search URL for `query` (tertiary fallback engine).
#[must_use]
pub fn mojeek_search_url(query: &str) -> String {
    format!("https://www.mojeek.com/search?q={}", percent_encode(query))
}

// ── dedup / tracking ──

/// Canonical dedup key for a URL: lowercased scheme+host (with `www.` and the
/// default port stripped), fragment removed, tracking parameters filtered, and
/// the trailing slash normalized.
#[must_use]
pub fn canonical_url_key(url: &str) -> String {
    let url = url.trim();
    let without_fragment = url.split('#').next().unwrap_or(url);
    let Some(scheme_end) = without_fragment.find("://") else {
        return without_fragment.to_string();
    };
    let scheme = without_fragment[..scheme_end].to_ascii_lowercase();
    let rest = &without_fragment[scheme_end + 3..];
    let (authority, path_query) = rest
        .find(['/', '?'])
        .map_or((rest, ""), |split| (&rest[..split], &rest[split..]));

    let mut host = authority.to_ascii_lowercase();
    let default_port = if scheme == "https" { ":443" } else { ":80" };
    if let Some(stripped) = host.strip_suffix(default_port) {
        host = stripped.to_string();
    }
    let host = host.strip_prefix("www.").unwrap_or(&host);

    let (path, query) = path_query.find('?').map_or((path_query, ""), |split| {
        (&path_query[..split], &path_query[split + 1..])
    });
    let path = path.trim_end_matches('/');
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| !pair.is_empty() && !is_tracking_param(pair))
        .collect();

    let mut key = format!("{scheme}://{host}{path}");
    if !kept.is_empty() {
        key.push('?');
        key.push_str(&kept.join("&"));
    }
    key
}

/// Whether a `name=value` query pair is pure tracking decoration.
fn is_tracking_param(pair: &str) -> bool {
    let name = pair.split('=').next().unwrap_or(pair).to_ascii_lowercase();
    name.starts_with("utm_")
        || matches!(
            name.as_str(),
            "fbclid"
                | "gclid"
                | "msclkid"
                | "yclid"
                | "igshid"
                | "mc_cid"
                | "mc_eid"
                | "ref"
                | "ref_src"
                | "si"
                | "spm"
        )
}

// ── focus bias ──

/// DuckDuckGo has no category verticals, so a non-web [`FocusMode`] is
/// approximated by biasing the query text toward the requested source kind.
#[must_use]
pub fn focus_biased_query(query: &str, focus: FocusMode) -> Option<String> {
    let suffix = match focus {
        FocusMode::Web => return None,
        FocusMode::Academic => "research paper",
        FocusMode::News => "news",
        FocusMode::Social => "forum discussion",
        FocusMode::Video => "video",
        FocusMode::Code => "documentation",
    };
    let lower = query.to_lowercase();
    if suffix.split_whitespace().all(|word| lower.contains(word)) {
        return None;
    }
    Some(format!("{} {suffix}", query.trim()))
}

// ── query expansion ──

/// Very small English stop-word set for building the keyword-only query variant.
const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "to", "in", "on", "for", "with", "how", "what", "is",
    "are", "do", "does", "can", "vs", "my", "i", "me",
];

/// Expand one query into up to `max` mechanical variants for deep research.
#[must_use]
pub fn expand_queries(query: &str, focus: FocusMode, max: usize) -> Vec<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() || max <= 1 {
        return vec![trimmed.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let push =
        |candidate: String, out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
            let candidate = candidate.trim().to_string();
            if !candidate.is_empty() && seen.insert(candidate.to_lowercase()) {
                out.push(candidate);
            }
        };

    push(trimmed.to_string(), &mut out, &mut seen);

    let word_count = trimmed.split_whitespace().count();
    if word_count >= 2 {
        push(format!("\"{trimmed}\""), &mut out, &mut seen);
    }

    let suffix = match focus {
        FocusMode::Code => "documentation",
        FocusMode::Academic => "paper pdf",
        FocusMode::News => "latest 2026",
        FocusMode::Social => "discussion forum",
        FocusMode::Video => "video tutorial",
        FocusMode::Web => "guide",
    };
    push(format!("{trimmed} {suffix}"), &mut out, &mut seen);

    let keywords: Vec<&str> = trimmed
        .split_whitespace()
        .filter(|word| {
            let lower = word
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            !lower.is_empty() && !STOP_WORDS.contains(&lower.as_str())
        })
        .collect();
    if keywords.len() >= 2 && keywords.len() < word_count {
        push(keywords.join(" "), &mut out, &mut seen);
    }

    out.truncate(max.max(1));
    out
}

// ── follow-up link extraction ──

/// Extract candidate follow-up links from a fetched result page's raw HTML for the
/// deep-mode 1-hop crawl.
#[must_use]
pub fn extract_result_links(html: &str, base_url: &str, query: &str, max: usize) -> Vec<String> {
    if max == 0 {
        return Vec::new();
    }
    let query_terms: std::collections::HashSet<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 3)
        .map(str::to_string)
        .collect();
    let origin = origin_of(base_url);
    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut cursor = 0usize;
    while let Some(rel) = html[cursor..].find("<a ") {
        let tag_start = cursor + rel;
        let Some(rel_gt) = html[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + rel_gt;
        let tag = &html[tag_start..tag_end];
        cursor = tag_end + 1;
        let text = html[cursor..]
            .find("</a>")
            .map_or("", |rel_close| &html[cursor..cursor + rel_close]);
        let Some(href) = extract_attr(tag, "href") else {
            continue;
        };
        let text_lower = strip_tags_impl(text).to_lowercase();
        let on_topic = query_terms.is_empty()
            || query_terms.iter().any(|term| text_lower.contains(term.as_str()));
        if !on_topic {
            continue;
        }
        let absolute = resolve_href(&href, &origin);
        if let Some(url) = validate_source_url(&absolute) {
            if seen.insert(url.clone()) {
                out.push(url);
                if out.len() >= max {
                    break;
                }
            }
        }
    }
    out
}

/// The `scheme://host[:port]` origin of `url` (best-effort; empty when unparseable).
fn origin_of(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return String::new();
    };
    let after = scheme_end + 3;
    let authority_len = url[after..]
        .find(['/', '?', '#'])
        .unwrap_or(url.len() - after);
    url[..after + authority_len].to_string()
}

/// Resolve an href against an origin. Absolute http(s) as-is, protocol-relative
/// `//host` upgraded to https, root-relative `/path` joined to the origin.
fn resolve_href(href: &str, origin: &str) -> String {
    let href = href.trim();
    if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if let Some(rest) = href.strip_prefix("//") {
        format!("https://{rest}")
    } else if href.starts_with('/') && !origin.is_empty() {
        format!("{origin}{href}")
    } else {
        String::new()
    }
}

/// Minimal inline strip_tags helper used by `extract_result_links` to avoid a dep
/// on `parse.rs`.
fn strip_tags_impl(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for character in html.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(character),
            _ => {}
        }
    }
    out
}
