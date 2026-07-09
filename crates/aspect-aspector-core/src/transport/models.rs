use std::time::Duration;

use serde_json::Value;

use super::endpoints::models_endpoint;

/// Fetch a provider's available model ids from its OpenAI-compatible `/models`
/// endpoint. Returns the raw `id` strings exactly as the provider reports them.
pub async fn list_provider_models(
    base_url: String,
    api_key: Option<String>,
) -> Result<Vec<String>, String> {
    let endpoint = models_endpoint(&base_url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?;
    let key = api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut builder = client
        .get(&endpoint)
        .header(reqwest::header::ACCEPT, "application/json");
    if let Some(key) = key {
        builder = builder.bearer_auth(key);
    }
    let response = builder
        .send()
        .await
        .map_err(|error| format!("Failed to reach {endpoint}: {error}"))?;
    let status = response.status();
    let body: Value = response
        .json()
        .await
        .map_err(|error| format!("Invalid models response: {error}"))?;
    if !status.is_success() {
        let detail = body
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("provider returned an error");
        return Err(format!("Models request failed ({status}): {detail}"));
    }
    let items = body
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| body.as_array())
        .ok_or_else(|| "Models response had no `data` array".to_string())?;
    let ids: Vec<String> = items
        .iter()
        .filter_map(|item| {
            item.get("id")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string)
        })
        .collect();
    Ok(ids)
}
