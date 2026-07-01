//! High-level agent-browser operations: status, invoke, stream, dashboard,
//! skills, screenshot reads, and install. These compose the resolver, process,
//! validation, and version layers into the behaviours the Tauri commands expose.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

use super::process::{list_sessions, read_version, run_json, session_invoke_options};
use super::resolver::{
    agent_browser_command, binary_source_label, desktop_package_dir, resolve_binary,
    resolve_binary_with_source, resolve_package_manager, DEFAULT_MAX_OUTPUT, DEFAULT_TIMEOUT_SECS,
    INSTALL_TIMEOUT_SECS, MAX_IMAGE_BYTES, MAX_OUTPUT_CAP, MAX_TIMEOUT_SECS,
};
use super::types::{
    AgentBrowserDashboardRequest, AgentBrowserDashboardResponse, AgentBrowserInstallRequest,
    AgentBrowserInstallResponse, AgentBrowserInstallStep, AgentBrowserInvokeRequest,
    AgentBrowserInvokeResponse, AgentBrowserReadImageRequest, AgentBrowserReadImageResponse,
    AgentBrowserSkillsRequest, AgentBrowserSkillsResponse, AgentBrowserStatusRequest,
    AgentBrowserStatusResponse, AgentBrowserStreamStatusRequest, AgentBrowserStreamStatusResponse,
    InvokeOptions,
};
use super::validate::{
    mime_type_for_path, sanitize_session, validate_domain_list, validate_provider,
    validate_proxy_url,
};
use super::version::{normalize_agent_browser_version, version_is_older};

// ── Status (read-only) ──

pub async fn status(
    request: AgentBrowserStatusRequest,
) -> Result<AgentBrowserStatusResponse, String> {
    let lightweight = request.lightweight == Some(true);
    status_inner(lightweight).await
}

#[allow(clippy::too_many_lines)]
async fn status_inner(lightweight: bool) -> Result<AgentBrowserStatusResponse, String> {
    let (binary, source) = resolve_binary_with_source()?;

    let version = read_version(&binary)
        .await
        .ok()
        .map(|text| normalize_agent_browser_version(&text));

    // Non-lightweight status is read-only: no npm latest check, no network I/O.
    // latest_version is always None in read-only mode. A separate background
    // updater can populate it in future work.
    let latest_version: Option<String> = None;

    let sessions = if lightweight {
        Vec::new()
    } else {
        list_sessions(&binary).await.unwrap_or_default()
    };

    // Doctor: in non-lightweight mode, only the doctor data value is carried
    // into the response (available computed from its success field below).
    let doctor: Option<serde_json::Value> = if lightweight {
        None
    } else {
        run_json(
            &binary,
            None,
            &["doctor", "--json", "--offline", "--quick"],
            45,
        )
        .await
        .map(|resp| resp.data)
        .ok()
    };

    // Fail-closed: non-lightweight requires explicit doctor success for available=true.
    let available = if lightweight {
        version.is_some()
    } else {
        doctor
            .as_ref()
            .and_then(|value| value.get("success"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };

    let detail = if available {
        format!(
            "agent-browser is available ({})",
            version
                .clone()
                .unwrap_or_else(|| "version unknown".to_string())
        )
    } else if lightweight {
        if version.is_some() {
            format!(
                "agent-browser {} (lightweight check)",
                version.as_deref().unwrap_or("version unknown")
            )
        } else {
            "agent-browser resolved but version unknown".to_string()
        }
    } else {
        "agent-browser responded, but doctor reported issues. Run `agent-browser doctor --fix` \
         in a terminal."
            .to_string()
    };

    // Determine if an update is available (cached vs installed).
    let (update_performed, update_detail) =
        if let (Some(current), Some(latest)) = (version.as_ref(), latest_version.as_ref()) {
            if version_is_older(current, latest) {
                (
                    false,
                    Some(format!("Update available: {latest} (installed: {current})")),
                )
            } else {
                (false, None)
            }
        } else {
            (false, None)
        };

    Ok(AgentBrowserStatusResponse {
        available,
        command_path: Some(binary.display().to_string()),
        version,
        latest_version,
        update_performed,
        update_detail,
        detail,
        sessions,
        doctor,
        binary_source: Some(binary_source_label(source).to_string()),
    })
}

// ── Invoke (validated) ──

pub async fn invoke(
    request: AgentBrowserInvokeRequest,
) -> Result<AgentBrowserInvokeResponse, String> {
    let binary = resolve_binary()?;
    let session = sanitize_session(&request.session);
    if request.args.is_empty() {
        return Err("Browser invoke requires at least one command argument.".to_string());
    }

    // ── Security validations ──
    // Deny file access and TLS bypass by default.
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

    // Validate allowed_domains if set.
    if let Some(ref domains) = request.allowed_domains {
        validate_domain_list(domains)?;
    }

    // Validate proxy URL if set.
    if let Some(ref proxy) = request.proxy {
        validate_proxy_url(proxy)?;
    }

    // Validate provider if set (allowlist of known safe providers).
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
    let response = run_json(
        &binary,
        Some(InvokeOptions {
            session: session.clone(),
            headed: request.headed,
            allowed_domains: request.allowed_domains.clone(),
            max_output,
            session_name: request.session_name.clone(),
            profile: request.profile.clone(),
            state_path: request.state_path.clone(),
            content_boundaries: request.content_boundaries,
            // Always deny these — the validate check above already rejected
            // the request if they were requested. Set to false/Never.
            ignore_https_errors: None,
            allow_file_access: None,
            provider: request.provider.clone(),
            proxy: request.proxy.clone(),
        }),
        &arg_refs,
        timeout_secs,
    )
    .await?;

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

// ── Read Image (restricted) ──

pub async fn read_image(
    request: AgentBrowserReadImageRequest,
) -> Result<AgentBrowserReadImageResponse, String> {
    let raw_path = request.path.trim();
    if raw_path.is_empty() {
        return Err("Image path is required.".to_string());
    }

    // Canonicalize the requested path to resolve symlinks and normalize.
    let path = tokio::fs::canonicalize(raw_path)
        .await
        .map_err(|error| format!("Invalid image path: {error}"))?;

    // Verify the canonical path is within an approved root directory.
    let approved_roots = {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Ok(cwd) = std::env::current_dir() {
            if let Ok(real) = std::fs::canonicalize(&cwd) {
                roots.push(real);
            } else {
                roots.push(cwd);
            }
        }
        roots.push(std::env::temp_dir());
        roots
    };

    let in_allowed_root = approved_roots.iter().any(|root| path.starts_with(root));

    if !in_allowed_root {
        return Err(format!(
            "Access denied: path '{}' is outside approved directories.",
            path.display()
        ));
    }

    if !path.exists() {
        return Err(format!("Screenshot file not found: {}", path.display()));
    }
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|error| format!("Failed to read screenshot metadata: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("Screenshot path is not a file: {}", path.display()));
    }
    if metadata.len() > MAX_IMAGE_BYTES as u64 {
        return Err(format!(
            "Screenshot exceeds maximum size ({} bytes > {} bytes)",
            metadata.len(),
            MAX_IMAGE_BYTES
        ));
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|error| format!("Failed to read screenshot: {error}"))?;
    let looks_like_image = bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || (bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP");
    if !looks_like_image {
        return Err("File is not a recognized image format.".to_string());
    }
    let byte_count = bytes.len();
    let mime_type = mime_type_for_path(&path);
    let encoded = BASE64_STANDARD.encode(bytes);
    Ok(AgentBrowserReadImageResponse {
        path: path.display().to_string(),
        data_url: format!("data:{mime_type};base64,{encoded}"),
        bytes: byte_count,
        mime_type,
    })
}

// ── Stream Status ──

pub async fn stream_status(
    request: AgentBrowserStreamStatusRequest,
) -> Result<AgentBrowserStreamStatusResponse, String> {
    let binary = resolve_binary()?;
    let session = sanitize_session(&request.session);
    let enable_stream = request.enable == Some(true);
    let invoke_options = session_invoke_options(session.clone(), DEFAULT_MAX_OUTPUT);
    if enable_stream {
        let mut enable_args = vec!["stream".to_string(), "enable".to_string()];
        if let Some(port) = request.port {
            enable_args.push("--port".to_string());
            enable_args.push(port.to_string());
        }
        let enable_refs: Vec<&str> = enable_args.iter().map(String::as_str).collect();
        let _ = run_json(&binary, Some(invoke_options.clone()), &enable_refs, 45).await;
    }
    let status = run_json(&binary, Some(invoke_options), &["stream", "status"], 30).await?;
    let port = stream_port_from_data(&status.data);
    let enabled = status.success && port.is_some();
    let websocket_url = port.map(|value| format!("ws://127.0.0.1:{value}"));
    Ok(AgentBrowserStreamStatusResponse {
        session,
        enabled,
        port,
        websocket_url,
        data: status.data,
    })
}

fn stream_port_from_data(data: &serde_json::Value) -> Option<u16> {
    for key in ["port", "streamPort", "stream_port"] {
        if let Some(value) = data.get(key).and_then(parse_port_value) {
            return Some(value);
        }
    }
    if let Some(nested) = data.get("stream").and_then(serde_json::Value::as_object) {
        for key in ["port", "streamPort"] {
            if let Some(value) = nested.get(key).and_then(parse_port_value) {
                return Some(value);
            }
        }
    }
    None
}

fn parse_port_value(value: &serde_json::Value) -> Option<u16> {
    match value {
        serde_json::Value::Number(number) => {
            number.as_u64().and_then(|port| u16::try_from(port).ok())
        }
        serde_json::Value::String(text) => text.trim().parse::<u16>().ok(),
        _ => None,
    }
}

// ── Dashboard ──

pub async fn dashboard(
    request: AgentBrowserDashboardRequest,
) -> Result<AgentBrowserDashboardResponse, String> {
    let binary = resolve_binary()?;
    let action = request.action.trim().to_ascii_lowercase();
    let port = request.port.unwrap_or(4848);
    let (args, url): (Vec<String>, Option<String>) = match action.as_str() {
        "start" => (
            vec![
                "dashboard".to_string(),
                "start".to_string(),
                "--port".to_string(),
                port.to_string(),
            ],
            Some(format!("http://127.0.0.1:{port}")),
        ),
        "stop" => (vec!["dashboard".to_string(), "stop".to_string()], None),
        "status" => (
            vec!["dashboard".to_string(), "status".to_string()],
            Some(format!("http://127.0.0.1:{port}")),
        ),
        other => return Err(format!("Unsupported dashboard action: {other}")),
    };
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let response = run_json(&binary, None, &arg_refs, 60).await?;
    let success = response.success;
    let detail = if success {
        format!("agent-browser dashboard {action} succeeded")
    } else {
        response.text.clone()
    };
    Ok(AgentBrowserDashboardResponse {
        action,
        success,
        port: Some(port),
        url,
        detail,
        data: response.data,
    })
}

// ── Skills ──

pub async fn skills(
    request: AgentBrowserSkillsRequest,
) -> Result<AgentBrowserSkillsResponse, String> {
    let binary = resolve_binary()?;
    let args: Vec<String> = if request.all == Some(true) {
        vec!["skills".to_string(), "get".to_string(), "--all".to_string()]
    } else if let Some(name) = request
        .name
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        vec![
            "skills".to_string(),
            "get".to_string(),
            name.trim().to_string(),
            "--full".to_string(),
        ]
    } else {
        vec!["skills".to_string(), "list".to_string()]
    };
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let response = run_json(&binary, None, &arg_refs, 90).await?;
    Ok(AgentBrowserSkillsResponse {
        success: response.success,
        content: response.text.clone(),
        data: response.data,
    })
}

// ── Install (local-only) ──

pub async fn install(
    request: AgentBrowserInstallRequest,
) -> Result<AgentBrowserInstallResponse, String> {
    let mut steps = Vec::new();
    let desktop_dir = desktop_package_dir();
    let (package_manager, install_args) = resolve_package_manager()?;
    let local_install = run_install_step_in_dir(
        "package-install-latest",
        package_manager,
        install_args,
        desktop_dir.as_deref(),
        INSTALL_TIMEOUT_SECS,
    )
    .await;
    steps.push(local_install);

    // NOTE: Global npm install removed. All installs go through the local
    // package manager (pnpm/npm in apps/desktop). If the local install fails,
    // surface the error so the user can fix the project's dependency state.

    // After package install, try to find the CLI and run upgrade/Chrome install.
    let binary = resolve_binary().ok();

    let chrome_args = if request.with_deps == Some(true) {
        vec!["install".to_string(), "--with-deps".to_string()]
    } else {
        vec!["install".to_string()]
    };

    if let Some(ref binary) = binary {
        let chrome_step = run_install_step(
            "agent-browser-install-chrome",
            binary.clone(),
            chrome_args,
            INSTALL_TIMEOUT_SECS,
        )
        .await;
        steps.push(chrome_step);
    }

    let command_path = resolve_binary().ok();
    let success = steps.iter().all(|step| step.success);
    let detail = if success {
        "agent-browser installed successfully.".to_string()
    } else {
        install_failure_detail(&steps)
    };

    Ok(AgentBrowserInstallResponse {
        success,
        command_path: command_path.map(|path| path.display().to_string()),
        steps,
        detail,
    })
}

/// Maximum number of characters of a failing step's captured output to embed in
/// the top-level `detail`. Full output is still available on the step itself;
/// this keeps the summary actionable without dumping an unbounded log.
const FAILURE_DETAIL_OUTPUT_CAP: usize = 800;

/// Build an actionable failure summary: name the first failing step and quote a
/// bounded slice of its captured output so the model/user gets a concrete next
/// step instead of a generic "review step output". Falls back to a step-count
/// summary when no output was captured.
fn install_failure_detail(steps: &[AgentBrowserInstallStep]) -> String {
    let Some(failed) = steps.iter().find(|step| !step.success) else {
        // Unreachable in practice (only called when a step failed), but keep a
        // sensible message rather than panicking.
        return "agent-browser installation finished with errors. Review step output.".to_string();
    };

    let captured = failed.output.trim();
    let output_hint = if captured.is_empty() {
        // No captured output at all — tell the user how to get more, since the
        // empty string is otherwise a dead end.
        " No output was captured for this step; re-run the install from a terminal \
         to see the underlying error."
            .to_string()
    } else {
        let bounded = bounded_tail(captured, FAILURE_DETAIL_OUTPUT_CAP);
        format!(" Output from '{}':\n{bounded}", failed.name)
    };

    format!(
        "agent-browser installation failed at step '{}'.{output_hint}",
        failed.name
    )
}

/// Return the last `max_chars` characters of `text`, prefixed with a marker when
/// the head was dropped. Tail (not head) because build/install errors — the
/// actionable part — are almost always at the end of the log.
fn bounded_tail(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let tail: String = text.chars().skip(count - max_chars).collect();
    format!(
        "[...{} earlier characters omitted...]\n{tail}",
        count - max_chars
    )
}

async fn run_install_step_in_dir(
    name: &str,
    program: PathBuf,
    args: Vec<String>,
    working_dir: Option<&Path>,
    timeout_secs: u64,
) -> AgentBrowserInstallStep {
    let started = Instant::now();
    let mut command = agent_browser_command(&program);
    command.args(&args);
    if let Some(dir) = working_dir {
        command.current_dir(dir);
    }
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(timeout_secs), command.output()).await;
    match output {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{stdout}{stderr}").trim().to_string();
            AgentBrowserInstallStep {
                name: name.to_string(),
                success: output.status.success(),
                output: combined,
                elapsed_ms: started.elapsed().as_millis(),
            }
        }
        Ok(Err(error)) => AgentBrowserInstallStep {
            name: name.to_string(),
            success: false,
            output: format!("Failed to start install step: {error}"),
            elapsed_ms: started.elapsed().as_millis(),
        },
        Err(_) => AgentBrowserInstallStep {
            name: name.to_string(),
            success: false,
            output: format!("Install step timed out after {timeout_secs}s"),
            elapsed_ms: started.elapsed().as_millis(),
        },
    }
}

async fn run_install_step(
    name: &str,
    program: PathBuf,
    args: Vec<String>,
    timeout_secs: u64,
) -> AgentBrowserInstallStep {
    run_install_step_in_dir(name, program, args, None, timeout_secs).await
}

#[cfg(test)]
mod tests {
    use super::{bounded_tail, install_failure_detail, FAILURE_DETAIL_OUTPUT_CAP};
    use crate::agent_browser::types::AgentBrowserInstallStep;

    fn step(name: &str, success: bool, output: &str) -> AgentBrowserInstallStep {
        AgentBrowserInstallStep {
            name: name.to_string(),
            success,
            output: output.to_string(),
            elapsed_ms: 0,
        }
    }

    #[test]
    fn failure_detail_names_first_failing_step_and_quotes_output() {
        let steps = vec![
            step("package-install-latest", true, "ok"),
            step(
                "agent-browser-install-chrome",
                false,
                "Error: failed to download Chromium (network unreachable)",
            ),
        ];
        let detail = install_failure_detail(&steps);
        assert!(
            detail.contains("agent-browser-install-chrome"),
            "detail must name the failing step: {detail}"
        );
        assert!(
            detail.contains("network unreachable"),
            "detail must include the captured step output: {detail}"
        );
    }

    #[test]
    fn failure_detail_handles_empty_output_with_next_step() {
        let steps = vec![step("package-install-latest", false, "   ")];
        let detail = install_failure_detail(&steps);
        assert!(detail.contains("package-install-latest"));
        assert!(
            detail.contains("No output was captured"),
            "empty output should yield an actionable hint: {detail}"
        );
        assert!(detail.contains("terminal"));
    }

    #[test]
    fn failure_detail_bounds_large_output() {
        let big = "x".repeat(FAILURE_DETAIL_OUTPUT_CAP * 3);
        let steps = vec![step("package-install-latest", false, &big)];
        let detail = install_failure_detail(&steps);
        // The whole 3x payload must not be embedded verbatim.
        assert!(detail.len() < big.len());
        assert!(detail.contains("earlier characters omitted"));
    }

    #[test]
    fn bounded_tail_keeps_actionable_end() {
        let text = "HEAD-noise\nMIDDLE\nERROR: the real problem is here";
        // Cap is smaller than the input but large enough to keep the actionable tail.
        let bounded = bounded_tail(text, 24);
        assert!(bounded.ends_with("the real problem is here"));
        assert!(bounded.contains("earlier characters omitted"));
    }

    #[test]
    fn bounded_tail_passthrough_when_short() {
        let text = "short output";
        assert_eq!(bounded_tail(text, 800), "short output");
    }
}
