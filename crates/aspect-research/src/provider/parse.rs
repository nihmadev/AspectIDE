//! HTML/JSON result parsers for SearxNG, DuckDuckGo, Brave, and Mojeek. Each
//! takes the raw body and returns parsed [`SearchHit`]s.

use crate::model::SearchHit;
use super::{extract_attr, percent_decode, validate_source_url};

// ── SearxNG ──

/// Parse a SearxNG `format=json` body into hits.
#[must_use]
pub fn parse_searxng_json(json: &serde_json::Value) -> Vec<SearchHit> {
    let Some(results) = json.get("results").and_then(|value| value.as_array()) else {
        return Vec::new();
    };
    results
        .iter()
        .filter_map(|item| {
            let url = item.get("url").and_then(|value| value.as_str())?.trim();
            let url = validate_source_url(url)?;
            Some(SearchHit {
                url,
                title: string_field(item, "title"),
                snippet: string_field(item, "content"),
                engine: item
                    .get("engine")
                    .and_then(|value| value.as_str())
                    .unwrap_or("searxng")
                    .to_string(),
                provider_score: item
                    .get("score")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0),
            })
        })
        .collect()
}

pub(crate) fn string_field(item: &serde_json::Value, key: &str) -> String {
    item.get(key)
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

// ── DuckDuckGo ──

/// Parse a DuckDuckGo results page. Tolerant of several markup variants.
#[must_use]
pub fn parse_duckduckgo_html(html: &str) -> Vec<SearchHit> {
    let mut hits = parse_result_anchors(html, "result__a", "result__snippet");
    hits.extend(parse_result_anchors(html, "result-link", "result-snippet"));
    hits.extend(parse_redirect_anchors(html, "result-snippet"));
    let mut seen = std::collections::HashSet::new();
    hits.retain(|hit| seen.insert(hit.url.clone()));
    hits
}

/// Scan for result anchors carrying `link_class`, paired with the nearest
/// following `snippet_class` block.
fn parse_result_anchors(html: &str, link_class: &str, snippet_class: &str) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = html[cursor..].find(link_class) {
        let class_pos = cursor + relative;
        let Some(tag_start) = html[..class_pos].rfind("<a") else {
            cursor = class_pos + link_class.len();
            continue;
        };
        let Some(tag_rel_end) = html[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + tag_rel_end;
        let tag = &html[tag_start..=tag_end];
        let url = normalize_ddg_href(&extract_attr(tag, "href").unwrap_or_default());

        let inner_start = tag_end + 1;
        let Some(close_rel) = html[inner_start..].find("</a>") else {
            break;
        };
        let title = strip_tags(&html[inner_start..inner_start + close_rel]);
        cursor = inner_start + close_rel + "</a>".len();

        if url.is_empty() || title.is_empty() {
            continue;
        }
        hits.push(SearchHit {
            url,
            title,
            snippet: extract_following_snippet(html, cursor, link_class, snippet_class),
            engine: "duckduckgo".to_string(),
            provider_score: 0.0,
        });
    }
    hits
}

/// Class-less fallback for the current lite page: scan every `<a href=...>`, keep
/// only anchors with `uddg=` redirect.
fn parse_redirect_anchors(html: &str, snippet_class: &str) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = html[cursor..].find("<a") {
        let tag_start = cursor + relative;
        let Some(tag_rel_end) = html[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + tag_rel_end;
        let tag = &html[tag_start..=tag_end];
        let href = extract_attr(tag, "href").unwrap_or_default();

        let inner_start = tag_end + 1;
        let Some(close_rel) = html[inner_start..].find("</a>") else {
            break;
        };
        let title = strip_tags(&html[inner_start..inner_start + close_rel]);
        cursor = inner_start + close_rel + "</a>".len();

        if !href.contains("uddg=") {
            continue;
        }
        let url = normalize_ddg_href(&href);
        if url.is_empty() || title.is_empty() {
            continue;
        }
        hits.push(SearchHit {
            url,
            title,
            snippet: extract_lite_snippet(html, cursor, snippet_class),
            engine: "duckduckgo".to_string(),
            provider_score: 0.0,
        });
    }
    hits
}

/// Pull the nearest following `snippet_class` cell for a lite result, bounded.
fn extract_lite_snippet(html: &str, from: usize, snippet_class: &str) -> String {
    let Some(relative) = html[from..].find(snippet_class) else {
        return String::new();
    };
    if relative > 1_500 {
        return String::new();
    }
    let snippet_pos = from + relative;
    let Some(open_rel) = html[snippet_pos..].find('>') else {
        return String::new();
    };
    let start = snippet_pos + open_rel + 1;
    let end = html[start..]
        .find("</td>")
        .or_else(|| html[start..].find("</a>"))
        .or_else(|| html[start..].find("</div>"))
        .map_or(html.len(), |rel| start + rel);
    strip_tags(&html[start..end])
}

/// Pull the `snippet_class` text that follows a result anchor, if present and
/// bounded by distance and the next result anchor.
fn extract_following_snippet(
    html: &str,
    from: usize,
    link_class: &str,
    snippet_class: &str,
) -> String {
    let Some(relative) = html[from..].find(snippet_class) else {
        return String::new();
    };
    if relative > 1_500 {
        return String::new();
    }
    if let Some(next_anchor) = html[from..].find(link_class) {
        if next_anchor < relative {
            return String::new();
        }
    }
    let snippet_pos = from + relative;
    let Some(open_rel) = html[snippet_pos..].find('>') else {
        return String::new();
    };
    let start = snippet_pos + open_rel + 1;
    let end = html[start..]
        .find("</a>")
        .or_else(|| html[start..].find("</td>"))
        .or_else(|| html[start..].find("</div>"))
        .map_or(html.len(), |rel| start + rel);
    strip_tags(&html[start..end])
}

/// Turn a DuckDuckGo result href into a real, http(s)-only URL: decode the `uddg=`
/// redirect param, upgrade a protocol-relative `//host`, and reject non-http(s).
fn normalize_ddg_href(href: &str) -> String {
    let resolved = if let Some(pos) = href.find("uddg=") {
        let rest = &href[pos + "uddg=".len()..];
        let encoded = rest.split('&').next().unwrap_or(rest);
        percent_decode(encoded)
    } else if let Some(stripped) = href.strip_prefix("//") {
        format!("https://{stripped}")
    } else {
        href.to_string()
    };
    validate_source_url(resolved.trim_start()).unwrap_or_default()
}

// ── Brave ──

/// Parse a Brave Search results page.
#[must_use]
pub fn parse_brave_html(html: &str) -> Vec<SearchHit> {
    const TITLE_TOKEN: &str = "search-snippet-title";
    const SNIPPET_TOKEN: &str = "generic-snippet";
    let mut hits = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = html[cursor..].find(TITLE_TOKEN) {
        let token_pos = cursor + relative;
        let Some(tag_start) = html[..token_pos].rfind("<a ") else {
            cursor = token_pos + TITLE_TOKEN.len();
            continue;
        };
        let Some(tag_rel_end) = html[tag_start..].find('>') else {
            break;
        };
        let anchor_tag = &html[tag_start..tag_start + tag_rel_end];
        let href = extract_attr(anchor_tag, "href").unwrap_or_default();

        let Some(title_tag_end_rel) = html[token_pos..].find('>') else {
            break;
        };
        let title_tag = &html[token_pos..token_pos + title_tag_end_rel];
        let inner_start = token_pos + title_tag_end_rel + 1;
        let inner_end = html[inner_start..]
            .find('<')
            .map_or(html.len(), |rel| inner_start + rel);
        let title = extract_attr(&format!("<div {title_tag}"), "title")
            .map(|value| strip_tags(&value))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| strip_tags(&html[inner_start..inner_end]));
        cursor = inner_end;

        let Some(url) = validate_source_url(href.trim()) else {
            continue;
        };
        if url.contains("://search.brave.com/") || title.is_empty() {
            continue;
        }
        let snippet = html[cursor..]
            .find(SNIPPET_TOKEN)
            .filter(|rel| *rel < 2_500)
            .and_then(|rel| {
                let snippet_pos = cursor + rel;
                let open = html[snippet_pos..].find('>')?;
                let start = snippet_pos + open + 1;
                let end = html[start..]
                    .find("</div>")
                    .map_or(html.len(), |r| start + r);
                Some(strip_tags(&html[start..end]))
            })
            .unwrap_or_default();

        hits.push(SearchHit {
            url,
            title,
            snippet,
            engine: "brave".to_string(),
            provider_score: 0.0,
        });
    }
    let mut seen = std::collections::HashSet::new();
    hits.retain(|hit| seen.insert(hit.url.clone()));
    hits
}

// ── Mojeek ──

/// Parse a Mojeek results page.
#[must_use]
pub fn parse_mojeek_html(html: &str) -> Vec<SearchHit> {
    const TITLE_TOKEN: &str = "class=\"title\"";
    const SNIPPET_TOKEN: &str = "<p class=\"s\">";
    let mut hits = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = html[cursor..].find(TITLE_TOKEN) {
        let token_pos = cursor + relative;
        let Some(tag_start) = html[..token_pos].rfind("<a ") else {
            cursor = token_pos + TITLE_TOKEN.len();
            continue;
        };
        let Some(tag_rel_end) = html[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + tag_rel_end;
        let tag = &html[tag_start..=tag_end];
        let href = extract_attr(tag, "href").unwrap_or_default();

        let inner_start = tag_end + 1;
        let Some(close_rel) = html[inner_start..].find("</a>") else {
            break;
        };
        let title = strip_tags(&html[inner_start..inner_start + close_rel]);
        cursor = inner_start + close_rel + "</a>".len();

        let Some(url) = validate_source_url(href.trim()) else {
            continue;
        };
        if url.contains("://www.mojeek.com/") || title.is_empty() {
            continue;
        }
        let snippet = html[cursor..]
            .find(SNIPPET_TOKEN)
            .filter(|rel| *rel < 1_500)
            .map(|rel| {
                let start = cursor + rel + SNIPPET_TOKEN.len();
                let end = html[start..].find("</p>").map_or(html.len(), |r| start + r);
                strip_tags(&html[start..end])
            })
            .unwrap_or_default();

        hits.push(SearchHit {
            url,
            title,
            snippet,
            engine: "mojeek".to_string(),
            provider_score: 0.0,
        });
    }
    let mut seen = std::collections::HashSet::new();
    hits.retain(|hit| seen.insert(hit.url.clone()));
    hits
}

// ── shared HTML helpers ──

pub(crate) fn strip_tags(html: &str) -> String {
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
    decode_entities(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
}
