/// Build the `/models` listing endpoint from a provider base URL.
pub fn models_endpoint(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("AI provider base URL is empty".to_string());
    }
    let url = reqwest::Url::parse(trimmed)
        .map_err(|error| format!("Invalid AI provider URL: {error}"))?;
    match url.scheme() {
        "http" | "https" => {}
        scheme => return Err(format!("Unsupported AI provider URL scheme: {scheme}")),
    }
    let text = url.as_str().trim_end_matches('/');
    let root = text.strip_suffix("/chat/completions").unwrap_or(text);
    Ok(format!("{root}/models"))
}

/// Build the `/embeddings` endpoint from a provider base URL.
pub fn embeddings_endpoint(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("AI provider base URL is empty".to_string());
    }
    let url = reqwest::Url::parse(trimmed)
        .map_err(|error| format!("Invalid AI provider URL: {error}"))?;
    match url.scheme() {
        "http" | "https" => {}
        scheme => return Err(format!("Unsupported AI provider URL scheme: {scheme}")),
    }
    let text = url.as_str().trim_end_matches('/');
    let root = text.strip_suffix("/chat/completions").unwrap_or(text);
    Ok(format!("{root}/embeddings"))
}

/// Build the `/chat/completions` endpoint from a provider base URL.
pub fn completion_endpoint(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("AI provider base URL is empty".to_string());
    }
    let url = reqwest::Url::parse(trimmed)
        .map_err(|error| format!("Invalid AI provider URL: {error}"))?;
    match url.scheme() {
        "http" | "https" => {}
        scheme => return Err(format!("Unsupported AI provider URL scheme: {scheme}")),
    }
    let text = url.as_str().trim_end_matches('/');
    if text.ends_with("/chat/completions") {
        Ok(text.to_string())
    } else {
        Ok(format!("{text}/chat/completions"))
    }
}

// ── Timeout constants ──

pub const CHAT_TIMEOUT_SECS: u64 = 180;
pub const STREAM_CONNECT_TIMEOUT_SECS: u64 = 30;
pub const TCP_KEEPALIVE_SECS: u64 = 30;
pub const NETWORK_RETRY_BUDGET: u32 = 9;
pub const MAX_TRANSIENT_RETRIES: u32 = 9;
pub const MAX_RETRY_DELAY_SECS: u64 = 30;
pub const MAX_SSE_BUFFER: usize = 8 * 1024 * 1024;

/// Transient HTTP statuses worth one bounded automatic retry.
pub const fn is_transient_status(status: u16) -> bool {
    matches!(status, 403 | 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

/// Network-level reqwest errors that are safe to retry.
pub fn is_transient_reqwest_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}
