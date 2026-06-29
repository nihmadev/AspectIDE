#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{
    net::IpAddr,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Windows `CREATE_NO_WINDOW` — prevents a console window from flashing when the
/// agent-browser CLI is spawned from the GUI app.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Check if IP is loopback, private, or unspecified (stable equivalent of
/// the nightly-only `IpAddr::is_private()` / `is_unspecified()`).
fn is_private_or_loopback_ip(ip: &IpAddr) -> bool {
    if ip.is_loopback() {
        return true;
    }
    if ip.is_unspecified() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            // RFC 1918: 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
            // Carrier-grade NAT: 100.64.0.0/10
            // Link-local: 169.254.0.0/16
            let octets = v4.octets();
            octets[0] == 10
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                || (octets[0] == 169 && octets[1] == 254)
        }
        IpAddr::V6(v6) => {
            // Unique local: fc00::/7
            v6.octets()[0] & 0xfe == 0xfc
        }
    }
}

/// Spawns the agent-browser CLI without a visible console window on Windows.
/// Centralizes the `creation_flags` call so no spawn site can forget it.
fn agent_browser_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    // `mut` is only exercised by the Windows `creation_flags` call below.
    #[cfg_attr(not(windows), allow(unused_mut))]
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
const READ_VERSION_TIMEOUT_SECS: u64 = 15;

// ── Binary source provenance ──
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinarySource {
    Bundled,
    EnvVar,
    Path,
}

// ── Request/Response DTOs ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserStatusRequest {
    /// Per-request command_path override. IGNORED by the Rust side -- binary
    /// resolution uses trusted sources only (bundled > env var > PATH).
    /// Kept for backward compat; frontend should remove from status requests.
    pub command_path: Option<String>,
    /// Deprecated -- no longer used. Status is always read-only.
    pub skip_auto_update: Option<bool>,
    /// When true, only resolve CLI + version. Skips doctor, session listing,
    /// and npm-latest check (no network, no Chromium).
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
    pub binary_source: Option<String>,
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
    /// Per-request command_path override. IGNORED by the Rust side -- binary
    /// resolution uses trusted sources only (bundled > env var > PATH).
    /// Kept for backward compat; frontend should remove from invoke requests.
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
    /// Per-request command_path override. IGNORED by the Rust side.
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
    /// Per-request command_path override. IGNORED by the Rust side.
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
    /// Per-request command_path override. IGNORED by the Rust side.
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
    /// Per-request command_path override. IGNORED by the Rust side.
    pub command_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrowserSkillsResponse {
    pub success: bool,
    pub content: String,
    pub data: serde_json::Value,
}

// ── Default implementations for request types ──

impl Default for AgentBrowserStatusRequest {
    fn default() -> Self {
        Self { command_path: None, skip_auto_update: None, lightweight: None }
    }
}

impl Default for AgentBrowserInstallRequest {
    fn default() -> Self {
        Self { command_path: None, with_deps: None }
    }
}

impl Default for AgentBrowserSkillsRequest {
    fn default() -> Self {
        Self { name: None, all: None, command_path: None }
    }
}

// ── Binary Resolution ──

/// Resolve the agent-browser binary path from trusted sources only.
/// Order: bundled > env var > PATH. Rejects per-request overrides.
fn resolve_binary() -> Result<PathBuf, String> {
    resolve_binary_with_source().map(|(path, _)| path)
}

fn resolve_binary_with_source() -> Result<(PathBuf, BinarySource), String> {
    // 1. Bundled binary (node_modules/.bin/agent-browser) — highest precedence.
    if let Some(path) = bundled_binary() {
        return Ok((path, BinarySource::Bundled));
    }

    // 2. Environment variables — user-configured at the OS/process level.
    if let Ok(path) =
        std::env::var("AGENT_BROWSER_PATH").or_else(|_| std::env::var("LUX_AGENT_BROWSER_COMMAND"))
    {
        let candidate = PathBuf::from(path.trim());
        if candidate.exists() {
            return Ok((candidate, BinarySource::EnvVar));
        }
    }

    // 3. PATH resolution.
    if let Ok(path) = which::which("agent-browser") {
        return Ok((path, BinarySource::Path));
    }

    Err(
        "agent-browser CLI is not installed. Use Settings -> Browser automation -> Install now, \
         or run `pnpm add agent-browser` in apps/desktop."
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

fn binary_source_label(source: BinarySource) -> &'static str {
    match source {
        BinarySource::Bundled => "bundled",
        BinarySource::EnvVar => "env",
        BinarySource::Path => "path",
    }
}

// ── Status (read-only) ──

pub async fn status(
    request: AgentBrowserStatusRequest,
) -> Result<AgentBrowserStatusResponse, String> {
    let lightweight = request.lightweight == Some(true);
    status_inner(lightweight).await
}

#[allow(clippy::too_many_lines)]
async fn status_inner(
    lightweight: bool,
) -> Result<AgentBrowserStatusResponse, String> {
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
        run_json(&binary, None, &["doctor", "--json", "--offline", "--quick"], 45)
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
    let (update_performed, update_detail) = if let (Some(current), Some(latest)) =
        (version.as_ref(), latest_version.as_ref())
    {
        if version_is_older(current, latest) {
            (false, Some(format!("Update available: {latest} (installed: {current})")))
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

// ── Domain/Proxy/Provider Validation ──

/// Validate a comma-separated list of domains. Rejects private/loopback IPs
/// and internal-only hostnames.
fn validate_domain_list(domains: &str) -> Result<(), String> {
    for domain in domains.split(',') {
        let domain = domain.trim();
        if domain.is_empty() {
            continue;
        }
        // Reject raw IPs in private ranges.
        if let Ok(ip) = domain.parse::<IpAddr>() {
            if is_private_or_loopback_ip(&ip) {
                return Err(format!(
                    "Domain '{}' is a private/local IP address and is not allowed. \
                     Use a public domain or approve it in AI preferences.",
                    domain
                ));
            }
            continue;
        }
        // Reject internal-only TLDs.
        let lower = domain.to_ascii_lowercase();
        for bad_tld in &[
            ".local",
            ".internal",
            ".localhost",
            ".localdomain",
            ".lan",
            ".intranet",
        ] {
            if lower == *bad_tld || lower.ends_with(bad_tld) {
                return Err(format!(
                    "Domain '{}' uses an internal-only TLD '{}' and is not allowed. \
                     Use a public domain.",
                    domain, bad_tld
                ));
            }
        }
        // Reject hostnames that are just "localhost" or "local".
        if lower == "localhost" || lower == "local" || lower.starts_with("localhost.") {
            return Err(format!(
                "Domain '{}' resolves to localhost and is not allowed. Use a public domain.",
                domain
            ));
        }
    }
    Ok(())
}

/// Validate a proxy URL (http/https scheme, public hosts only, no private IPs).
/// Uses basic string parsing to avoid pulling in the `url` crate.
fn validate_proxy_url(proxy: &str) -> Result<(), String> {
    let trimmed = proxy.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    // Require http:// or https:// prefix.
    let lower = trimmed.to_ascii_lowercase();
    let rest = if let Some(rest) = lower.strip_prefix("https://") {
        rest
    } else if let Some(rest) = lower.strip_prefix("http://") {
        rest
    } else {
        return Err(format!(
            "Invalid proxy URL '{}': scheme must be http or https.",
            trimmed
        ));
    };
    // Split host:port from the rest.
    let host_part = rest.split('/').next().unwrap_or(rest);
    let host = if let Some(idx) = host_part.rfind(':') {
        // Could be port, or IPv6 address with brackets.
        let candidate = &host_part[..idx];
        if candidate.is_empty() {
            // IPv6 like [::1]:8080 — take the bracketed part.
            let ipv6_end = host_part.rfind(']');
            ipv6_end.map(|end| &host_part[1..end]).unwrap_or(host_part)
        } else {
            candidate
        }
    } else {
        host_part
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return Err("Invalid proxy URL: empty host.".to_string());
    }
    // Reject private IP hosts.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_or_loopback_ip(&ip) {
            return Err(format!(
                "Proxy URL host '{host}' is a private/local address and is not allowed."
            ));
        }
    } else {
        let host_lower = host.to_ascii_lowercase();
        if host_lower == "localhost" || host_lower.ends_with(".local") || host_lower.ends_with(".internal") {
            return Err(format!(
                "Proxy URL host '{host}' is an internal hostname and is not allowed."
            ));
        }
    }
    Ok(())
}

/// Validate the provider identifier against an allowlist of known safe providers.
fn validate_provider(provider: &str) -> Result<(), String> {
    let trimmed = provider.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Ok(());
    }
    // Allowlisted known-safe providers. Unknown providers are rejected.
    let allowed = [
        "chrome", "browserless", "browserbase", "kernel", "agentcore", "ios",
    ];
    if allowed.contains(&trimmed.as_str()) {
        return Ok(());
    }
    Err(format!(
        "Provider '{}' is not in the allowlist of known providers. \
         Allowed: {}",
        provider,
        allowed.join(", ")
    ))
}

// ── Invoke Options ──

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

// ── CLI Response Parsing ──

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
    let stdout_pipe = child.stdout.take().ok_or_else(|| "no stdout pipe".to_string())?;
    let stderr_pipe = child.stderr.take().ok_or_else(|| "no stderr pipe".to_string())?;

    // Allocate a generous but bounded buffer for stdout (max_output + JSON margin).
    let stdout_budget = max_output.saturating_add(4096);
    // Stderr is error info — 32 KiB is generous.
    let stderr_budget: usize = 32_768;

    let result = tokio::time::timeout(
        Duration::from_secs(timeout_secs.saturating_add(5)),
        async {
            let stdout_buf = read_pipe_bounded(stdout_pipe, stdout_budget).await;
            let stderr_buf = read_pipe_bounded(stderr_pipe, stderr_budget).await;
            let status = child.wait().await;
            (stdout_buf, stderr_buf, status)
        },
    )
    .await
    .map_err(|_| format!("agent-browser timed out after {timeout_secs}s"))?;

    let (stdout_bytes, stderr_bytes, status_result) = result;
    let exit_code = status_result
        .map(|status| status.code())
        .unwrap_or(None);

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

/// Read a pipe into a Vec<u8>, stopping after `max_bytes` have been collected.
async fn read_pipe_bounded<R: AsyncReadExt + Unpin>(
    mut reader: R,
    max_bytes: usize,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(max_bytes.min(4096));
    let mut tmp = [0u8; 8192];
    loop {
        let n = match reader.read(&mut tmp).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        let remaining = max_bytes.saturating_sub(buf.len());
        if remaining == 0 {
            break;
        }
        let to_take = n.min(remaining);
        buf.extend_from_slice(&tmp[..to_take]);
        if to_take < n {
            break; // truncated at cap
        }
    }
    buf
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

// ── Version Helpers ──

async fn read_version(binary: &Path) -> Result<String, String> {
    let mut command = agent_browser_command(binary);
    command.arg("--version");
    command.stdin(Stdio::null());
    command.kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(READ_VERSION_TIMEOUT_SECS), command.output())
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

async fn list_sessions(binary: &Path) -> Result<Vec<String>, String> {
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

// ── Session Sanitization ──

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

    let in_allowed_root = approved_roots
        .iter()
        .any(|root| path.starts_with(root));

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

fn desktop_package_dir() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().map(std::path::Path::to_path_buf)
}

fn resolve_package_manager() -> Result<(PathBuf, Vec<String>), String> {
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
        "agent-browser installation finished with errors. Review step output.".to_string()
    };

    Ok(AgentBrowserInstallResponse {
        success,
        command_path: command_path.map(|path| path.display().to_string()),
        steps,
        detail,
    })
}

// ── Install Step Runner ──

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

// ── Tauri Commands ──

#[tauri::command]
pub async fn agent_browser_status(
    request: Option<AgentBrowserStatusRequest>,
) -> Result<AgentBrowserStatusResponse, String> {
    status(request.unwrap_or_default()).await
}

#[tauri::command]
pub async fn agent_browser_invoke(
    request: AgentBrowserInvokeRequest,
) -> Result<AgentBrowserInvokeResponse, String> {
    invoke(request).await
}

#[tauri::command]
pub async fn agent_browser_install(
    request: Option<AgentBrowserInstallRequest>,
) -> Result<AgentBrowserInstallResponse, String> {
    install(request.unwrap_or_default()).await
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
    skills(request.unwrap_or_default()).await
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::sanitize_session;

    #[test]
    fn sanitize_session_prefixes_and_filters() {
        assert_eq!(sanitize_session("chat-123"), "lux-chat-123");
        assert_eq!(sanitize_session(""), "lux-default");
    }

    #[test]
    fn version_parsing_orders_correctly() {
        use super::{parse_version_parts, version_is_older};
        assert_eq!(parse_version_parts("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version_parts("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version_parts("1"), Some((1, 0, 0)));
        assert!(parse_version_parts("").is_none());
        assert!(version_is_older("1.0.0", "1.0.1"));
        assert!(version_is_older("1.0.0", "2.0.0"));
        assert!(!version_is_older("1.0.1", "1.0.0"));
        assert!(!version_is_older("1.0.0", "1.0.0"));
    }

    #[test]
    fn domain_validation_rejects_private_ip() {
        use super::validate_domain_list;
        assert!(validate_domain_list("example.com").is_ok());
        assert!(validate_domain_list("example.com,test.org").is_ok());
        assert!(validate_domain_list("127.0.0.1").is_err());
        assert!(validate_domain_list("192.168.1.1").is_err());
        assert!(validate_domain_list("10.0.0.5").is_err());
        assert!(validate_domain_list("172.16.0.1").is_err());
        assert!(validate_domain_list("localhost").is_err());
        assert!(validate_domain_list("internal.local").is_err());
    }

    #[test]
    fn proxy_validation_rejects_internal() {
        use super::validate_proxy_url;
        assert!(validate_proxy_url("http://proxy.example.com:8080").is_ok());
        assert!(validate_proxy_url("https://proxy.example.com").is_ok());
        assert!(validate_proxy_url("http://127.0.0.1:8080").is_err());
        assert!(validate_proxy_url("file:///tmp/proxy").is_err());
        assert!(validate_proxy_url("").is_ok());
    }

    #[test]
    fn provider_validation_allowlists_known() {
        use super::validate_provider;
        assert!(validate_provider("chrome").is_ok());
        assert!(validate_provider("browserless").is_ok());
        assert!(validate_provider("browserbase").is_ok());
        assert!(validate_provider("").is_ok());
        assert!(validate_provider("evil-provider").is_err());
        assert!(validate_provider("CHROME").is_ok());
    }
}
