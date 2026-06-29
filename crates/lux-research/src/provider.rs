//! Search-provider URL construction + result parsing for SearxNG (JSON API) and a
//! keyless DuckDuckGo HTML fallback. Pure: callers do the HTTP and hand the body
//! back here to parse.

use std::net::{IpAddr, Ipv6Addr};

use crate::model::{FocusMode, SearchHit};

/// Localhost names (and aliases) that must never become a fetchable source even
/// before DNS resolution.
const BLOCKED_HOST_NAMES: &[&str] = &["localhost", "ip6-localhost", "ip6-loopback"];

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
            // SSRF gate: drop any provider URL that isn't a safe, public http(s)
            // source before it can ever reach the fetch layer.
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
            snippet: extract_following_snippet(html, cursor, link_class, snippet_class),
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
/// reasonably close (within ~1.5KB). Bounded by the next result anchor: if another
/// `link_class` result starts before the next snippet, the current result simply has
/// no snippet — we return empty rather than stealing (and duplicating) the next
/// result's snippet.
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
    // The snippet must belong to THIS result: if the next result's anchor appears
    // before it, this result has none of its own.
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
    // Funnel through the shared SSRF gate so a DDG redirect can't smuggle a
    // loopback/metadata/private host (or userinfo-obfuscated one) into a source.
    validate_source_url(resolved.trim_start()).unwrap_or_default()
}

/// The single SSRF gate every provider URL passes through before becoming a
/// [`SearchHit`]. Returns the normalized URL when it is a safe, fetchable public
/// http(s) source, or `None` to drop it. Rejects: non-http(s) schemes, embedded
/// userinfo (`user:pass@host`), localhost aliases, and any IP literal that is
/// loopback, private, link-local (incl. the `169.254.169.254` cloud-metadata
/// address), unique-local, multicast, or unspecified. Hostnames that aren't IP
/// literals are allowed through here; the fetch layer must re-validate the
/// resolved address after DNS and after every redirect hop.
#[must_use]
pub fn validate_source_url(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = strip_http_scheme(url)?;
    // Authority ends at the first '/', '?' or '#'.
    let authority_end = rest
        .find(['/', '?', '#'])
        .unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() {
        return None;
    }
    // Reject userinfo: `https://example.com@127.0.0.1/` resolves to the host
    // AFTER the '@', so anything before it is an obfuscation vector.
    if authority.contains('@') {
        return None;
    }
    let host = host_from_authority(authority);
    if host.is_empty() || is_blocked_host(&host) {
        return None;
    }
    Some(url.to_string())
}

/// Lowercased scheme strip: returns the post-`scheme://` remainder for http(s)
/// only (the one place the http/https allowlist is enforced).
fn strip_http_scheme(url: &str) -> Option<&str> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") {
        Some(&url["https://".len()..])
    } else if lower.starts_with("http://") {
        Some(&url["http://".len()..])
    } else {
        None
    }
}

/// Extract the host from an `authority` (`host[:port]`), unwrapping a bracketed
/// IPv6 literal (`[::1]:8080` → `::1`).
fn host_from_authority(authority: &str) -> String {
    if let Some(after) = authority.strip_prefix('[') {
        // IPv6 literal: host is everything up to the closing bracket.
        return after
            .split(']')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
    }
    authority
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Whether `host` is a name/IP we must never fetch (SSRF guard).
fn is_blocked_host(host: &str) -> bool {
    if BLOCKED_HOST_NAMES.contains(&host) {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(ip) => is_blocked_ip(&ip),
        // A non-IP hostname passes the literal check; DNS-time re-validation in
        // the fetch layer is what closes the rebinding gap.
        Err(_) => false,
    }
}

/// Reject loopback, private, link-local (incl. cloud metadata), unique-local,
/// multicast, and unspecified addresses.
fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local() // 169.254.0.0/16 — includes 169.254.169.254 metadata
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_documentation()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                || is_unique_local_v6(v6)
                || is_link_local_v6(v6)
                // An IPv4-mapped address (::ffff:127.0.0.1) must be judged by its
                // embedded v4 so it can't bypass the v4 rules above.
                || v6.to_ipv4_mapped().is_some_and(|v4| is_blocked_ip(&IpAddr::V4(v4)))
        }
    }
}

/// `fc00::/7` unique-local (the IPv6 analogue of RFC1918). Hand-rolled because
/// `Ipv6Addr::is_unique_local` is unstable.
fn is_unique_local_v6(ip: &Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

/// `fe80::/10` link-local. Hand-rolled because `Ipv6Addr::is_unicast_link_local`
/// is unstable.
fn is_link_local_v6(ip: &Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
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
    fn snippet_less_result_does_not_steal_the_next_snippet() {
        // Result A has no snippet of its own; result B does. A must NOT borrow B's.
        let html = r#"
          <a class="result__a" href="https://a.com/aaa">Result A</a>
          <a class="result__a" href="https://b.com/bbb">Result B</a>
          <a class="result__snippet" href="x">This snippet belongs to B.</a>
        "#;
        let hits = parse_duckduckgo_html(html);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://a.com/aaa");
        assert_eq!(hits[0].snippet, "", "A must not borrow B's snippet");
        assert_eq!(hits[1].url, "https://b.com/bbb");
        assert_eq!(hits[1].snippet, "This snippet belongs to B.");
    }

    #[test]
    fn percent_roundtrip() {
        assert_eq!(percent_decode(&percent_encode("a b/c?d")), "a b/c?d");
        assert_eq!(percent_decode("a+b"), "a b");
    }

    #[test]
    fn validate_source_url_allows_public_https() {
        assert_eq!(
            validate_source_url("https://example.com/page?q=1"),
            Some("https://example.com/page?q=1".to_string())
        );
        assert_eq!(
            validate_source_url("http://docs.rs/"),
            Some("http://docs.rs/".to_string())
        );
    }

    #[test]
    fn validate_source_url_blocks_ssrf_targets() {
        // Non-http(s) schemes.
        assert_eq!(validate_source_url("javascript:alert(1)"), None);
        assert_eq!(validate_source_url("file:///etc/passwd"), None);
        // Loopback / localhost aliases.
        assert_eq!(validate_source_url("http://127.0.0.1/"), None);
        assert_eq!(validate_source_url("http://localhost:8080/admin"), None);
        assert_eq!(validate_source_url("http://[::1]/"), None);
        // Cloud metadata + link-local.
        assert_eq!(validate_source_url("http://169.254.169.254/latest/meta-data"), None);
        // RFC1918 / private + ULA.
        assert_eq!(validate_source_url("http://10.0.0.5/"), None);
        assert_eq!(validate_source_url("http://192.168.1.1/"), None);
        assert_eq!(validate_source_url("http://172.16.0.1/"), None);
        assert_eq!(validate_source_url("http://[fc00::1]/"), None);
        // Userinfo obfuscation: the real host is AFTER the '@'.
        assert_eq!(validate_source_url("https://example.com@127.0.0.1/"), None);
        // IPv4-mapped IPv6 loopback must not bypass the v4 rules.
        assert_eq!(validate_source_url("http://[::ffff:127.0.0.1]/"), None);
        // Unspecified.
        assert_eq!(validate_source_url("http://0.0.0.0/"), None);
    }

    #[test]
    fn searxng_json_drops_ssrf_urls() {
        let json = serde_json::json!({
            "results": [
                { "url": "https://safe.example/ok", "title": "ok" },
                { "url": "http://169.254.169.254/latest", "title": "metadata" },
                { "url": "http://localhost/internal", "title": "loopback" }
            ]
        });
        let hits = parse_searxng_json(&json);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://safe.example/ok");
    }
}
