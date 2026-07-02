use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs},
    time::Duration,
};

use futures_util::StreamExt;
use reqwest::redirect::Policy;
use serde::Serialize;
use tokio::time::timeout;

const DEFAULT_TIMEOUT_SECS: u64 = 20;
const MAX_TIMEOUT_SECS: u64 = 60;
const DEFAULT_MAX_BYTES: u64 = 250_000;
const MAX_BYTES: u64 = 1_000_000;
const USER_AGENT: &str = "LuxIDE-WebFetch/0.1";
/// Browser-like UA for SEARCH-PROVIDER requests only — `DuckDuckGo`/`SearxNG` serve an
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
) -> Result<WebFetchResponse, String> {
    let started = std::time::Instant::now();
    let initial = validate_url(&url)?;
    let max_bytes = max_bytes
        .unwrap_or(DEFAULT_MAX_BYTES)
        .clamp(1_024, MAX_BYTES);
    let timeout_secs = timeout_secs
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS);

    // The model never gets to disable the SSRF guard: `allow_private` is hard
    // `false` here (H1). DNS is resolved once and pinned per hop, redirects are
    // followed manually with the guard re-applied on every hop, and the body is
    // capped during streaming — so rebinding (H2), redirect-to-private (H3) and
    // unbounded buffering (M3) are all closed.
    let guarded = fetch_guarded(GuardedRequest {
        initial,
        accept: None,
        accept_language: false,
        user_agent: USER_AGENT,
        use_native_tls: false,
        timeout_secs,
        max_bytes: usize::try_from(max_bytes).unwrap_or(usize::MAX),
        allow_private: false,
        form: None,
    })
    .await?;

    let raw_text = String::from_utf8_lossy(&guarded.body).to_string();
    let text = normalize_text(&raw_text, guarded.content_type.as_deref());
    let title = extract_html_title(&raw_text);

    Ok(WebFetchResponse {
        url: url.trim().to_string(),
        final_url: guarded.final_url.to_string(),
        status: guarded.status,
        content_type: guarded.content_type,
        title,
        text,
        bytes_read: guarded.body.len() as u64,
        truncated: guarded.truncated,
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
    fetch_search_text(url, None, accept, timeout_secs, max_bytes, allow_private).await
}

/// SSRF-safe POST that submits `form` as an `application/x-www-form-urlencoded`
/// body and returns the raw response text. The public `DuckDuckGo` fallback keeps
/// the private-host guard on (`allow_private = false`): DDG's html/lite endpoints
/// now answer a plain GET with an anomaly/empty page and only return results for a
/// POST whose body carries the query, so the research search path uses this.
pub async fn fetch_text_form(
    url: &str,
    form: &[(&str, &str)],
    accept: &str,
    timeout_secs: u64,
    max_bytes: usize,
) -> Result<String, String> {
    let owned: Vec<(String, String)> = form
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect();
    fetch_search_text(url, Some(owned), accept, timeout_secs, max_bytes, false).await
}

/// Shared core for [`fetch_text`] (GET) and [`fetch_text_form`] (POST). Builds the
/// browser-UA, native-TLS search request and enforces a 2xx status.
async fn fetch_search_text(
    url: &str,
    form: Option<Vec<(String, String)>>,
    accept: &str,
    timeout_secs: u64,
    max_bytes: usize,
    allow_private: bool,
) -> Result<String, String> {
    let initial = validate_url(url)?;
    let timeout_secs = timeout_secs.clamp(1, MAX_TIMEOUT_SECS);
    // `allow_private` here is NOT model-controlled — it is set from user config
    // (a self-hosted SearxNG on localhost/LAN is explicitly trusted); the public
    // DuckDuckGo fallback keeps the guard on. Search providers fingerprint the
    // rustls ClientHello and serve a challenge page, so the search path opts into
    // the platform-native TLS backend (see `fetch_guarded`).
    let guarded = fetch_guarded(GuardedRequest {
        initial,
        accept: Some(accept.to_string()),
        accept_language: true,
        user_agent: SEARCH_USER_AGENT,
        use_native_tls: true,
        timeout_secs,
        max_bytes,
        allow_private,
        form,
    })
    .await?;
    if !(200..300).contains(&guarded.status) {
        return Err(format!("search provider returned HTTP {}", guarded.status));
    }
    Ok(String::from_utf8_lossy(&guarded.body).into_owned())
}

/// Parameters for a single SSRF-guarded fetch.
struct GuardedRequest {
    initial: reqwest::Url,
    accept: Option<String>,
    accept_language: bool,
    user_agent: &'static str,
    use_native_tls: bool,
    timeout_secs: u64,
    max_bytes: usize,
    allow_private: bool,
    /// When present, issue a POST with these fields as an
    /// `application/x-www-form-urlencoded` body instead of a GET. Used by the
    /// research search path: `DuckDuckGo`'s html/lite endpoints now serve an
    /// anomaly/empty page to a plain GET and only return results for a POST whose
    /// body carries the query.
    form: Option<Vec<(String, String)>>,
}

/// Outcome of a guarded fetch: final hop, status, content-type, capped body.
struct GuardedResponse {
    final_url: reqwest::Url,
    status: u16,
    content_type: Option<String>,
    body: Vec<u8>,
    truncated: bool,
}

const MAX_REDIRECTS: usize = 5;

/// The SSRF-safe fetch core shared by [`fetch`] and [`fetch_text`].
///
/// Closes the structural SSRF holes in one place: for every hop it resolves the
/// host once, rejects any resolved private IP (unless `allow_private`), and pins
/// the connection to exactly those vetted IPs via `ClientBuilder::resolve_to_addrs`
/// so reqwest cannot re-resolve to a different (private) address at connect time
/// (H2 rebinding). Redirects are followed manually — `Policy::none()` — so each
/// `Location` is re-validated with full DNS before the next request (H3). The body
/// is read as a stream and cut off the moment it exceeds `max_bytes` (M3), and an
/// oversized `Content-Length` is rejected up front.
async fn fetch_guarded(req: GuardedRequest) -> Result<GuardedResponse, String> {
    let mut current = req.initial;
    let mut redirects = 0usize;

    loop {
        // Resolve + validate this hop, and obtain the exact IPs to pin to.
        let pinned = resolve_and_screen(&current, req.allow_private).await?;
        let host = current
            .host_str()
            .ok_or_else(|| "WebFetch URL must include a host".to_string())?
            .to_string();

        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(req.timeout_secs))
            .redirect(Policy::none())
            .user_agent(req.user_agent)
            .resolve_to_addrs(&host, &pinned);
        if req.use_native_tls {
            builder = builder.use_native_tls();
        }
        let client = builder.build().map_err(|error| error.to_string())?;

        // POST the form body only on the FIRST hop; a redirect is followed as a
        // GET (standard 303 semantics) so we never replay the body to a new host.
        let mut request = match &req.form {
            Some(fields) if redirects == 0 => client.post(current.clone()).form(fields),
            _ => client.get(current.clone()),
        };
        if let Some(accept) = &req.accept {
            request = request.header(reqwest::header::ACCEPT, accept.clone());
        }
        if req.accept_language {
            request = request.header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9");
        }

        let response = timeout(Duration::from_secs(req.timeout_secs + 5), request.send())
            .await
            .map_err(|_| "WebFetch request timed out".to_string())?
            .map_err(|error| error.to_string())?;

        let status = response.status();
        // Follow redirects ourselves so each target is re-screened with DNS.
        if status.is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| "WebFetch redirect missing Location header".to_string())?;
            let next = current
                .join(location)
                .map_err(|error| format!("WebFetch redirect URL invalid: {error}"))?;
            match next.scheme() {
                "http" | "https" => {}
                scheme => {
                    return Err(format!("WebFetch redirect to unsupported scheme: {scheme}"));
                }
            }
            redirects += 1;
            if redirects > MAX_REDIRECTS {
                return Err("WebFetch exceeded the redirect limit".to_string());
            }
            current = next;
            continue;
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(ToString::to_string);
        // Reject an oversized declared length before reading a single byte.
        if let Some(declared) = response.content_length() {
            if declared > req.max_bytes as u64 * 8 {
                return Err("WebFetch response Content-Length exceeds the limit".to_string());
            }
        }

        let (body, truncated) = read_capped(response, req.max_bytes).await?;
        return Ok(GuardedResponse {
            final_url: current,
            status: status.as_u16(),
            content_type,
            body,
            truncated,
        });
    }
}

/// Stream a response body, stopping the moment it exceeds `max_bytes`. The cap is
/// enforced *during* the read so a hostile/endless body never fully materializes.
async fn read_capped(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<(Vec<u8>, bool), String> {
    let mut body = Vec::with_capacity(max_bytes.min(64 * 1024));
    let mut truncated = false;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| error.to_string())?;
        let remaining = max_bytes.saturating_sub(body.len());
        if chunk.len() >= remaining {
            body.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        body.extend_from_slice(&chunk);
    }
    Ok((body, truncated))
}

/// Resolve a URL's host to socket addresses, reject private targets (unless
/// explicitly allowed), and return the vetted IPs to pin the connection to.
async fn resolve_and_screen(
    url: &reqwest::Url,
    allow_private: bool,
) -> Result<Vec<SocketAddr>, String> {
    let host = url
        .host_str()
        .ok_or_else(|| "WebFetch URL must include a host".to_string())?
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "WebFetch URL has no usable port".to_string())?;

    if !allow_private && is_localhost_name(&host) {
        return Err("WebFetch blocks localhost/private hosts by default".to_string());
    }

    // A bare IP literal needs no DNS — screen it directly.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !allow_private && is_private_ip(ip) {
            return Err(
                "WebFetch blocks localhost/private network addresses by default".to_string(),
            );
        }
        return Ok(vec![SocketAddr::new(ip, port)]);
    }

    let lookup_host = host.clone();
    let addresses = tokio::task::spawn_blocking(move || {
        (lookup_host.as_str(), port)
            .to_socket_addrs()
            .map(Iterator::collect::<Vec<_>>)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;

    if addresses.is_empty() {
        return Err("WebFetch host did not resolve to any address".to_string());
    }
    if !allow_private && addresses.iter().any(|socket| is_private_ip(socket.ip())) {
        return Err("WebFetch blocks localhost/private network addresses by default".to_string());
    }
    Ok(addresses)
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

fn is_localhost_name(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost")
}

const fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        // 100.64.0.0/10 — CGNAT (RFC 6598); not "private" per std but never public.
        || (ip.octets()[0] == 100 && (ip.octets()[1] & 0xc0) == 0x40)
        // 192.0.0.0/24 — IETF protocol assignments (RFC 6890).
        || (ip.octets()[0] == 192 && ip.octets()[1] == 0 && ip.octets()[2] == 0)
        // 198.18.0.0/15 — benchmarking (RFC 2544).
        || (ip.octets()[0] == 198 && (ip.octets()[1] & 0xfe) == 18)
}

const fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_private_ipv4(ip),
        IpAddr::V6(ip) => {
            // IPv4-mapped (`::ffff:a.b.c.d`) and IPv4-compatible (`::a.b.c.d`)
            // literals tunnel an IPv4 destination through an `IpAddr::V6`; route the
            // embedded address through the IPv4 predicate so `[::ffff:127.0.0.1]`
            // can't slip past the loopback/RFC1918 checks. `const` forbids
            // `to_ipv4_mapped`, so decode the mapping from the raw segments.
            if let Some(v4) = embedded_ipv4(ip.segments()) {
                return is_private_ipv4(v4);
            }
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.segments()[0] & 0xffc0 == 0xfe80
        }
    }
}

/// Extract an IPv4 address embedded in an IPv6 literal: both IPv4-mapped
/// (`::ffff:0:0/96`) and the deprecated IPv4-compatible (`::/96`, excluding the
/// `::`/`::1` specials) forms. Returns `None` for genuine IPv6 addresses.
const fn embedded_ipv4(segments: [u16; 8]) -> Option<Ipv4Addr> {
    // The high 80 bits (segments 0..=4) are zero for both embedded forms; segment 5
    // is the discriminator (0xffff = mapped, 0x0000 = compatible) and 6..=7 hold the
    // IPv4 octets.
    let high_zero = segments[0] == 0
        && segments[1] == 0
        && segments[2] == 0
        && segments[3] == 0
        && segments[4] == 0;
    let marker = segments[5];
    let high_octets = segments[6];
    let low_octets = segments[7];
    let v4 = Ipv4Addr::new(
        (high_octets >> 8) as u8,
        (high_octets & 0xff) as u8,
        (low_octets >> 8) as u8,
        (low_octets & 0xff) as u8,
    );
    let is_mapped = marker == 0xffff; // ::ffff:a.b.c.d
                                      // ::a.b.c.d — exclude the `::`/`::1` specials (not real embedded IPv4).
    let is_compatible = marker == 0 && (high_octets != 0 || low_octets > 1);
    if high_zero && (is_mapped || is_compatible) {
        Some(v4)
    } else {
        None
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
    fn private_ip_guard_blocks_ipv4_mapped_ipv6_bypass() {
        // The SSRF bypass: IPv4-mapped/compatible IPv6 literals carrying a private
        // IPv4 destination must be rejected, not treated as opaque IPv6.
        assert!(is_private_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_private_ip("::ffff:10.0.0.1".parse().unwrap()));
        assert!(is_private_ip("::ffff:192.168.1.1".parse().unwrap()));
        assert!(is_private_ip("::127.0.0.1".parse().unwrap()));
        // A mapped *public* address is still allowed (don't over-block).
        assert!(!is_private_ip("::ffff:8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn private_ip_guard_blocks_cgnat_and_benchmark_ranges() {
        assert!(is_private_ip("100.64.0.1".parse().unwrap())); // CGNAT
        assert!(is_private_ip("198.18.0.1".parse().unwrap())); // benchmarking
        assert!(is_private_ip("192.0.0.1".parse().unwrap())); // IETF protocol
        assert!(!is_private_ip("100.128.0.1".parse().unwrap())); // outside CGNAT /10
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
