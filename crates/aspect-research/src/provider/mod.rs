//! Search-provider URL construction + result parsing for SearxNG (JSON API) and
//! the keyless HTML fallbacks (DuckDuckGo, then Brave Search when DuckDuckGo
//! serves its anomaly/challenge page, then Mojeek when both stonewall). Pure:
//! callers do the HTTP and hand the body back here to parse.

use std::net::{IpAddr, Ipv6Addr};

mod parse;
mod url;

pub use parse::{parse_brave_html, parse_duckduckgo_html, parse_mojeek_html, parse_searxng_json};
pub use url::{
    brave_search_url, canonical_url_key, duckduckgo_lite_search_url, duckduckgo_search_url,
    expand_queries, extract_result_links, focus_biased_query, mojeek_search_url,
    searxng_search_url,
};

/// Localhost names (and aliases) that must never become a fetchable source even
/// before DNS resolution.
const BLOCKED_HOST_NAMES: &[&str] = &["localhost", "ip6-localhost", "ip6-loopback"];

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
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() {
        return None;
    }
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
        return after.split(']').next().unwrap_or_default().to_ascii_lowercase();
    }
    authority.split(':').next().unwrap_or_default().to_ascii_lowercase()
}

/// Whether `host` is a name/IP we must never fetch (SSRF guard).
fn is_blocked_host(host: &str) -> bool {
    if BLOCKED_HOST_NAMES.contains(&host) {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(ip) => is_blocked_ip(&ip),
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
                || v4.is_link_local()
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
                || v6.to_ipv4_mapped().is_some_and(|v4| is_blocked_ip(&IpAddr::V4(v4)))
        }
    }
}

/// `fc00::/7` unique-local (the IPv6 analogue of RFC1918).
fn is_unique_local_v6(ip: &Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

/// `fe80::/10` link-local.
fn is_link_local_v6(ip: &Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}

// ── shared HTML/URL helpers ──

/// Extract an attribute value, tolerating both `name="…"` and `name='…'` quoting.
pub(crate) fn extract_attr(tag: &str, name: &str) -> Option<String> {
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

/// RFC-3986 component encoding (unreserved chars pass through).
pub(crate) fn percent_encode(input: &str) -> String {
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

pub(crate) fn percent_decode(input: &str) -> String {
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
