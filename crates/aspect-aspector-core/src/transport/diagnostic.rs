use std::time::{Duration, Instant};

use crate::protocol;

use super::auth::apply_auth;
use super::endpoints::completion_endpoint;
use super::retry::stream_response_error;
use super::types::{AiChatCompletionRequest, AiProviderDiagnosticResponse};

/// Test an AI provider connection with a lightweight request.
pub async fn provider_diagnostic(
    request: AiChatCompletionRequest,
) -> Result<AiProviderDiagnosticResponse, String> {
    let anthropic = protocol::is_anthropic(&request.protocol);
    let endpoint = if anthropic {
        protocol::messages_endpoint(&request.base_url)?
    } else {
        completion_endpoint(&request.base_url)?
    };
    let payload = if anthropic {
        protocol::to_anthropic_request(&request.payload)
    } else {
        request.payload.clone()
    };
    let model = request
        .payload
        .get("model")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let started = Instant::now();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?;
    let builder = client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&payload);
    let api_key = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    let builder = apply_auth(builder, anthropic, api_key);

    match tokio::time::timeout(Duration::from_secs(25), builder.send()).await {
        Ok(Ok(response)) => {
            let status = response.status().as_u16();
            let error = if status >= 400 {
                let text = response.text().await.unwrap_or_default();
                Some(stream_response_error(status, &text))
            } else {
                None
            };
            Ok(AiProviderDiagnosticResponse {
                ok: status < 400,
                status: Some(status),
                latency_ms: started.elapsed().as_millis(),
                error,
                model,
                base_url: request.base_url,
            })
        }
        Ok(Err(error)) => Ok(AiProviderDiagnosticResponse {
            ok: false,
            status: None,
            latency_ms: started.elapsed().as_millis(),
            error: Some(error.to_string()),
            model,
            base_url: request.base_url,
        }),
        Err(_) => Ok(AiProviderDiagnosticResponse {
            ok: false,
            status: None,
            latency_ms: started.elapsed().as_millis(),
            error: Some("AI provider diagnostic timed out".to_string()),
            model,
            base_url: request.base_url,
        }),
    }
}

/// Generate an OpenAI-shape embedding vector for `input`.
#[allow(clippy::cast_possible_truncation)]
pub async fn embeddings(
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
    input: &str,
) -> Result<Vec<f32>, String> {
    let endpoint = super::endpoints::embeddings_endpoint(base_url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;
    let key = api_key.map(str::trim).filter(|value| !value.is_empty());
    let mut builder = client
        .post(&endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&serde_json::json!({ "model": model, "input": input }));
    if let Some(key) = key {
        builder = builder.bearer_auth(key);
    }
    let response = builder
        .send()
        .await
        .map_err(|error| format!("Failed to reach {endpoint}: {error}"))?;
    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("Invalid embeddings response: {error}"))?;
    if !status.is_success() {
        let detail = body
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("provider returned an error");
        return Err(format!("Embeddings request failed ({status}): {detail}"));
    }
    let vector = body
        .get("data")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("embedding"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "Embeddings response had no data[0].embedding array".to_string())?
        .iter()
        .filter_map(|value| value.as_f64().map(|v| v as f32))
        .collect();
    Ok(vector)
}
