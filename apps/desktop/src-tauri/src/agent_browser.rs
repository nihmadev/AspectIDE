#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Mutex,
    time::{Duration, Instant},
};

static NPM_LATEST_CACHE: Mutex<Option<(String, Instant)>> = Mutex::new(None);
static LAST_AUTO_UPGRADE: Mutex<Option<Instant>> = Mutex::new(None);
static STATUS_PROBE_IN_FLIGHT: Mutex<bool> = Mutex::new(false);

const NPM_CACHE_TTL: Duration = Duration::from_hours(1);
const AUTO_UPGRADE_COOLDOWN: Duration = Duration::from_mins(30);
const NPM_LATEST_URL: &str = "https://registry.npmjs.org/agent-browser/latest";

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

/// Windows `CREATE_NO_WINDOW` — prevents a console window from flashing when the
/// agent-browser CLI (and its npm/install steps) are spawned from the GUI app.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Spawns the agent-browser CLI without a visible console window on Windows.
/// Centralizes the `creation_flags` call so no spawn site can forget it.
fn agent_browser_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

const DEFAULT_TIMEOUT_SECS: u64 = 90;
const MAX_TIMEOUT_SECS: u64 = 180;
const DEFAULT_MAX_OUTPUT: usize = 50_000;
const MAX_OUTPUT_CAP: usize = 120_000;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const INSTALL_TIMEOUT_SECS: u64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserStatusRequest {
    pub command_path: Option<String>,
    /// When true, skip npm latest check and auto-upgrade (faster status-only probe).
    pub skip_auto_update: Option<bool>,
    /// When true, only resolve CLI + version. Skips doctor and session listing (no Chromium).
    pub lightweight: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserStatusResponse {
    pub available: bool,
    pub command_path: Option<String>,
    pub version: Option<String>,
    pub latest_version: Option<String>,
    pub update_performed: bool,
    pub update_detail: Option<String>,
    pub detail: String,
    pub sessions: Vec<String>,
    pub doctor: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserInvokeRequest {
    pub session: String,
    pub args: Vec<String>,
    pub headed: Option<bool>,
    pub allowed_domains: Option<String>,
    pub max_output: Option<usize>,
    pub timeout_secs: Option<u64>,
    pub command_path: Option<String>,
    pub session_name: Option<String>,
    pub profile: Option<String>,
    pub state_path: Option<String>,
    pub content_boundaries: Option<bool>,
    pub ignore_https_errors: Option<bool>,
    pub allow_file_access: Option<bool>,
    pub provider: Option<String>,
    pub proxy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserInvokeResponse {
    pub session: String,
    pub command: String,
    pub success: bool,
    pub data: serde_json::Value,
    pub text: String,
    pub elapsed_ms: u128,
    pub truncated: bool,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserInstallRequest {
    pub command_path: Option<String>,
    pub with_deps: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserInstallStep {
    pub name: String,
    pub success: bool,
    pub output: String,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserInstallResponse {
    pub success: bool,
    pub command_path: Option<String>,
    pub steps: Vec<AgentBrowserInstallStep>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserReadImageRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserReadImageResponse {
    pub path: String,
    pub data_url: String,
    pub bytes: usize,
    pub mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserStreamStatusRequest {
    pub session: String,
    pub command_path: Option<String>,
    pub enable: Option<bool>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserStreamStatusResponse {
    pub session: String,
    pub enabled: bool,
    pub port: Option<u16>,
    pub websocket_url: Option<String>,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserDashboardRequest {
    pub action: String,
    pub port: Option<u16>,
    pub command_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserDashboardResponse {
    pub action: String,
    pub success: bool,
    pub port: Option<u16>,
    pub url: Option<String>,
    pub detail: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserSkillsRequest {
    pub name: Option<String>,
    pub all: Option<bool>,
    pub command_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserSkillsResponse {
    pub success: bool,
    pub content: String,
    pub data: serde_json::Value,
}

/// RAII guard that clears `STATUS_PROBE_IN_FLIGHT` on drop, so the in-flight flag is
/// released even if `status_inner` panics or its future is cancelled before returning.
/// A manual reset would be skipped on those paths, wedging every future probe.
struct StatusProbeGuard;

impl Drop for StatusProbeGuard {
    fn drop(&mut self) {
        if let Ok(mut in_flight) = STATUS_PROBE_IN_FLIGHT.lock() {
            *in_flight = false;
        }
    }
}

pub async fn status(
    request: AgentBrowserStatusRequest,
) -> Result<AgentBrowserStatusResponse, String> {
    let lightweight = request.lightweight == Some(true);
    let _probe_guard = if lightweight {
        let Ok(mut in_flight) = STATUS_PROBE_IN_FLIGHT.lock() else {
            return Err("agent-browser status probe lock poisoned".to_string());
        };
        if *in_flight {
            return Ok(AgentBrowserStatusResponse {
                available: false,
                command_path: None,
                version: None,
                latest_version: None,
                update_performed: false,
                update_detail: Some("Status probe already running.".to_string()),
                detail: "agent-browser status probe already running".to_string(),
                sessions: Vec::new(),
                doctor: None,
            });
        }
        *in_flight = true;
        drop(in_flight);
        Some(StatusProbeGuard)
    } else {
        None
    };

    status_inner(request, lightweight).await
}

// Linear status-resolution flow (fetch latest, auto-upgrade, doctor, detail string); splitting it
// would scatter shared local state across helpers without reducing real complexity.
#[allow(clippy::too_many_lines)]
async fn status_inner(
    request: AgentBrowserStatusRequest,
    lightweight: bool,
) -> Result<AgentBrowserStatusResponse, String> {
    let use_bundled = request
        .command_path
        .as_ref()
        .is_none_or(|value| value.trim().is_empty());
    let mut update_performed = false;
    let mut update_detail: Option<String> = None;
    let mut latest_version: Option<String> = None;

    if use_bundled && request.skip_auto_update != Some(true) {
        latest_version = fetch_npm_latest_version_cached().await.ok();
        if let Some(latest) = latest_version.as_ref() {
            if let Ok(path) = resolve_binary(None) {
                if let Ok(current_raw) = read_version(&path).await {
                    let current = normalize_agent_browser_version(&current_raw);
                    if version_is_older(&current, latest) && try_begin_auto_upgrade() {
                        match upgrade_bundled_agent_browser().await {
                            Ok(detail) => {
                                update_performed = true;
                                update_detail = Some(detail);
                            }
                            Err(error) => {
                                update_detail = Some(format!("Auto-update failed: {error}"));
                            }
                        }
                    }
                }
            }
        }
    }

    let binary = match resolve_binary(request.command_path.as_deref()) {
        Ok(path) => path,
        Err(error) => {
            return Ok(AgentBrowserStatusResponse {
                available: false,
                command_path: None,
                version: None,
                latest_version,
                update_performed,
                update_detail,
                detail: error,
                sessions: Vec::new(),
                doctor: None,
            });
        }
    };

    let version = read_version(&binary)
        .await
        .ok()
        .map(|text| normalize_agent_browser_version(&text));
    let sessions = if lightweight {
        Vec::new()
    } else {
        list_sessions(&binary).await.unwrap_or_default()
    };
    let doctor = if lightweight {
        None
    } else {
        run_json(
            &binary,
            None,
            &["doctor", "--json", "--offline", "--quick"],
            45,
        )
        .await
        .ok()
        .map(|response| response.data)
    };

    let available = if lightweight {
        version.is_some()
    } else {
        doctor
            .as_ref()
            .and_then(|value| value.get("success"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true)
    };

    let base_detail = if available {
        format!(
            "agent-browser is available ({})",
            version
                .clone()
                .unwrap_or_else(|| "version unknown".to_string())
        )
    } else {
        "agent-browser responded, but doctor reported issues. Run `agent-browser doctor --fix` in a terminal.".to_string()
    };

    let detail = if update_performed {
        format!(
            "agent-browser updated to {} ({})",
            version.clone().unwrap_or_else(|| "unknown".to_string()),
            update_detail.clone().unwrap_or_default()
        )
    } else if let (Some(current), Some(latest)) = (version.as_ref(), latest_version.as_ref()) {
        if version_is_older(current, latest) {
            format!("agent-browser {current} (npm latest {latest}; update pending or failed)")
        } else if available {
            format!("agent-browser is up to date ({current})")
        } else {
            base_detail
        }
    } else {
        base_detail
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
    })
}

pub async fn invoke(
    request: AgentBrowserInvokeRequest,
) -> Result<AgentBrowserInvokeResponse, String> {
    let binary = resolve_binary(request.command_path.as_deref())?;
    let session = sanitize_session(&request.session);
    if request.args.is_empty() {
        return Err("Browser invoke requires at least one command argument.".to_string());
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
            ignore_https_errors: request.ignore_https_errors,
            allow_file_access: request.allow_file_access,
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

#[derive(Clone)]
struct InvokeOptions {
    session: String,
    headed: Option<bool>,
    allowed_domains: Option<String>,
    max_output: usize,
    session_name: Option<String>,
    profile: Option<String>,
    state_path: Option<String>,
    content_boundaries: Option<bool>,
    ignore_https_errors: Option<bool>,
    allow_file_access: Option<bool>,
    provider: Option<String>,
    proxy: Option<String>,
}

const fn session_invoke_options(session: String, max_output: usize) -> InvokeOptions {
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

struct ParsedCliResponse {
    success: bool,
    data: serde_json::Value,
    text: String,
    elapsed_ms: u128,
    truncated: bool,
    exit_code: Option<i32>,
}

#[allow(
    clippy::too_many_lines,
    reason = "cohesive subprocess JSON invocation; splitting would scatter shared state"
)]
async fn run_json(
    binary: &PathBuf,
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

    let child = command
        .spawn()
        .map_err(|error| format!("Failed to start agent-browser: {error}"))?;

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs.saturating_add(5)),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| format!("agent-browser timed out after {timeout_secs}s"))?
    .map_err(|error| format!("agent-browser process failed: {error}"))?;

    let exit_code = output.status.code();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if stdout.is_empty() && !stderr.is_empty() && !output.status.success() {
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
        .unwrap_or_else(|| output.status.success());
    let parsed = parsed_json.unwrap_or_else(|| {
        serde_json::json!({
            "success": output.status.success(),
            "data": { "stdout": stdout.as_str(), "stderr": stderr.as_str() },
        })
    });
    let data = parsed
        .get("data")
        .cloned()
        .unwrap_or_else(|| parsed.clone());
    let text = extract_text(&data, &parsed, &stderr);
    let (text, truncated) = truncate_text(text, max_output);

    Ok(ParsedCliResponse {
        success,
        data,
        text,
        elapsed_ms: started.elapsed().as_millis(),
        truncated,
        exit_code,
    })
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

async fn read_version(binary: &PathBuf) -> Result<String, String> {
    let mut command = agent_browser_command(binary);
    command.arg("--version");
    command.stdin(Stdio::null());
    command.kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(15), command.output())
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

async fn list_sessions(binary: &PathBuf) -> Result<Vec<String>, String> {
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

fn resolve_binary(override_path: Option<&str>) -> Result<PathBuf, String> {
    if let Some(path) = override_path.filter(|value| !value.trim().is_empty()) {
        let candidate = PathBuf::from(path.trim());
        if candidate.exists() {
            return Ok(candidate);
        }
        return Err(format!(
            "agent-browser not found at configured path: {}",
            candidate.display()
        ));
    }

    if let Ok(path) =
        std::env::var("AGENT_BROWSER_PATH").or_else(|_| std::env::var("LUX_AGENT_BROWSER_COMMAND"))
    {
        let candidate = PathBuf::from(path.trim());
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    if let Ok(path) = which::which("agent-browser") {
        return Ok(path);
    }

    if let Some(path) = bundled_binary() {
        return Ok(path);
    }

    Err(
        "agent-browser CLI is not installed. Use Settings → Browser automation → Install now, or run `pnpm add agent-browser` in apps/desktop."
            .to_string(),
    )
}

fn bundled_binary() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let desktop_dir = manifest_dir.parent()?;
    let bin_name = if cfg!(windows) {
        "agent-browser.cmd"
    } else {
        "agent-browser"
    };
    let candidate = desktop_dir.join("node_modules").join(".bin").join(bin_name);
    candidate.exists().then_some(candidate)
}

#[must_use]
pub fn sanitize_session(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "lux-default".to_string()
    } else {
        format!("lux-{}", &trimmed[..trimmed.len().min(48)])
    }
}

#[tauri::command]
pub async fn agent_browser_status(
    request: Option<AgentBrowserStatusRequest>,
) -> Result<AgentBrowserStatusResponse, String> {
    status(request.unwrap_or(AgentBrowserStatusRequest {
        command_path: None,
        skip_auto_update: None,
        lightweight: None,
    }))
    .await
}

#[tauri::command]
pub async fn agent_browser_invoke(
    request: AgentBrowserInvokeRequest,
) -> Result<AgentBrowserInvokeResponse, String> {
    invoke(request).await
}

pub async fn install(
    request: AgentBrowserInstallRequest,
) -> Result<AgentBrowserInstallResponse, String> {
    let mut steps = Vec::new();
    let desktop_dir = desktop_package_dir();
    let (package_manager, install_args) = resolve_package_manager_latest()?;
    let local_install = run_install_step_in_dir(
        "package-install-latest",
        package_manager.clone(),
        install_args,
        desktop_dir.as_deref(),
        INSTALL_TIMEOUT_SECS,
    )
    .await;
    steps.push(local_install.clone());

    if !local_install.success {
        let npm = resolve_npm()?;
        let npm_install = run_install_step(
            "npm-install-global-latest",
            npm,
            vec![
                "install".to_string(),
                "-g".to_string(),
                "agent-browser@latest".to_string(),
            ],
            INSTALL_TIMEOUT_SECS,
        )
        .await;
        steps.push(npm_install);
    }

    if let Ok(binary) = resolve_binary(request.command_path.as_deref()) {
        let upgrade_step = run_install_step(
            "agent-browser-cli-upgrade",
            binary,
            vec!["upgrade".to_string()],
            INSTALL_TIMEOUT_SECS,
        )
        .await;
        steps.push(upgrade_step);
    }

    let binary = resolve_binary(request.command_path.as_deref()).ok();
    let chrome_args = if request.with_deps == Some(true) {
        vec!["install".to_string(), "--with-deps".to_string()]
    } else {
        vec!["install".to_string()]
    };

    let chrome_step = if let Some(binary) = binary {
        run_install_step(
            "agent-browser-install-chrome",
            binary,
            chrome_args.clone(),
            INSTALL_TIMEOUT_SECS,
        )
        .await
    } else {
        let fallback = resolve_binary_after_install()?;
        run_install_step(
            "agent-browser-install-chrome",
            fallback,
            chrome_args,
            INSTALL_TIMEOUT_SECS,
        )
        .await
    };
    steps.push(chrome_step);

    let command_path = resolve_binary(request.command_path.as_deref()).ok();
    let success = steps.iter().all(|step| step.success);
    let detail = if success {
        "agent-browser installed successfully.".to_string()
    } else {
        "agent-browser installation finished with errors. Review step output.".to_string()
    };

    Ok(AgentBrowserInstallResponse {
        success,
        command_path: command_path.map(|path| path.display().to_string()),
        steps,
        detail,
    })
}

pub async fn read_image(
    request: AgentBrowserReadImageRequest,
) -> Result<AgentBrowserReadImageResponse, String> {
    let path = PathBuf::from(request.path.trim());
    if request.path.trim().is_empty() {
        return Err("Image path is required.".to_string());
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

pub async fn stream_status(
    request: AgentBrowserStreamStatusRequest,
) -> Result<AgentBrowserStreamStatusResponse, String> {
    let binary = resolve_binary(request.command_path.as_deref())?;
    let session = sanitize_session(&request.session);
    let enable_stream = request.enable == Some(true);
    // Stream state is session-scoped: BOTH `stream enable` and `stream status` must
    // target the chat's daemon via `--session`. Previously the status-only query passed
    // no session, so it probed the default daemon (a different/absent session) and the
    // live preview never found its WebSocket port — it sat in "waiting"/"disconnected"
    // until a manual refresh (the only path that enabled, and thus scoped, the stream).
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

pub async fn dashboard(
    request: AgentBrowserDashboardRequest,
) -> Result<AgentBrowserDashboardResponse, String> {
    let binary = resolve_binary(request.command_path.as_deref())?;
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

pub async fn skills(
    request: AgentBrowserSkillsRequest,
) -> Result<AgentBrowserSkillsResponse, String> {
    let binary = resolve_binary(request.command_path.as_deref())?;
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

fn desktop_package_dir() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().map(std::path::Path::to_path_buf)
}

fn resolve_package_manager_latest() -> Result<(PathBuf, Vec<String>), String> {
    if let Ok(path) = which::which("pnpm") {
        return Ok((
            path,
            vec!["add".to_string(), "agent-browser@latest".to_string()],
        ));
    }
    let npm = resolve_npm()?;
    Ok((
        npm,
        vec!["install".to_string(), "agent-browser@latest".to_string()],
    ))
}

async fn fetch_npm_latest_version_cached() -> Result<String, String> {
    if let Ok(cache) = NPM_LATEST_CACHE.lock() {
        if let Some((version, fetched_at)) = cache.as_ref() {
            if fetched_at.elapsed() < NPM_CACHE_TTL {
                return Ok(version.clone());
            }
        }
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(NPM_LATEST_URL)
        .send()
        .await
        .map_err(|error| format!("Failed to reach npm registry: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("npm registry returned HTTP {}", response.status()));
    }
    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("Invalid npm registry response: {error}"))?;
    let version = payload
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "npm registry response missing version field".to_string())?
        .to_string();
    if let Ok(mut cache) = NPM_LATEST_CACHE.lock() {
        *cache = Some((version.clone(), Instant::now()));
    }
    Ok(version)
}

fn normalize_agent_browser_version(raw: &str) -> String {
    raw.trim()
        .strip_prefix("agent-browser")
        .unwrap_or(raw)
        .trim()
        .to_string()
}

fn parse_version_parts(version: &str) -> Option<(u32, u32, u32)> {
    let core = version.split('+').next()?.split('-').next()?.trim();
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn version_is_older(current: &str, latest: &str) -> bool {
    match (parse_version_parts(current), parse_version_parts(latest)) {
        (Some(current_parts), Some(latest_parts)) => current_parts < latest_parts,
        _ => current != latest,
    }
}

/// Atomically claims the auto-upgrade cooldown slot: under a single lock, checks
/// whether the cooldown has elapsed and, if so, stamps `Instant::now()` and returns
/// `true`. This makes the check-then-act a single critical section so two concurrent
/// `status()` calls cannot both pass the gate and launch simultaneous installs.
fn try_begin_auto_upgrade() -> bool {
    let Ok(mut guard) = LAST_AUTO_UPGRADE.lock() else {
        return true;
    };
    let allowed = guard
        .as_ref()
        .is_none_or(|instant| instant.elapsed() >= AUTO_UPGRADE_COOLDOWN);
    if allowed {
        *guard = Some(Instant::now());
    }
    allowed
}

async fn upgrade_bundled_agent_browser() -> Result<String, String> {
    let desktop_dir = desktop_package_dir();
    let (package_manager, install_args) = resolve_package_manager_latest()?;
    let package_step = run_install_step_in_dir(
        "auto-update-package",
        package_manager,
        install_args,
        desktop_dir.as_deref(),
        INSTALL_TIMEOUT_SECS,
    )
    .await;
    if !package_step.success {
        return Err(package_step.output);
    }
    let binary = resolve_binary(None)?;
    let cli_upgrade = run_install_step(
        "auto-update-cli",
        binary.clone(),
        vec!["upgrade".to_string()],
        INSTALL_TIMEOUT_SECS,
    )
    .await;
    let version = read_version(&binary).await.unwrap_or_default();
    if cli_upgrade.success {
        Ok(format!(
            "Updated via npm/pnpm and agent-browser upgrade ({version})"
        ))
    } else {
        Ok(format!("Updated package to latest ({version})"))
    }
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
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        command.output(),
    )
    .await;
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

fn resolve_npm() -> Result<PathBuf, String> {
    if cfg!(windows) {
        if let Ok(path) = which::which("npm.cmd") {
            return Ok(path);
        }
    }
    which::which("npm").map_err(|_| {
        "npm was not found on PATH. Install Node.js 24+ before installing agent-browser."
            .to_string()
    })
}

fn resolve_binary_after_install() -> Result<PathBuf, String> {
    resolve_binary(None)
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

fn mime_type_for_path(path: &Path) -> String {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg" | "jpe") => "image/jpeg".to_string(),
        Some("webp") => "image/webp".to_string(),
        Some("gif") => "image/gif".to_string(),
        _ => "image/png".to_string(),
    }
}

#[tauri::command]
pub async fn agent_browser_install(
    request: Option<AgentBrowserInstallRequest>,
) -> Result<AgentBrowserInstallResponse, String> {
    install(request.unwrap_or(AgentBrowserInstallRequest {
        command_path: None,
        with_deps: None,
    }))
    .await
}

#[tauri::command]
pub async fn agent_browser_read_image(
    request: AgentBrowserReadImageRequest,
) -> Result<AgentBrowserReadImageResponse, String> {
    read_image(request).await
}

#[tauri::command]
pub async fn agent_browser_stream_status(
    request: AgentBrowserStreamStatusRequest,
) -> Result<AgentBrowserStreamStatusResponse, String> {
    stream_status(request).await
}

#[tauri::command]
pub async fn agent_browser_dashboard(
    request: AgentBrowserDashboardRequest,
) -> Result<AgentBrowserDashboardResponse, String> {
    dashboard(request).await
}

#[tauri::command]
pub async fn agent_browser_skills(
    request: Option<AgentBrowserSkillsRequest>,
) -> Result<AgentBrowserSkillsResponse, String> {
    skills(request.unwrap_or(AgentBrowserSkillsRequest {
        name: None,
        all: None,
        command_path: None,
    }))
    .await
}

#[cfg(test)]
mod tests {
    use super::sanitize_session;

    #[test]
    fn sanitize_session_prefixes_and_filters() {
        assert_eq!(sanitize_session("chat-123"), "lux-chat-123");
        assert_eq!(sanitize_session(""), "lux-default");
    }
}
