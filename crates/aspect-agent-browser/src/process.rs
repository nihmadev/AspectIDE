use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;

use crate::resolver::{agent_browser_command, DEFAULT_MAX_OUTPUT};
use crate::types::{InvokeOptions, ParsedCliResponse};

pub async fn run_json(
    binary: &Path,
    options: Option<InvokeOptions>,
    args: &[&str],
    timeout_secs: u64,
) -> Result<ParsedCliResponse, String> {
    let started = Instant::now();
    let mut command = agent_browser_command(binary);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);
    command.arg("--json");

    if let Some(cwd) = options.as_ref().and_then(|o| o.cwd.as_deref()) {
        let simplified = dunce::simplified(Path::new(cwd));
        if simplified.is_dir() {
            command.current_dir(simplified);
        }
    }

    let max_output = options
        .as_ref()
        .map_or(DEFAULT_MAX_OUTPUT, |value| value.max_output);
    if let Some(options) = options.as_ref() {
        command.arg("--session").arg(&options.session);
        if options.headed == Some(true) {
            command.arg("--headed");
        }
        if let Some(domains) = options
            .allowed_domains
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            command.arg("--allowed-domains").arg(domains);
        }
        if let Some(session_name) = options
            .session_name
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            command.arg("--session-name").arg(session_name);
        }
        if let Some(profile) = options
            .profile
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            command.arg("--profile").arg(profile);
        }
        if let Some(state_path) = options
            .state_path
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            command.arg("--state").arg(state_path);
        }
        if options.content_boundaries == Some(true) {
            command.arg("--content-boundaries");
        }
        if options.ignore_https_errors == Some(true) {
            command.arg("--ignore-https-errors");
        }
        if options.allow_file_access == Some(true) {
            command.arg("--allow-file-access");
        }
        if let Some(provider) = options
            .provider
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            command.arg("--provider").arg(provider.trim());
        }
        if let Some(proxy) = options
            .proxy
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            command.arg("--proxy").arg(proxy.trim());
        }
        command.env("AGENT_BROWSER_MAX_OUTPUT", options.max_output.to_string());
    }

    command.args(args);

    let mut child = command
        .spawn()
        .map_err(|error| format!("Failed to start agent-browser: {error}"))?;

    let stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| "no stdout pipe".to_string())?;
    let stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| "no stderr pipe".to_string())?;

    let stdout_budget = max_output.saturating_add(4096);
    let stderr_budget: usize = 32_768;

    let shared_stdout = Arc::new(tokio::sync::Mutex::new(BoundedRead::default()));
    let shared_stderr = Arc::new(tokio::sync::Mutex::new(BoundedRead::default()));
    let stdout_sink = Arc::clone(&shared_stdout);
    let mut stdout_task =
        tokio::spawn(async move { drain_into(stdout_pipe, stdout_budget, &stdout_sink).await });
    let stderr_sink = Arc::clone(&shared_stderr);
    let mut stderr_task =
        tokio::spawn(async move { drain_into(stderr_pipe, stderr_budget, &stderr_sink).await });

    let Ok(status_result) = tokio::time::timeout(
        Duration::from_secs(timeout_secs.saturating_add(5)),
        child.wait(),
    )
    .await
    else {
        let _ = child.start_kill();
        stdout_task.abort();
        stderr_task.abort();
        return Err(format!("agent-browser timed out after {timeout_secs}s"));
    };

    const PIPE_DRAIN_GRACE_SECS: u64 = 2;
    if tokio::time::timeout(Duration::from_secs(PIPE_DRAIN_GRACE_SECS), &mut stdout_task)
        .await
        .is_err()
    {
        stdout_task.abort();
    }
    if tokio::time::timeout(Duration::from_secs(PIPE_DRAIN_GRACE_SECS), &mut stderr_task)
        .await
        .is_err()
    {
        stderr_task.abort();
    }
    let stdout_read = std::mem::take(&mut *shared_stdout.lock().await);
    let stderr_read = std::mem::take(&mut *shared_stderr.lock().await);
    let stdout_bytes = stdout_read.bytes;
    let stderr_bytes = stderr_read.bytes;
    let clipped_at_pipe = stdout_read.clipped;
    let exit_code = status_result.map_or(None, |status| status.code());

    let stdout = String::from_utf8_lossy(&stdout_bytes).trim().to_string();
    let stderr = String::from_utf8_lossy(&stderr_bytes).trim().to_string();

    if stdout.is_empty() && !stderr.is_empty() && exit_code != Some(0) {
        return Err(stderr);
    }

    let parsed_json = serde_json::from_str::<serde_json::Value>(&stdout).ok();
    let success = parsed_json
        .as_ref()
        .and_then(|value| value.get("success"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or_else(|| exit_code == Some(0));
    let parsed = parsed_json.unwrap_or_else(|| {
        serde_json::json!({
            "success": exit_code == Some(0),
            "data": { "stdout": stdout.as_str(), "stderr": stderr.as_str() },
        })
    });
    let mut data = parsed
        .get("data")
        .cloned()
        .unwrap_or_else(|| parsed.clone());
    let text = extract_text(&data, &parsed, &stderr);
    let (mut text, char_truncated) = truncate_text(text, max_output);

    let truncated = char_truncated || clipped_at_pipe;
    if clipped_at_pipe {
        if !char_truncated {
            text.push_str(
                "\n\n[output clipped: agent-browser produced more data than the parent read \
                 budget; result is incomplete]",
            );
        }
        annotate_truncation(&mut data, max_output);
    }

    Ok(ParsedCliResponse {
        success,
        data,
        text,
        elapsed_ms: started.elapsed().as_millis(),
        truncated,
        exit_code,
    })
}

fn annotate_truncation(data: &mut serde_json::Value, budget_chars: usize) {
    if let Some(map) = data.as_object_mut() {
        map.entry("truncated")
            .or_insert(serde_json::Value::Bool(true));
        map.insert(
            "truncationReason".to_string(),
            serde_json::Value::String("parent-read-budget".to_string()),
        );
        map.insert(
            "truncationNote".to_string(),
            serde_json::Value::String(format!(
                "agent-browser output exceeded the parent read budget (~{budget_chars} chars) \
                 and was clipped; the result is incomplete."
            )),
        );
    }
}

#[derive(Default)]
struct BoundedRead {
    bytes: Vec<u8>,
    clipped: bool,
}

async fn drain_into<R: AsyncReadExt + Unpin>(
    mut reader: R,
    max_bytes: usize,
    sink: &tokio::sync::Mutex<BoundedRead>,
) {
    let mut tmp = [0u8; 8192];
    loop {
        let n = match reader.read(&mut tmp).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let mut guard = sink.lock().await;
        let remaining = max_bytes.saturating_sub(guard.bytes.len());
        if remaining == 0 {
            guard.clipped = true;
            break;
        }
        let to_take = n.min(remaining);
        guard.bytes.extend_from_slice(&tmp[..to_take]);
        if to_take < n {
            guard.clipped = true;
            break;
        }
    }
}

fn extract_text(data: &serde_json::Value, root: &serde_json::Value, stderr: &str) -> String {
    for key in [
        "snapshot",
        "text",
        "message",
        "note",
        "output",
        "path",
        "file",
        "screenshot",
        "screenshotPath",
    ] {
        if let Some(value) = data.get(key).and_then(serde_json::Value::as_str) {
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }
    if let Some(entries) = data.get("entries").and_then(serde_json::Value::as_array) {
        let lines = entries
            .iter()
            .filter_map(|entry| {
                entry
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .collect::<Vec<_>>();
        if !lines.is_empty() {
            return lines.join("\n");
        }
    }
    if let Some(sessions) = data.get("sessions").and_then(serde_json::Value::as_array) {
        return sessions
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>()
            .join("\n");
    }
    if let Some(value) = root.get("error").and_then(serde_json::Value::as_str) {
        if !value.is_empty() {
            return value.to_string();
        }
    }
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string())
}

fn truncate_text(text: String, max_chars: usize) -> (String, bool) {
    if text.chars().count() <= max_chars {
        return (text, false);
    }
    let truncated: String = text.chars().take(max_chars).collect();
    (
        format!("{truncated}\n\n[truncated to {max_chars} characters]"),
        true,
    )
}
