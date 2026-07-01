//! Subprocess execution: spawns the agent-browser CLI, reads stdout/stderr with
//! parent-side byte caps, parses the JSON envelope, and exposes the lightweight
//! version/session probes built on top of it.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;

use super::resolver::{agent_browser_command, DEFAULT_MAX_OUTPUT, READ_VERSION_TIMEOUT_SECS};
use super::types::{InvokeOptions, ParsedCliResponse};

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
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "cohesive subprocess JSON invocation; splitting would scatter shared state"
)]
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

    // ── Incremental stdout/stderr reading with parent-side byte cap ──
    // Read incrementally instead of wait_with_output to bound memory use
    // from noisy actions before the child finishes.
    let stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| "no stdout pipe".to_string())?;
    let stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| "no stderr pipe".to_string())?;

    // Allocate a generous but bounded buffer for stdout (max_output + JSON margin).
    let stdout_budget = max_output.saturating_add(4096);
    // Stderr is error info — 32 KiB is generous.
    let stderr_budget: usize = 32_768;

    let result = tokio::time::timeout(Duration::from_secs(timeout_secs.saturating_add(5)), async {
        let stdout_buf = read_pipe_bounded(stdout_pipe, stdout_budget).await;
        let stderr_buf = read_pipe_bounded(stderr_pipe, stderr_budget).await;
        let status = child.wait().await;
        (stdout_buf, stderr_buf, status)
    })
    .await
    .map_err(|_| format!("agent-browser timed out after {timeout_secs}s"))?;

    let (stdout_read, stderr_read, status_result) = result;
    // `clipped_at_pipe` means the child produced more bytes than the parent-side
    // budget, so raw stdout was cut before parsing/formatting. This is distinct
    // from the character-level `truncate_text` cap below: a pipe clip can leave
    // the visible text *shorter* than `max_output`, which would otherwise be
    // indistinguishable from a genuinely short answer. Surface it explicitly.
    let stdout_bytes = stdout_read.bytes;
    let stderr_bytes = stderr_read.bytes;
    let clipped_at_pipe = stdout_read.clipped;
    let exit_code = status_result.map_or(None, |status| status.code());

    let stdout = String::from_utf8_lossy(&stdout_bytes).trim().to_string();
    let stderr = String::from_utf8_lossy(&stderr_bytes).trim().to_string();

    if stdout.is_empty() && !stderr.is_empty() && exit_code != Some(0) {
        return Err(stderr);
    }

    // Parse stdout once. Only trust an explicit JSON `success` field when the JSON
    // actually parsed; otherwise (empty / non-JSON stdout) fall back to the process
    // exit code rather than a synthesized hardcoded `false`, so a genuinely successful
    // exit-0 run with non-JSON output is not mis-reported as failed.
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

    // Output is truncated if either the char-level formatter clipped it or the
    // parent-side pipe budget was hit while draining the child. Callers/model
    // read `truncated`; carry a structured, machine-readable note so the reason
    // (pipe clip vs. char cap) is not lost when only the flag is inspected.
    let truncated = char_truncated || clipped_at_pipe;
    if clipped_at_pipe {
        // Only annotate when the pipe clip is the *sole* signal — if the char
        // cap already appended its own `[truncated ...]` marker, don't stack a
        // second, potentially contradictory note onto the visible text.
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

/// Attach a machine-readable truncation note to the response `data` object so
/// callers that inspect structured fields (not just the flag/text) can detect
/// that the raw CLI output was clipped at the parent-side read budget. No-op if
/// `data` is not a JSON object.
fn annotate_truncation(data: &mut serde_json::Value, budget_chars: usize) {
    if let Some(map) = data.as_object_mut() {
        // Don't clobber a `truncated` field the CLI may itself have emitted.
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

/// Result of a bounded pipe read: the collected bytes plus whether the reader
/// stopped because the byte cap was hit (rather than reaching EOF). `clipped`
/// lets the caller distinguish silently-clipped output from a genuinely short
/// answer.
struct BoundedRead {
    bytes: Vec<u8>,
    clipped: bool,
}

/// Read a pipe into a `Vec<u8>`, stopping after `max_bytes` have been collected.
/// Reports whether the cap was reached before EOF via [`BoundedRead::clipped`].
async fn read_pipe_bounded<R: AsyncReadExt + Unpin>(
    mut reader: R,
    max_bytes: usize,
) -> BoundedRead {
    let mut buf = Vec::with_capacity(max_bytes.min(4096));
    let mut tmp = [0u8; 8192];
    let mut clipped = false;
    loop {
        let n = match reader.read(&mut tmp).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let remaining = max_bytes.saturating_sub(buf.len());
        if remaining == 0 {
            // Cap already full but the child still had bytes to give: clipped.
            clipped = true;
            break;
        }
        let to_take = n.min(remaining);
        buf.extend_from_slice(&tmp[..to_take]);
        if to_take < n {
            clipped = true;
            break; // truncated at cap
        }
    }
    BoundedRead {
        bytes: buf,
        clipped,
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

#[cfg(test)]
mod tests {
    use super::{annotate_truncation, read_pipe_bounded};

    // `&[u8]` implements tokio's `AsyncRead`, so it stands in for a child pipe.
    #[tokio::test]
    async fn read_pipe_bounded_flags_clip_when_over_budget() {
        // Reader yields more bytes than the budget: `clipped` must be true and
        // the collected bytes are capped at the budget.
        let data = vec![b'a'; 100];
        let result = read_pipe_bounded(data.as_slice(), 10).await;
        assert!(result.clipped, "over-budget read should be flagged clipped");
        assert_eq!(result.bytes.len(), 10);
    }

    #[tokio::test]
    async fn read_pipe_bounded_not_clipped_when_within_budget() {
        // A genuinely short answer (fits under the budget) is NOT flagged, so it
        // stays distinguishable from a clipped one.
        let data = vec![b'a'; 8];
        let result = read_pipe_bounded(data.as_slice(), 10).await;
        assert!(
            !result.clipped,
            "within-budget read should not be flagged clipped"
        );
        assert_eq!(result.bytes.len(), 8);
    }

    #[tokio::test]
    async fn read_pipe_bounded_not_clipped_at_exact_budget() {
        // Exactly at the budget with EOF right after is not a clip.
        let data = vec![b'a'; 10];
        let result = read_pipe_bounded(data.as_slice(), 10).await;
        assert!(!result.clipped);
        assert_eq!(result.bytes.len(), 10);
    }

    #[test]
    fn annotate_truncation_adds_machine_readable_note() {
        let mut data = serde_json::json!({ "stdout": "partial" });
        annotate_truncation(&mut data, 2000);
        assert_eq!(
            data.get("truncated").and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            data.get("truncationReason")
                .and_then(serde_json::Value::as_str),
            Some("parent-read-budget")
        );
        assert!(data
            .get("truncationNote")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|note| note.contains("clipped")));
    }

    #[test]
    fn annotate_truncation_preserves_existing_truncated_flag() {
        // If the CLI already reported truncated=false for its own reasons we do
        // not silently flip it; the parent signal rides on the reason/note.
        let mut data = serde_json::json!({ "truncated": false });
        annotate_truncation(&mut data, 2000);
        assert_eq!(
            data.get("truncated").and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            data.get("truncationReason")
                .and_then(serde_json::Value::as_str),
            Some("parent-read-budget")
        );
    }

    #[test]
    fn annotate_truncation_noop_on_non_object() {
        let mut data = serde_json::json!("just a string");
        annotate_truncation(&mut data, 2000);
        assert_eq!(data, serde_json::json!("just a string"));
    }
}
