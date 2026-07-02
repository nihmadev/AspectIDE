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

/// Parse a DuckDuckGo results page. Tolerant of several markup variants so a
/// selector drift on either endpoint degrades gracefully instead of yielding zero:
///   * the full page (`result__a` / `result__snippet`),
///   * the lite page's historical class (`result-link` / `result-snippet`), and
///   * the lite page's *current* class-less markup, where result anchors carry no
///     class at all and are recognized purely by their `/l/?uddg=` redirect href.
///
/// Results are deduped by URL. Best-effort and defensive: malformed entries are
/// skipped.
#[must_use]
pub fn parse_duckduckgo_html(html: &str) -> Vec<SearchHit> {
    let mut hits = parse_result_anchors(html, "result__a", "result__snippet");
    hits.extend(parse_result_anchors(html, "result-link", "result-snippet"));
    // Lite-page fallback: current lite markup dropped the `result-link` class, so
    // the class-based pass above finds nothing there. Recognize a result by its
    // DuckDuckGo redirect href instead, and pair it with the nearest following
    // `result-snippet` cell.
    hits.extend(parse_redirect_anchors(html, "result-snippet"));
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

/// Class-less fallback for the current lite page: scan every `<a href=...>`, keep
/// only anchors whose href is a DuckDuckGo `/l/?uddg=` redirect (the shape a real
/// organic result uses), and normalize each to its public http(s) target. This
/// deliberately ignores navigation/footer/settings anchors (which are relative
/// `/lite/…` or absolute duckduckgo.com links without `uddg=`), so it does not
/// pollute results with chrome links.
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

        // Only anchors that are DuckDuckGo result redirects are organic results.
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

/// Pull the nearest following `snippet_class` cell for a lite result, bounded so a
/// result with no snippet of its own doesn't borrow a distant one. Unlike
/// [`extract_following_snippet`], there is no per-result link class to bound
/// against on the class-less lite page, so we bound purely by distance.
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
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
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

/// Very small English stop-word set for building the keyword-only query variant.
/// Deliberately tiny — just the highest-frequency function words — so it never
/// strips a term that could matter for a technical query.
const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "to", "in", "on", "for", "with", "how", "what", "is",
    "are", "do", "does", "can", "vs", "my", "i", "me",
];

/// Expand one query into up to `max` mechanical variants for deep research, so a
/// single phrasing doesn't cap recall. No LLM: purely lexical transforms —
///   * the original query (always first),
///   * a quoted exact phrase (when multi-word) to surface pages with the literal phrase,
///   * a focus-aware suffix variant (e.g. `… documentation` for code, `… 2026` for news),
///   * a stop-word-stripped keyword-only variant (when it differs from the original).
///
/// Variants are de-duplicated case-insensitively and capped at `max` (which the
/// caller derives from the depth profile). Returns at least the original query.
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

    // Focus-aware suffix: bias the extra query toward the requested source kind.
    let suffix = match focus {
        FocusMode::Code => "documentation",
        FocusMode::Academic => "paper pdf",
        FocusMode::News => "latest 2026",
        FocusMode::Social => "discussion forum",
        FocusMode::Video => "video tutorial",
        FocusMode::Web => "guide",
    };
    push(format!("{trimmed} {suffix}"), &mut out, &mut seen);

    // Keyword-only variant: drop stop words so a verbose question matches
    // term-dense pages. Only added when it actually differs from the original.
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

/// Extract candidate follow-up links from a fetched result page's raw HTML for the
/// deep-mode 1-hop crawl. Keeps only anchors whose visible text shares at least one
/// query term (so we follow on-topic links, not nav/footer chrome), resolves
/// relative/protocol-relative hrefs against `base_url`, funnels every candidate
/// through the SSRF gate ([`validate_source_url`]), and de-duplicates. Bounded to
/// `max` links so one link-heavy page can't explode the crawl.
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

    let bytes = html.as_bytes();
    let mut cursor = 0usize;
    while let Some(rel) = html[cursor..].find("<a ") {
        let tag_start = cursor + rel;
        let Some(rel_gt) = html[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + rel_gt;
        let tag = &html[tag_start..tag_end];
        cursor = tag_end + 1;
        // Anchor text: from after '>' to the next '</a>'.
        let text = html[cursor..]
            .find("</a>")
            .map_or("", |rel_close| &html[cursor..cursor + rel_close]);
        let Some(href) = extract_attr(tag, "href") else {
            continue;
        };
        // Only follow links whose anchor text is on-topic.
        let text_lower = strip_tags(text).to_lowercase();
        let on_topic = query_terms.is_empty()
            || query_terms
                .iter()
                .any(|term| text_lower.contains(term.as_str()));
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
        // `bytes` kept in scope to satisfy borrow reasoning on `html` slicing.
        let _ = bytes;
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

/// Resolve an href against an origin: absolute http(s) as-is, protocol-relative
/// `//host` upgraded to https, root-relative `/path` joined to the origin. Anything
/// else (fragment, mailto, relative path) yields an empty string (dropped later).
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
    fn parses_classless_lite_results() {
        // CURRENT lite markup: result anchors carry NO class — they are recognized
        // purely by their `/l/?uddg=` redirect href — and the snippet is a
        // `result-snippet` cell. This is the markup the old class-based pass missed
        // (returning zero results); this regression-guards the href-based fallback.
        let html = r#"
          <table>
            <tr><td valign="top">1.&nbsp;</td>
                <td><a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2F&amp;rut=abc">The Rust Programming Language</a></td></tr>
            <tr><td>&nbsp;</td><td class="result-snippet">A language empowering everyone to build reliable software.</td></tr>
            <tr><td>&nbsp;</td><td><span class="link-text">www.rust-lang.org</span></td></tr>
            <tr><td valign="top">2.&nbsp;</td>
                <td><a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org%2Fbook%2F">The Rust Book</a></td></tr>
            <tr><td>&nbsp;</td><td class="result-snippet">The official Rust book.</td></tr>
            <tr><td><a href="/lite/?q=rust&s=30&nextParams=">Next Page &gt;</a></td></tr>
          </table>
        "#;
        let hits = parse_duckduckgo_html(html);
        assert_eq!(hits.len(), 2, "both organic results, no nav anchor");
        assert_eq!(hits[0].url, "https://www.rust-lang.org/");
        assert_eq!(hits[0].title, "The Rust Programming Language");
        assert_eq!(
            hits[0].snippet,
            "A language empowering everyone to build reliable software."
        );
        assert_eq!(hits[1].url, "https://doc.rust-lang.org/book/");
        assert_eq!(hits[1].title, "The Rust Book");
        // The "Next Page" nav anchor (relative, no uddg=) must NOT become a hit.
        assert!(
            hits.iter().all(|hit| !hit.url.contains("/lite/")),
            "navigation anchors must be excluded"
        );
    }

    #[test]
    fn classless_lite_pass_ignores_non_redirect_anchors() {
        // Header/footer/settings anchors on the lite page (absolute duckduckgo.com
        // links WITHOUT a uddg= redirect, or relative links) must never be results.
        let html = r#"
          <a href="https://duckduckgo.com/settings">Settings</a>
          <a href="/lite/about">About</a>
          <a href="https://duckduckgo.com/html/?q=x">Switch to HTML</a>
        "#;
        assert!(
            parse_duckduckgo_html(html).is_empty(),
            "chrome links are not organic results"
        );
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
        assert_eq!(
            validate_source_url("http://169.254.169.254/latest/meta-data"),
            None
        );
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

    #[test]
    fn expand_queries_produces_deduped_capped_variants() {
        let out = expand_queries("rust async runtime", FocusMode::Code, 5);
        assert_eq!(out[0], "rust async runtime", "original is always first");
        assert!(
            out.iter().any(|q| q == "\"rust async runtime\""),
            "multi-word query gets a quoted exact-phrase variant"
        );
        assert!(
            out.iter().any(|q| q.contains("documentation")),
            "code focus adds a documentation-suffixed variant"
        );
        assert!(out.len() <= 5, "capped at max");
        // No duplicates (case-insensitive).
        let lowered: std::collections::HashSet<String> =
            out.iter().map(|q| q.to_lowercase()).collect();
        assert_eq!(lowered.len(), out.len());
    }

    #[test]
    fn expand_queries_single_word_and_cap_one() {
        // A one-word query has no quoted/keyword variant but still gets the suffix.
        let out = expand_queries("bitcoin", FocusMode::News, 5);
        assert_eq!(out[0], "bitcoin");
        assert!(out.iter().any(|q| q.contains("2026")));
        // max = 1 collapses to just the original.
        assert_eq!(expand_queries("a b c", FocusMode::Web, 1), vec!["a b c"]);
    }

    #[test]
    fn extract_result_links_filters_and_resolves() {
        let html = r#"
            <a href="/docs/guide">rust async guide</a>
            <a href="https://other.example/async">async in rust</a>
            <a href="/about">company about page</a>
            <a href="https://evil.test/x">rust async</a>
            <a href="http://127.0.0.1/rust">rust async localhost</a>
        "#;
        let links = extract_result_links(html, "https://base.example/page", "rust async", 10);
        // Root-relative resolved against origin, and the absolute on-topic link kept.
        assert!(links.contains(&"https://base.example/docs/guide".to_string()));
        assert!(links.contains(&"https://other.example/async".to_string()));
        // Off-topic anchor text ("company about page") dropped.
        assert!(!links.iter().any(|u| u.ends_with("/about")));
        // SSRF loopback dropped even though the anchor text is on-topic.
        assert!(!links.iter().any(|u| u.contains("127.0.0.1")));
    }

    #[test]
    fn extract_result_links_respects_max() {
        let html = r#"
            <a href="https://a.example/rust">rust one</a>
            <a href="https://b.example/rust">rust two</a>
            <a href="https://c.example/rust">rust three</a>
        "#;
        let links = extract_result_links(html, "https://base.example/", "rust", 2);
        assert_eq!(links.len(), 2, "crawl budget cap honored");
    }
}
