//! AspectIDE managed-gateway device linking.
//!
//! The open-source client ships no API key. To use the bundled AspectIDE provider a
//! user links their Telegram account: the IDE asks the gateway for a link code +
//! deep link, the user opens the bot and solves a captcha, and the IDE polls until
//! the gateway hands back a per-device token (1 Telegram = 1 user = 1 set of limits).
//! A cross-origin browser fetch from the webview can't reach the gateway (no `CORS`),
//! so these hops run here in Rust via `reqwest`.

use serde::{Deserialize, Serialize};
use tauri_plugin_opener::OpenerExt;

/// Open an http(s) URL in the user's default handler (browser / Telegram app). The
/// webview's own `<a target="_blank">` does NOT reach the OS, so the link modal calls
/// this to open the t.me deep link.
#[tauri::command]
pub fn aspect_open_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let u = url.trim();
    if !u.starts_with("https://") && !u.starts_with("http://") {
        return Err("invalid url".to_string());
    }
    app.opener()
        .open_url(u.to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// Gateway origin (drops the trailing `/v1`) for the link endpoints, validated to be
/// an http(s) URL.
fn gateway_origin(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let origin = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
    if !origin.starts_with("https://") && !origin.starts_with("http://") {
        return Err("invalid gateway base url".to_string());
    }
    Ok(origin.to_string())
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

/// A started device link: a code + a t.me deep link the user opens to bind.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LinkStart {
    pub code: String,
    #[serde(default)]
    pub deep_link: String,
}

/// A link poll result. `status` is "pending" until the bot binds it, then "ready"
/// (exactly once) with the token.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LinkStatus {
    pub status: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub device_id: String,
}

/// Ask the gateway to start a device link. Returns a code + deep link to the bot.
#[tauri::command]
pub async fn aspect_link_start(base_url: String) -> Result<LinkStart, String> {
    let origin = gateway_origin(&base_url)?;
    let client = http_client()?;
    let start: LinkStart = client
        .post(format!("{origin}/link/start"))
        .send()
        .await
        .map_err(|e| format!("link start failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("link start rejected: {e}"))?
        .json()
        .await
        .map_err(|e| format!("link start decode failed: {e}"))?;
    if start.code.is_empty() {
        return Err("gateway returned an empty link code".to_string());
    }
    Ok(start)
}

/// Poll a link code. Returns status "pending" until the user finishes in the bot,
/// then "ready" with the token (handed out exactly once).
#[tauri::command]
pub async fn aspect_link_poll(base_url: String, code: String) -> Result<LinkStatus, String> {
    let origin = gateway_origin(&base_url)?;
    if code.trim().is_empty() {
        return Err("missing link code".to_string());
    }
    let client = http_client()?;
    let resp = client
        .get(format!("{origin}/link/status"))
        .query(&[("code", code.trim())])
        .send()
        .await
        .map_err(|e| format!("link poll failed: {e}"))?;
    // 404/410 (unknown/expired) surface as an error so the client restarts the flow.
    if !resp.status().is_success() {
        return Err(format!("link poll rejected: {}", resp.status()));
    }
    resp.json()
        .await
        .map_err(|e| format!("link poll decode failed: {e}"))
}

/// One rolling window's usage against its per-user cap (`cap == 0` means uncapped).
/// `used` is THIS user's spend; `cap` is the per-user allowance.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AspectWindowUsage {
    pub window: String,
    #[serde(default)]
    pub used: i64,
    #[serde(default)]
    pub cap: i64,
}

/// Per-model usage for the live composer indicator: all-time total plus each
/// rolling window's used/cap, scoped to the calling user.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AspectModelUsage {
    pub id: String,
    #[serde(default)]
    pub total: i64,
    #[serde(default)]
    pub windows: Vec<AspectWindowUsage>,
}

/// Fetch this user's per-model usage/cap from the gateway (`GET {base_url}/usage`).
/// Returns an empty list when no token is available yet (not linked) so callers can
/// render nothing without treating it as an error.
#[tauri::command]
pub async fn aspect_usage(
    base_url: String,
    token: String,
) -> Result<Vec<AspectModelUsage>, String> {
    if token.trim().is_empty() {
        return Ok(Vec::new());
    }
    let trimmed = base_url.trim().trim_end_matches('/');
    if !trimmed.starts_with("https://") && !trimmed.starts_with("http://") {
        return Err("invalid gateway base url".to_string());
    }
    let client = http_client()?;
    let response = client
        .get(format!("{trimmed}/usage"))
        .bearer_auth(token.trim())
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("usage request failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("usage failed: {}", response.status()));
    }
    #[derive(Deserialize)]
    struct UsageResponse {
        #[serde(default)]
        models: Vec<AspectModelUsage>,
    }
    let parsed: UsageResponse = response
        .json()
        .await
        .map_err(|e| format!("usage decode failed: {e}"))?;
    Ok(parsed.models)
}
