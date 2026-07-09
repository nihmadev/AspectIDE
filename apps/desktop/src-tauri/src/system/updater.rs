//! Automatic update orchestration for AspectIDE.
//!
//! Wraps `tauri-plugin-updater` behind two explicit, frontend-driven commands so
//! the UI fully controls the experience (no surprise restarts):
//!
//! - [`update_check`] queries the configured release endpoints and returns the
//!   available update's metadata (version + release notes) without downloading.
//! - [`update_install`] downloads the verified bundle for the current platform,
//!   applies it, and relaunches the app. Download progress is streamed to the
//!   frontend via `aspect://update` events so the UI can render a progress bar.
//!
//! ## Stability & safety
//!
//! - Signature verification is enforced by the plugin (the public key is baked
//!   into the bundle at release build time); a tampered or unsigned artifact is
//!   rejected before it is ever written to disk.
//! - The updater is a no-op in dev / when no endpoints are configured: `check`
//!   returns "up to date" instead of erroring, so the UI degrades gracefully.
//! - All network/IO runs off the UI thread (the commands are `async`).
//! - Progress events are best-effort; a failed emit never aborts the install.

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// Result of an update check, returned to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    /// Whether a newer version is available at the configured endpoints.
    pub available: bool,
    /// The currently running version.
    pub current_version: String,
    /// The available version, when `available` is true.
    pub version: Option<String>,
    /// Release notes / changelog body for the available version, when provided.
    pub notes: Option<String>,
}

/// Download/apply progress, emitted on `aspect://update`.
///
/// `rename_all` on the enum only camel-cases the variant *names* (the `kind`
/// tag); struct-variant fields need their own `rename_all`, so each carries one
/// to match the frontend's `contentLength` contract.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum UpdateProgress {
    /// Download started; `contentLength` is the total bytes when known.
    #[serde(rename_all = "camelCase")]
    Started { content_length: Option<u64> },
    /// Incremental progress: bytes downloaded so far out of the total (if known).
    #[serde(rename_all = "camelCase")]
    Progress {
        downloaded: u64,
        content_length: Option<u64>,
    },
    /// Download finished; the installer is about to be applied.
    Finished,
}

const UPDATE_EVENT: &str = "aspect://update";

/// Whether the updater plugin is configured (and therefore registered). The
/// plugin is only registered when `plugins.updater` exists in the resolved Tauri
/// config (CI-prepared release builds). Calling `app.updater()` without it would
/// panic on missing managed state, so commands must gate on this first.
fn updater_configured(app: &AppHandle) -> bool {
    app.config().plugins.0.contains_key("updater")
}

/// Checks the configured release endpoints for a newer signed build.
///
/// Returns `available: false` (never an error) when the updater is not
/// configured — e.g. local dev builds without `plugins.updater` — so the UI can
/// treat "no updater" and "up to date" identically.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<UpdateCheckResult, String> {
    let current_version = app.package_info().version.to_string();

    // No updater configured (dev build / missing endpoints): treat as current.
    // Checked via config (not `app.updater()`) because the plugin — and thus its
    // managed state — is absent in dev, and `app.updater()` would panic.
    if !updater_configured(&app) {
        return Ok(UpdateCheckResult {
            available: false,
            current_version,
            version: None,
            notes: None,
        });
    }
    let Ok(updater) = app.updater() else {
        return Ok(UpdateCheckResult {
            available: false,
            current_version,
            version: None,
            notes: None,
        });
    };

    match updater.check().await {
        Ok(Some(update)) => Ok(UpdateCheckResult {
            available: true,
            current_version: update.current_version,
            version: Some(update.version),
            notes: update.body,
        }),
        Ok(None) => Ok(UpdateCheckResult {
            available: false,
            current_version,
            version: None,
            notes: None,
        }),
        Err(error) => Err(format!("Update check failed: {error}")),
    }
}

/// Downloads, verifies, applies the available update, then relaunches the app.
///
/// Streams [`UpdateProgress`] events on `aspect://update`. On success the process is
/// replaced by the new version and this call does not return; on failure it
/// returns a human-readable error and the running app is left untouched.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    if !updater_configured(&app) {
        return Err("Updater is not configured in this build.".to_string());
    }
    let updater = app
        .updater()
        .map_err(|error| format!("Updater is not available: {error}"))?;

    let update = updater
        .check()
        .await
        .map_err(|error| format!("Update check failed: {error}"))?
        .ok_or_else(|| "No update is available to install.".to_string())?;

    let mut downloaded: u64 = 0;
    let emit_app = app.clone();
    let started_app = app.clone();
    let finished_app = app.clone();

    update
        .download_and_install(
            move |chunk_length, content_length| {
                if downloaded == 0 {
                    let _ =
                        started_app.emit(UPDATE_EVENT, UpdateProgress::Started { content_length });
                }
                downloaded = downloaded.saturating_add(chunk_length as u64);
                let _ = emit_app.emit(
                    UPDATE_EVENT,
                    UpdateProgress::Progress {
                        downloaded,
                        content_length,
                    },
                );
            },
            move || {
                let _ = finished_app.emit(UPDATE_EVENT, UpdateProgress::Finished);
            },
        )
        .await
        .map_err(|error| format!("Update installation failed: {error}"))?;

    // On Windows the NSIS installer exits the app itself; on macOS/Linux we
    // relaunch into the freshly applied bundle so the user lands on the new
    // version without a manual restart.
    app.restart();
}

