use std::time::Duration;

use serde_json::Value;
use tokio::time::timeout;

use crate::protocol;

use super::auth::apply_auth;
use super::endpoints::{is_transient_reqwest_error, is_transient_status, CHAT_TIMEOUT_SECS, NETWORK_RETRY_BUDGET, TCP_KEEPALIVE_SECS};
use super::retry::{backoff_delay, emit_retry, response_error, retry_budget_for_status, retry_reason_for_error, retry_reason_for_status};
use super::types::{AiChatCompletionRequest, AiChatCompletionResponse, RetryNotice};

pub async fn completion<R>(
    request: AiChatCompletionRequest,
    mut on_retry: R,
) -> Result<AiChatCompletionResponse, String>
where
    R: FnMut(RetryNotice),
{
    let anthropic = protocol::is_anthropic(&request.protocol);
    let endpoint = if anthropic {
        protocol::messages_endpoint(&request.base_url)?
    } else {
        super::endpoints::completion_endpoint(&request.base_url)?
    };
    let payload = if anthropic {
        protocol::to_anthropic_request(&request.payload)
    } else {
        request.payload.clone()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CHAT_TIMEOUT_SECS))
        .tcp_keepalive(Duration::from_secs(TCP_KEEPALIVE_SECS))
        .build()
        .map_err(|error| error.to_string())?;
    let api_key = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(ToString::to_string);

    let mut attempt: u32 = 0;
    loop {
        let builder = client
            .post(endpoint.as_str())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "application/json")
            .json(&payload);
        let builder = apply_auth(builder, anthropic, api_key.as_deref());

        let send_result = timeout(Duration::from_secs(CHAT_TIMEOUT_SECS + 5), builder.send()).await;
        let response = match send_result {
            Err(_) => {
                if attempt < NETWORK_RETRY_BUDGET {
                    let delay = backoff_delay(attempt);
                    emit_retry(&mut on_retry, attempt, NETWORK_RETRY_BUDGET, "timeout", "request timed out", delay);
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                    continue;
                }
                return Err("AI request timed out".to_string());
            }
            Ok(Err(error)) => {
                if attempt < NETWORK_RETRY_BUDGET && is_transient_reqwest_error(&error) {
                    let delay = backoff_delay(attempt);
                    emit_retry(&mut on_retry, attempt, NETWORK_RETRY_BUDGET, retry_reason_for_error(&error), "connection failed", delay);
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                    continue;
                }
                return Err(error.to_string());
            }
            Ok(Ok(response)) => response,
        };

        let status = response.status().as_u16();
        if status >= 400 {
            let budget = retry_budget_for_status(status);
            if attempt < budget && is_transient_status(status) {
                let delay = super::retry::transient_retry_delay(status, response.headers(), attempt);
                emit_retry(&mut on_retry, attempt, budget, retry_reason_for_status(status), format!("HTTP {status}"), delay);
                tokio::time::sleep(delay).await;
                attempt += 1;
                continue;
            }
            let body = response.json::<Value>().await.unwrap_or(Value::Null);
            return Err(response_error(status, &body));
        }

        let text = response
            .text()
            .await
            .map_err(|error| format!("Failed to read AI provider response: {error}"))?;
        let body = serde_json::from_str::<Value>(&text).map_err(|_| {
            let preview: String = text.trim().chars().take(180).collect();
            if preview.is_empty() {
                "AI provider returned an empty response. Check the model id, base URL, and that the endpoint is an OpenAI-compatible /chat/completions.".to_string()
            } else {
                format!("AI provider returned a non-JSON response (is the base URL correct?): {preview}")
            }
        })?;
        let body = if anthropic {
            protocol::from_anthropic_response(&body)
        } else {
            body
        };
        return Ok(AiChatCompletionResponse { status, body });
    }
}
