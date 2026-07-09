use crate::protocol;

/// Attach provider auth to a request builder.
pub fn apply_auth(
    builder: reqwest::RequestBuilder,
    anthropic: bool,
    api_key: Option<&str>,
) -> reqwest::RequestBuilder {
    if anthropic {
        let builder = builder.header("anthropic-version", protocol::ANTHROPIC_VERSION);
        return match api_key {
            Some(key) => builder.header("x-api-key", key),
            None => builder,
        };
    }
    match api_key {
        Some(key) => builder.bearer_auth(key),
        None => builder,
    }
}
