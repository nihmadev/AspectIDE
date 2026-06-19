//! Search-provider URL construction + result parsing for SearxNG (JSON API) and a
//! keyless DuckDuckGo HTML fallback. Pure: callers do the HTTP and hand the body
//! back here to parse.

use crate::model::{FocusMode, SearchHit};

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

/// Parse a SearxNG `format=json` body into hits (`results[]` → url/title/content/engine/score).
#[must_use]
pub fn parse_searxng_json(json: &serde_json::Value) -> Vec<SearchHit> {
    let Some(results) = json.get("results").and_then(|value| value.as_array()) else {
        return Vec::new();
    };
    results
        .iter()
        .filter_map(|item| {
            let url = item.get("url").and_then(|value| value.as_str())?.trim();
            if url.is_empty() {
                return None;
            }
            Some(SearchHit {
                url: url.to_string(),
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

/// Build a keyless DuckDuckGo HTML search URL for `query` (full results page).
#[must_use]
pub fn duckduckgo_search_url(query: &str) -> String {
    format!(
        "https://html.duckduckgo.com/html/?q={}",
        percent_encode(query)
    )
}

/// Build the DuckDuckGo *lite* search URL — a minimal, JS-free results table that
/// is far more stable to scrape; used as a secondary when the full page yields none.
#[must_use]
pub fn duckduckgo_lite_search_url(query: &str) -> String {
    format!(
        "https://lite.duckduckgo.com/lite/?q={}",
        percent_encode(query)
    )
}

/// Parse a DuckDuckGo results page. Handles both the full page (`result__a` /
/// `result__snippet`) and the lite page (`result-link` / `result-snippet`), and
/// dedupes by URL. Best-effort and defensive: malformed entries are skipped.
#[must_use]
pub fn parse_duckduckgo_html(html: &str) -> Vec<SearchHit> {
    let mut hits = parse_result_anchors(html, "result__a", "result__snippet");
    hits.extend(parse_result_anchors(html, "result-link", "result-snippet"));
    let mut seen = std::collections::HashSet::new();
    hits.retain(|hit| seen.insert(hit.url.clone()));
    hits
}

// ── internals ──

/// Scan for result anchors carrying `link_class`, pairing each with the nearest
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
            snippet: extract_following_snippet(html, cursor, snippet_class),
            engine: "duckduckgo".to_string(),
            provider_score: 0.0,
        });
    }
    hits
}

fn string_field(item: &serde_json::Value, key: &str) -> String {
    item.get(key)
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Pull the `snippet_class` text that follows a result anchor, if present and
/// reasonably close (within ~1.5KB) to avoid grabbing the next result's snippet.
fn extract_following_snippet(html: &str, from: usize, snippet_class: &str) -> String {
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
        .find("</a>")
        .or_else(|| html[start..].find("</td>"))
        .or_else(|| html[start..].find("</div>"))
        .map_or(html.len(), |rel| start + rel);
    strip_tags(&html[start..end])
}

/// Extract an attribute value, tolerating both `name="…"` and `name='…'` quoting.
fn extract_attr(tag: &str, name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let needle = format!("{name}={quote}");
        if let Some(open) = tag.find(&needle) {
            let start = open + needle.len();
            if let Some(rel_end) = tag[start..].find(quote) {
                return Some(tag[start..start + rel_end].to_string());
            }
        }
    }
    None
}

/// Turn a DuckDuckGo result href into a real, http(s)-only URL: decode the `uddg=`
/// redirect param, upgrade a protocol-relative `//host`, and drop anything that is
/// not an http(s) URL (so a `javascript:`/`data:` href can never become a source).
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
    let lower = resolved.trim_start().to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        resolved
    } else {
        String::new()
    }
}

fn strip_tags(html: &str) -> String {
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

/// RFC-3986 component encoding (unreserved chars pass through).
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                match (hex_value(bytes[index + 1]), hex_value(bytes[index + 2])) {
                    (Some(high), Some(low)) => {
                        out.push(high * 16 + low);
                        index += 3;
                    }
                    _ => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searxng_url_encodes_query_and_category() {
        let url = searxng_search_url("https://searx.example/", "rust async", FocusMode::Social);
        assert_eq!(
            url,
            "https://searx.example/search?q=rust%20async&format=json&safesearch=0&categories=social%20media"
        );
    }

    #[test]
    fn parses_searxng_json() {
        let json = serde_json::json!({
            "results": [
                { "url": "https://a.com", "title": "A", "content": "about a", "engine": "google", "score": 1.5 },
                { "title": "no url" },
                { "url": "https://b.com", "title": "B" }
            ]
        });
        let hits = parse_searxng_json(&json);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://a.com");
        assert_eq!(hits[0].snippet, "about a");
        assert!((hits[0].provider_score - 1.5).abs() < 1e-9);
        assert_eq!(hits[1].engine, "searxng");
    }

    #[test]
    fn parses_duckduckgo_html_with_uddg_redirect() {
        let html = r#"
          <div class="result">
            <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage&rut=x">Example <b>Page</b></a>
            <a class="result__snippet" href="...">A helpful snippet here.</a>
          </div>
          <div class="result">
            <a class="result__a" href="https://direct.example/x">Direct Title</a>
          </div>
        "#;
        let hits = parse_duckduckgo_html(html);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://example.com/page");
        assert_eq!(hits[0].title, "Example Page");
        assert_eq!(hits[0].snippet, "A helpful snippet here.");
        assert_eq!(hits[1].url, "https://direct.example/x");
    }

    #[test]
    fn parses_duckduckgo_lite_results() {
        // Lite page: result-link anchors (single-quoted) + result-snippet cells.
        let html = r#"
          <table>
            <tr><td><a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org%2F" class='result-link'>The Rust Language</a></td></tr>
            <tr><td class="result-snippet">A language empowering everyone.</td></tr>
          </table>
        "#;
        let hits = parse_duckduckgo_html(html);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://rust-lang.org/");
        assert_eq!(hits[0].title, "The Rust Language");
        assert_eq!(hits[0].snippet, "A language empowering everyone.");
    }

    #[test]
    fn drops_non_http_result_urls() {
        let html = r#"<a class="result__a" href="javascript:alert(1)">Evil</a>"#;
        assert!(parse_duckduckgo_html(html).is_empty());
    }

    #[test]
    fn percent_roundtrip() {
        assert_eq!(percent_decode(&percent_encode("a b/c?d")), "a b/c?d");
        assert_eq!(percent_decode("a+b"), "a b");
    }
}
