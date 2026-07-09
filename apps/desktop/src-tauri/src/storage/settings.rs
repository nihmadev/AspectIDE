use aspect_core::{
    KeybindingProfile, AspectEvent, RecentWorkspace, SettingValue, SettingsScope, WorkspaceInfo,
};
use aspect_settings::SettingsStore;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::{lock_error, SharedState};

#[tauri::command]
pub fn recent_workspaces(state: State<'_, SharedState>) -> Result<Vec<RecentWorkspace>, String> {
    with_settings(&state, |settings| settings.recent_workspaces()).map_err(String::from)
}

#[tauri::command]
pub fn recent_workspace_forget(
    state: State<'_, SharedState>,
    root: std::path::PathBuf,
) -> Result<Vec<RecentWorkspace>, String> {
    with_settings(&state, |settings| settings.forget_recent_workspace(root)).map_err(String::from)
}

#[tauri::command]
pub fn settings_get(
    state: State<'_, SharedState>,
    scope: SettingsScope,
    key: String,
) -> Result<Option<SettingValue>, String> {
    let settings = state.settings.lock().map_err(lock_error)?;
    Ok(settings.as_ref().and_then(|store| store.get(scope, &key)))
}

#[tauri::command]
pub fn settings_set(
    app: AppHandle,
    state: State<'_, SharedState>,
    scope: SettingsScope,
    key: String,
    value: Value,
) -> Result<SettingValue, String> {
    let mut settings = state.settings.lock().map_err(lock_error)?;
    let store = settings
        .as_mut()
        .ok_or_else(|| "settings store is not initialized".to_string())?;
    let setting = store.set(scope, key.clone(), value).map_err(String::from)?;
    if let Err(error) = emit_settings_changed(&app, key) {
        tracing::warn!(%error, "failed to emit settings-changed");
    }
    Ok(setting)
}

/// Sets the global CPU budget for filesystem scans and content search. Called
/// from the frontend on startup and whenever the user changes the preference.
/// Accepts `"auto"` (reserve a core for the UI), `"all"`, or `"half"`.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri command arguments are deserialized by value"
)]
pub fn set_scan_concurrency(mode: String) {
    aspect_core::set_scan_concurrency(aspect_core::ScanConcurrency::from_preference(&mode));
}

#[tauri::command]
pub fn keybindings_get(state: State<'_, SharedState>) -> Result<KeybindingProfile, String> {
    with_settings(&state, |settings| Ok(settings.keybinding_profile())).map_err(String::from)
}

#[tauri::command]
pub fn keybindings_set(
    app: AppHandle,
    state: State<'_, SharedState>,
    profile: KeybindingProfile,
) -> Result<KeybindingProfile, String> {
    let mut settings = state.settings.lock().map_err(lock_error)?;
    let store = settings
        .as_mut()
        .ok_or_else(|| "settings store is not initialized".to_string())?;
    let profile = store
        .set_keybinding_profile(profile)
        .map_err(String::from)?;
    if let Err(error) = emit_settings_changed(&app, "workbench.keybindings") {
        tracing::warn!(%error, "failed to emit settings-changed");
    }
    Ok(profile)
}

pub fn record_recent_workspace(
    state: &State<'_, SharedState>,
    workspace: &WorkspaceInfo,
) -> Result<(), String> {
    with_settings(state, |settings| {
        settings.record_recent_workspace(workspace)
    })
    .map(|_| ())
    .map_err(String::from)
}

fn with_settings<T>(
    state: &State<'_, SharedState>,
    action: impl FnOnce(&mut SettingsStore) -> aspect_core::AppResult<T>,
) -> aspect_core::AppResult<T> {
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| aspect_core::AppError::Service("settings lock poisoned".to_string()))?;
    let settings = settings.as_mut().ok_or_else(|| {
        aspect_core::AppError::Service("settings store is not initialized".to_string())
    })?;
    action(settings)
}

fn emit_settings_changed(app: &AppHandle, key: impl Into<String>) -> Result<(), String> {
    app.emit("aspect://event", AspectEvent::SettingsChanged { key: key.into() })
        .map_err(|error| error.to_string())
}
