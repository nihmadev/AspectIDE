use std::time::Duration;

use crate::process::run_json;
use crate::resolver::{resolve_binary, DEFAULT_MAX_OUTPUT, DEFAULT_TIMEOUT_SECS, MAX_OUTPUT_CAP, MAX_TIMEOUT_SECS};
use crate::types::{AgentBrowserInvokeRequest, AgentBrowserInvokeResponse, InvokeOptions};
use crate::validate::{sanitize_session, validate_domain_list, validate_provider, validate_proxy_url};

pub async fn invoke(
    request: AgentBrowserInvokeRequest,
) -> Result<AgentBrowserInvokeResponse, String> {
    let binary = resolve_binary()?;
    let session = sanitize_session(&request.session);
    if request.args.is_empty() {
        return Err("Browser invoke requires at least one command argument.".to_string());
    }

    if request.allow_file_access == Some(true) {
        return Err(
            "agent-browser --allow-file-access is denied by default for security. \
             Enable it in your AI preferences if you trust the session domain."
                .to_string(),
        );
    }
    if request.ignore_https_errors == Some(true) {
        return Err(
            "agent-browser --ignore-https-errors is denied by default for security. \
             Enable it in your AI preferences if you trust the session domain."
                .to_string(),
        );
    }

    if let Some(ref domains) = request.allowed_domains {
        validate_domain_list(domains)?;
    }

    if let Some(ref proxy) = request.proxy {
        validate_proxy_url(proxy)?;
    }

    if let Some(ref provider) = request.provider {
        validate_provider(provider)?;
    }

    let timeout_secs = request
        .timeout_secs
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(5, MAX_TIMEOUT_SECS);
    let max_output = request
        .max_output
        .unwrap_or(DEFAULT_MAX_OUTPUT)
        .clamp(2_000, MAX_OUTPUT_CAP);

    let arg_refs: Vec<&str> = request.args.iter().map(String::as_str).collect();
    let options = InvokeOptions {
        session: session.clone(),
        headed: request.headed,
        allowed_domains: request.allowed_domains.clone(),
        max_output,
        session_name: request.session_name.clone(),
        profile: request.profile.clone(),
        state_path: request.state_path.clone(),
        content_boundaries: request.content_boundaries,
        ignore_https_errors: None,
        allow_file_access: None,
        provider: request.provider.clone(),
        proxy: request.proxy.clone(),
        cwd: request.cwd.clone(),
    };
    let mut response = run_json(&binary, Some(options.clone()), &arg_refs, timeout_secs).await?;

    if !response.success && is_retryable_daemon_conflict(&response.text) {
        tokio::time::sleep(Duration::from_millis(750)).await;
        response = run_json(&binary, Some(options), &arg_refs, timeout_secs).await?;
    }

    Ok(AgentBrowserInvokeResponse {
        session,
        command: request.args.join(" "),
        success: response.success,
        data: response.data,
        text: response.text,
        elapsed_ms: response.elapsed_ms,
        truncated: response.truncated,
        exit_code: response.exit_code,
    })
}

fn is_retryable_daemon_conflict(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("retry the command")
        || lower.contains("daemon version mismatch")
        || lower.contains("started concurrently with different daemon configuration")
}
