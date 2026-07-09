use std::net::IpAddr;
use std::path::Path;

pub fn is_private_or_loopback_ip(ip: &IpAddr) -> bool {
    if ip.is_loopback() {
        return true;
    }
    if ip.is_unspecified() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            octets[0] == 10
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                || (octets[0] == 169 && octets[1] == 254)
        }
        IpAddr::V6(v6) => {
            v6.octets()[0] & 0xfe == 0xfc
        }
    }
}

pub fn validate_domain_list(domains: &str) -> Result<(), String> {
    for domain in domains.split(',') {
        let domain = domain.trim();
        if domain.is_empty() {
            continue;
        }
        if let Ok(ip) = domain.parse::<IpAddr>() {
            if is_private_or_loopback_ip(&ip) {
                return Err(format!(
                    "Domain '{domain}' is a private/local IP address and is not allowed. \
                     Use a public domain or approve it in AI preferences."
                ));
            }
            continue;
        }
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
        if lower == "localhost" || lower == "local" || lower.starts_with("localhost.") {
            return Err(format!(
                "Domain '{domain}' resolves to localhost and is not allowed. Use a public domain."
            ));
        }
    }
    Ok(())
}

#[allow(clippy::case_sensitive_file_extension_comparisons)]
pub fn validate_proxy_url(proxy: &str) -> Result<(), String> {
    let trimmed = proxy.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
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
    let host_part = rest.split('/').next().unwrap_or(rest);
    let host = host_part.rfind(':').map_or(host_part, |idx| {
        let candidate = &host_part[..idx];
        if candidate.is_empty() {
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

pub fn validate_provider(provider: &str) -> Result<(), String> {
    let trimmed = provider.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Ok(());
    }
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

const SESSION_PREFIX: &str = "aspect-";
const SESSION_BODY_MAX: usize = 48;

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
    let body = trimmed
        .strip_prefix(SESSION_PREFIX)
        .map_or(trimmed, |rest| rest.trim_matches('-'));
    if body.is_empty() {
        return format!("{SESSION_PREFIX}default");
    }
    let bounded = &body[..body.len().min(SESSION_BODY_MAX)];
    format!("{SESSION_PREFIX}{bounded}")
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
