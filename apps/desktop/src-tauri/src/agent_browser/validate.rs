//! Input validation and sanitisation for browser-automation requests: domain
//! allowlists, proxy URLs, provider names, session-id sanitisation, and MIME
//! type inference. Pure functions — no I/O.

use std::net::IpAddr;
use std::path::Path;

/// Check if IP is loopback, private, or unspecified (stable equivalent of
/// the nightly-only `IpAddr::is_private()` / `is_unspecified()`).
pub fn is_private_or_loopback_ip(ip: &IpAddr) -> bool {
    if ip.is_loopback() {
        return true;
    }
    if ip.is_unspecified() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            // RFC 1918: 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
            // Carrier-grade NAT: 100.64.0.0/10
            // Link-local: 169.254.0.0/16
            let octets = v4.octets();
            octets[0] == 10
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                || (octets[0] == 169 && octets[1] == 254)
        }
        IpAddr::V6(v6) => {
            // Unique local: fc00::/7
            v6.octets()[0] & 0xfe == 0xfc
        }
    }
}

/// Validate a comma-separated list of domains. Rejects private/loopback IPs
/// and internal-only hostnames.
pub fn validate_domain_list(domains: &str) -> Result<(), String> {
    for domain in domains.split(',') {
        let domain = domain.trim();
        if domain.is_empty() {
            continue;
        }
        // Reject raw IPs in private ranges.
        if let Ok(ip) = domain.parse::<IpAddr>() {
            if is_private_or_loopback_ip(&ip) {
                return Err(format!(
                    "Domain '{domain}' is a private/local IP address and is not allowed. \
                     Use a public domain or approve it in AI preferences."
                ));
            }
            continue;
        }
        // Reject internal-only TLDs.
        let lower = domain.to_ascii_lowercase();
        for bad_tld in &[
            ".local",
            ".internal",
            ".localhost",
            ".localdomain",
            ".lan",
            ".intranet",
        ] {
            if lower == *bad_tld || lower.ends_with(bad_tld) {
                return Err(format!(
                    "Domain '{domain}' uses an internal-only TLD '{bad_tld}' and is not allowed. \
                     Use a public domain."
                ));
            }
        }
        // Reject hostnames that are just "localhost" or "local".
        if lower == "localhost" || lower == "local" || lower.starts_with("localhost.") {
            return Err(format!(
                "Domain '{domain}' resolves to localhost and is not allowed. Use a public domain."
            ));
        }
    }
    Ok(())
}

/// Validate a proxy URL (http/https scheme, public hosts only, no private IPs).
/// Uses basic string parsing to avoid pulling in the `url` crate.
// `.ends_with(".local")` checks a hostname suffix (already lowercased), not a
// file extension — the lint's case-insensitive suggestion does not apply.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
pub fn validate_proxy_url(proxy: &str) -> Result<(), String> {
    let trimmed = proxy.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    // Require http:// or https:// prefix.
    let lower = trimmed.to_ascii_lowercase();
    let rest = if let Some(rest) = lower.strip_prefix("https://") {
        rest
    } else if let Some(rest) = lower.strip_prefix("http://") {
        rest
    } else {
        return Err(format!(
            "Invalid proxy URL '{trimmed}': scheme must be http or https."
        ));
    };
    // Split host:port from the rest.
    let host_part = rest.split('/').next().unwrap_or(rest);
    let host = host_part.rfind(':').map_or(host_part, |idx| {
        // Could be port, or IPv6 address with brackets.
        let candidate = &host_part[..idx];
        if candidate.is_empty() {
            // IPv6 like [::1]:8080 — take the bracketed part.
            host_part
                .rfind(']')
                .map_or(host_part, |end| &host_part[1..end])
        } else {
            candidate
        }
    });
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return Err("Invalid proxy URL: empty host.".to_string());
    }
    // Reject private IP hosts.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_or_loopback_ip(&ip) {
            return Err(format!(
                "Proxy URL host '{host}' is a private/local address and is not allowed."
            ));
        }
    } else {
        let host_lower = host.to_ascii_lowercase();
        if host_lower == "localhost"
            || host_lower.ends_with(".local")
            || host_lower.ends_with(".internal")
        {
            return Err(format!(
                "Proxy URL host '{host}' is an internal hostname and is not allowed."
            ));
        }
    }
    Ok(())
}

/// Validate the provider identifier against an allowlist of known safe providers.
pub fn validate_provider(provider: &str) -> Result<(), String> {
    let trimmed = provider.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Ok(());
    }
    // Allowlisted known-safe providers. Unknown providers are rejected.
    let allowed = [
        "chrome",
        "browserless",
        "browserbase",
        "kernel",
        "agentcore",
        "ios",
    ];
    if allowed.contains(&trimmed.as_str()) {
        return Ok(());
    }
    Err(format!(
        "Provider '{}' is not in the allowlist of known providers. \
         Allowed: {}",
        provider,
        allowed.join(", ")
    ))
}

#[must_use]
pub fn sanitize_session(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "lux-default".to_string()
    } else {
        format!("lux-{}", &trimmed[..trimmed.len().min(48)])
    }
}

pub fn mime_type_for_path(path: &Path) -> String {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg" | "jpe") => "image/jpeg".to_string(),
        Some("webp") => "image/webp".to_string(),
        Some("gif") => "image/gif".to_string(),
        _ => "image/png".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{sanitize_session, validate_domain_list, validate_provider, validate_proxy_url};

    #[test]
    fn sanitize_session_prefixes_and_filters() {
        assert_eq!(sanitize_session("chat-123"), "lux-chat-123");
        assert_eq!(sanitize_session(""), "lux-default");
    }

    #[test]
    fn domain_validation_rejects_private_ip() {
        assert!(validate_domain_list("example.com").is_ok());
        assert!(validate_domain_list("example.com,test.org").is_ok());
        assert!(validate_domain_list("127.0.0.1").is_err());
        assert!(validate_domain_list("192.168.1.1").is_err());
        assert!(validate_domain_list("10.0.0.5").is_err());
        assert!(validate_domain_list("172.16.0.1").is_err());
        assert!(validate_domain_list("localhost").is_err());
        assert!(validate_domain_list("internal.local").is_err());
    }

    #[test]
    fn proxy_validation_rejects_internal() {
        assert!(validate_proxy_url("http://proxy.example.com:8080").is_ok());
        assert!(validate_proxy_url("https://proxy.example.com").is_ok());
        assert!(validate_proxy_url("http://127.0.0.1:8080").is_err());
        assert!(validate_proxy_url("file:///tmp/proxy").is_err());
        assert!(validate_proxy_url("").is_ok());
    }

    #[test]
    fn provider_validation_allowlists_known() {
        assert!(validate_provider("chrome").is_ok());
        assert!(validate_provider("browserless").is_ok());
        assert!(validate_provider("browserbase").is_ok());
        assert!(validate_provider("").is_ok());
        assert!(validate_provider("evil-provider").is_err());
        assert!(validate_provider("CHROME").is_ok());
    }
}
