use std::{
    net::{IpAddr, ToSocketAddrs},
    time::Duration,
};

use reqwest::redirect::Policy;
use serde::Serialize;
use tokio::time::timeout;

const DEFAULT_TIMEOUT_SECS: u64 = 20;
const MAX_TIMEOUT_SECS: u64 = 60;
const DEFAULT_MAX_BYTES: u64 = 250_000;
const MAX_BYTES: u64 = 1_000_000;
const USER_AGENT: &str = "LuxIDE-WebFetch/0.1";
/// Browser-like UA for SEARCH-PROVIDER requests only — DuckDuckGo/SearxNG serve an
/// empty/challenge page to bot user-agents, so the research search query must look
/// like a browser. (`fetch`/`WebFetch` keep the honest `USER_AGENT`.)
const SEARCH_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebFetchResponse {
    url: String,
    final_url: String,
    status: u16,
    content_type: Option<String>,
    title: Option<String>,
    text: String,
    bytes_read: u64,
    truncated: bool,
    elapsed_ms: u128,
}

impl WebFetchResponse {
    /// Extracted, normalized page text (read by the research engine).
    pub fn text(&self) -> &str {
        &self.text
    }
    /// Page title, if one was found.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }
}

pub async fn fetch(
    url: String,
    max_bytes: Option<u64>,
    timeout_secs: Option<u64>,
    allow_private_hosts: Option<bool>,
) -> Result<WebFetchResponse, String> {
    let started = std::time::Instant::now();
    let url = validate_url(&url)?;
    let allow_private_hosts = allow_private_hosts.unwrap_or(false);
    if !allow_private_hosts {
        reject_private_host(&url).await?;
    }

    let max_bytes = max_bytes
        .unwrap_or(DEFAULT_MAX_BYTES)
        .clamp(1_024, MAX_BYTES);
    let timeout_secs = timeout_secs
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .redirect(ssrf_redirect_policy())
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| error.to_string())?;

    let response = timeout(
        Duration::from_secs(timeout_secs + 5),
        client.get(url.clone()).send(),
    )
    .await
    .map_err(|_| "WebFetch request timed out".to_string())?
    .map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    if !allow_private_hosts {
        reject_private_host(response.url()).await?;
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let bytes = response.bytes().await.map_err(|error| error.to_string())?;
    let truncated = bytes.len() as u64 > max_bytes;
    let visible = &bytes[..usize::min(bytes.len(), max_bytes as usize)];
    let raw_text = String::from_utf8_lossy(visible).to_string();
    let text = normalize_text(&raw_text, content_type.as_deref());
    let title = extract_html_title(&raw_text);

    Ok(WebFetchResponse {
        url: url.to_string(),
        final_url,
        status,
        content_type,
        title,
        text,
        bytes_read: visible.len() as u64,
        truncated,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

/// SSRF-safe GET that returns the raw response body as text. Reuses the same URL
/// validation + private-host rejection as [`fetch`]. Used by the research engine
/// to query a search provider: a user-configured `SearxNG` instance is trusted
/// (`allow_private = true`, since it is commonly self-hosted on localhost/LAN),
/// while the public `DuckDuckGo` fallback keeps the private-host guard on.
pub async fn fetch_text(
    url: &str,
    accept: &str,
    timeout_secs: u64,
    max_bytes: usize,
    allow_private: bool,
) -> Result<String, String> {
    let parsed = validate_url(url)?;
    if !allow_private {
        reject_private_host(&parsed).await?;
    }
    let timeout_secs = timeout_secs.clamp(1, MAX_TIMEOUT_SECS);
    // Search providers (DuckDuckGo especially) fingerprint the TLS ClientHello and
    // serve a content-free challenge page to reqwest's default rustls stack — the
    // request returns 200 with ~14KB and zero result anchors. The platform-native
    // TLS backend (Schannel / Secure Transport / vendored OpenSSL) presents a
    // browser-like handshake that passes, returning the real results page. Normal
    // WebFetch keeps rustls; only this search path opts into native TLS.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .redirect(ssrf_redirect_policy())
        .user_agent(SEARCH_USER_AGENT)
        .use_native_tls()
        .build()
        .map_err(|error| error.to_string())?;
    let response = timeout(
        Duration::from_secs(timeout_secs + 5),
        client
            .get(parsed.clone())
            .header(reqwest::header::ACCEPT, accept)
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
            .send(),
    )
    .await
    .map_err(|_| "search request timed out".to_string())?
    .map_err(|error| error.to_string())?;
    if !allow_private {
        reject_private_host(response.url()).await?;
    }
    if !response.status().is_success() {
        return Err(format!(
            "search provider returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let bytes = response.bytes().await.map_err(|error| error.to_string())?;
    let visible = &bytes[..usize::min(bytes.len(), max_bytes)];
    Ok(String::from_utf8_lossy(visible).into_owned())
}

/// Redirect policy that re-checks SSRF on EVERY hop, not just the first/final URL.
/// `Policy::limited` would follow a 302 into a private/metadata host before the
/// caller's final-URL guard runs; this rejects literal localhost/private-IP hops
/// synchronously (a redirect callback cannot await DNS — the caller's async
/// `reject_private_host` still covers DNS-resolved privates on the first/last hop).
fn ssrf_redirect_policy() -> Policy {
    Policy::custom(|attempt| {
        let host = attempt.url().host_str().unwrap_or("");
        if host.trim().is_empty() {
            return attempt.stop();
        }
        let lower = host.trim_end_matches('.').to_ascii_lowercase();
        if lower == "localhost" || lower.ends_with(".localhost") {
            return attempt.error("redirect blocked: localhost host");
        }
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_private_ip(ip) {
                return attempt.error("redirect blocked: private IP");
            }
        }
        if attempt.previous().len() >= 5 {
            return attempt.stop();
        }
        attempt.follow()
    })
}

fn validate_url(url: &str) -> Result<reqwest::Url, String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("WebFetch URL is empty".to_string());
    }
    let parsed =
        reqwest::Url::parse(trimmed).map_err(|error| format!("Invalid WebFetch URL: {error}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => return Err(format!("Unsupported WebFetch URL scheme: {scheme}")),
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "WebFetch URL must include a host".to_string())?;
    if host.trim().is_empty() {
        return Err("WebFetch URL host is empty".to_string());
    }
    Ok(parsed)
}

async fn reject_private_host(url: &reqwest::Url) -> Result<(), String> {
    let host = url
        .host_str()
        .ok_or_else(|| "WebFetch URL must include a host".to_string())?
        .to_string();
    if is_localhost_name(&host) {
        return Err("WebFetch blocks localhost/private hosts by default".to_string());
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "WebFetch URL has no usable port".to_string())?;
    let addresses = tokio::task::spawn_blocking(move || {
        (host.as_str(), port)
            .to_socket_addrs()
            .map(|iter| iter.map(|socket| socket.ip()).collect::<Vec<_>>())
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    if addresses.is_empty() {
        return Err("WebFetch host did not resolve to any address".to_string());
    }
    if addresses.iter().any(|ip| is_private_ip(*ip)) {
        return Err("WebFetch blocks localhost/private network addresses by default".to_string());
    }
    Ok(())
}

fn is_localhost_name(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost")
}

const fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.segments()[0] & 0xffc0 == 0xfe80
        }
    }
}

fn normalize_text(text: &str, content_type: Option<&str>) -> String {
    let looks_html = content_type.map_or_else(
        || text.contains("<html") || text.contains("<body") || text.contains("<!DOCTYPE html"),
        |value| value.to_ascii_lowercase().contains("html"),
    );
    let normalized = if looks_html {
        html_to_text(text)
    } else {
        text.to_string()
    };
    compact_whitespace(&normalized)
}

fn html_to_text(html: &str) -> String {
    let without_scripts = strip_html_block(html, "script");
    let without_styles = strip_html_block(&without_scripts, "style");
    let with_breaks = without_styles
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</p>", "\n")
        .replace("</div>", "\n")
        .replace("</li>", "\n")
        .replace("</h1>", "\n")
        .replace("</h2>", "\n")
        .replace("</h3>", "\n");
    let mut output = String::with_capacity(with_breaks.len());
    let mut in_tag = false;
    for character in with_breaks.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(character),
            _ => {}
        }
    }
    decode_basic_html_entities(&output)
}

fn strip_html_block(input: &str, tag: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let open_pattern = format!("<{tag}");
    let close_pattern = format!("</{tag}>");
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(relative_start) = lower[cursor..].find(&open_pattern) {
        let start = cursor + relative_start;
        output.push_str(&input[cursor..start]);
        let Some(relative_end) = lower[start..].find(&close_pattern) else {
            return output;
        };
        cursor = start + relative_end + close_pattern.len();
    }
    output.push_str(&input[cursor..]);
    output
}

fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let after_open = lower[start..].find('>')? + start + 1;
    let end = lower[after_open..].find("</title>")? + after_open;
    let title = decode_basic_html_entities(&html[after_open..end]);
    let compact = compact_whitespace(&title);
    (!compact.is_empty()).then_some(compact)
}

fn compact_whitespace(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_basic_html_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_guard_rejects_unsupported_schemes() {
        assert!(validate_url("file:///C:/Windows/win.ini").is_err());
        assert!(validate_url("ftp://example.com/file.txt").is_err());
        assert!(validate_url("https://example.com/docs").is_ok());
    }

    #[test]
    fn private_ip_guard_detects_local_ranges() {
        assert!(is_private_ip("127.0.0.1".parse().unwrap()));
        assert!(is_private_ip("10.0.0.12".parse().unwrap()));
        assert!(is_private_ip("172.16.0.1".parse().unwrap()));
        assert!(is_private_ip("192.168.1.20".parse().unwrap()));
        assert!(is_private_ip("::1".parse().unwrap()));
        assert!(!is_private_ip("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn html_to_text_removes_scripts_and_extracts_title() {
        let html = r"<!doctype html><html><head><title>Docs &amp; API</title><style>.x{}</style><script>secret()</script></head><body><h1>Hello</h1><p>World&nbsp;now</p></body></html>";

        assert_eq!(extract_html_title(html).as_deref(), Some("Docs & API"));
        let text = normalize_text(html, Some("text/html; charset=utf-8"));

        assert!(text.contains("Hello"));
        assert!(text.contains("World now"));
        assert!(!text.contains("secret()"));
        assert!(!text.contains(".x{}"));
    }

    // Live network-gated check: the search client must use a TLS backend DuckDuckGo
    // does not fingerprint-block. Ignored by default (needs network); run with
    // `cargo test -p lux-desktop search_client_reaches_ddg -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "requires network"]
    async fn search_client_reaches_ddg() {
        let url = lux_research::duckduckgo_lite_search_url("rust async runtime");
        let body = fetch_text(&url, "text/html", 18, 600_000, false)
            .await
            .expect("search fetch should succeed");
        let hits = lux_research::parse_duckduckgo_html(&body);
        assert!(
            !hits.is_empty(),
            "expected DDG result hits, got body len {} (TLS-block challenge page?)",
            body.len()
        );
    }
}
