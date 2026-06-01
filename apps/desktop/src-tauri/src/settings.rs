use lux_core::{
    KeybindingProfile, LuxEvent, RecentWorkspace, SettingValue, SettingsScope, WorkspaceInfo,
};
use lux_settings::SettingsStore;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use super::{lock_error, SharedState};

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
    emit_settings_changed(&app, key)?;
    Ok(setting)
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
    emit_settings_changed(&app, "workbench.keybindings")?;
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
    action: impl FnOnce(&mut SettingsStore) -> lux_core::AppResult<T>,
) -> lux_core::AppResult<T> {
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| lux_core::AppError::Service("settings lock poisoned".to_string()))?;
    let settings = settings.as_mut().ok_or_else(|| {
        lux_core::AppError::Service("settings store is not initialized".to_string())
    })?;
    action(settings)
}

fn emit_settings_changed(app: &AppHandle, key: impl Into<String>) -> Result<(), String> {
    app.emit("lux://event", LuxEvent::SettingsChanged { key: key.into() })
        .map_err(|error| error.to_string())
}
