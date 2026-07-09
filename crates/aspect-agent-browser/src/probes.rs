use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use crate::resolver::{agent_browser_command, READ_VERSION_TIMEOUT_SECS};
use crate::types::InvokeOptions;
use crate::process::run_json;

pub const fn session_invoke_options(session: String, max_output: usize) -> InvokeOptions {
    InvokeOptions {
        session,
        headed: None,
        allowed_domains: None,
        max_output,
        session_name: None,
        profile: None,
        state_path: None,
        content_boundaries: None,
        ignore_https_errors: None,
        allow_file_access: None,
        provider: None,
        proxy: None,
        cwd: None,
    }
}

pub async fn read_version(binary: &Path) -> Result<String, String> {
    let mut command = agent_browser_command(binary);
    command.arg("--version");
    command.stdin(Stdio::null());
    command.kill_on_drop(true);
    let output = tokio::time::timeout(
        Duration::from_secs(READ_VERSION_TIMEOUT_SECS),
        command.output(),
    )
    .await
    .map_err(|_| "agent-browser --version timed out".to_string())?
    .map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        Err("agent-browser --version returned empty output".to_string())
    } else {
        Ok(text)
    }
}

pub async fn list_sessions(binary: &Path) -> Result<Vec<String>, String> {
    let response = run_json(binary, None, &["session", "list", "--json"], 20).await?;
    let sessions = response
        .data
        .get("sessions")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| match item {
                    serde_json::Value::String(value) => Some(value.clone()),
                    serde_json::Value::Object(map) => map
                        .get("name")
                        .or_else(|| map.get("id"))
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(sessions)
}
