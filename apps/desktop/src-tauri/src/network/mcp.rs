//! Thin Tauri command surface over the `aspect-mcp` crate.
//!
//! Re-exports everything from `aspect-mcp` so existing `crate::network::mcp::*` references
//! continue to work. The settings-aware `read_mcp_config`/`save_mcp_config` and all
//! `#[tauri::command]` functions live here because they depend on
//! `crate::SharedState`.

pub use aspect_mcp::*;

use serde_json::{json, Value};

use crate::SharedState;

pub fn read_mcp_config(state: &tauri::State<'_, SharedState>) -> Vec<McpServerConfig> {
    let Ok(guard) = state.settings.lock() else {
        return Vec::new();
    };
    let Some(store) = guard.as_ref() else {
        return Vec::new();
    };
    let Some(setting) = store.get(aspect_core::SettingsScope::User, MCP_SERVERS_KEY) else {
        return Vec::new();
    };
    serde_json::from_value(setting.value).unwrap_or_default()
}

fn save_mcp_config(
    state: &tauri::State<'_, SharedState>,
    configs: &[McpServerConfig],
) -> Result<(), String> {
    let mut guard = state
        .settings
        .lock()
        .map_err(|_| "settings lock poisoned".to_string())?;
    let store = guard
        .as_mut()
        .ok_or_else(|| "settings store unavailable".to_string())?;
    let value = serde_json::to_value(configs).map_err(|e| e.to_string())?;
    store
        .set(
            aspect_core::SettingsScope::User,
            MCP_SERVERS_KEY.to_string(),
            value,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn mcp_connect_all(
    state: tauri::State<'_, SharedState>,
) -> Result<Vec<McpServerStatus>, String> {
    let configs = read_mcp_config(&state);
    let mut statuses = Vec::new();
    for config in configs.into_iter().filter(|c| c.enabled) {
        match connect_server(config.clone()).await {
            Ok(status) => statuses.push(status),
            Err(error) => statuses.push(McpServerStatus {
                id: config.id,
                name: config.name,
                state: "error".to_string(),
                error: Some(error),
                tools: Vec::new(),
            }),
        }
    }
    Ok(statuses)
}

#[tauri::command]
pub async fn mcp_connect(config: McpServerConfig) -> Result<McpServerStatus, String> {
    connect_server(config).await
}

#[tauri::command]
pub async fn mcp_disconnect(id: String) -> Result<(), String> {
    disconnect_server(&id).await;
    Ok(())
}

#[tauri::command]
pub async fn mcp_status() -> Result<Vec<McpServerStatus>, String> {
    Ok(all_status().await)
}

#[tauri::command]
pub async fn mcp_call(
    server_id: String,
    tool: String,
    arguments: Option<Value>,
) -> Result<String, String> {
    call_tool(&server_id, &tool, arguments.unwrap_or_else(|| json!({}))).await
}

#[tauri::command]
pub async fn mcp_add(
    state: tauri::State<'_, SharedState>,
    config: McpServerConfig,
) -> Result<McpServerStatus, String> {
    if !is_valid_id(&config.id) {
        return Err("invalid MCP server id (use letters, digits, - or _)".to_string());
    }
    let mut configs = read_mcp_config(&state);
    configs.retain(|c| c.id != config.id);
    configs.push(config.clone());
    save_mcp_config(&state, &configs)?;
    if config.enabled {
        connect_server(config).await
    } else {
        Ok(McpServerStatus {
            id: config.id,
            name: config.name,
            state: "disabled".to_string(),
            error: None,
            tools: Vec::new(),
        })
    }
}

#[tauri::command]
pub async fn mcp_remove(
    state: tauri::State<'_, SharedState>,
    id: String,
) -> Result<(), String> {
    disconnect_server(&id).await;
    let mut configs = read_mcp_config(&state);
    configs.retain(|c| c.id != id);
    save_mcp_config(&state, &configs)
}

#[tauri::command]
pub async fn mcp_enable(
    state: tauri::State<'_, SharedState>,
    id: String,
    enabled: bool,
) -> Result<McpServerStatus, String> {
    let mut configs = read_mcp_config(&state);
    let config = configs
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("MCP server '{id}' not found"))?;
    config.enabled = enabled;
    let config = config.clone();
    save_mcp_config(&state, &configs)?;
    if enabled {
        connect_server(config).await
    } else {
        disconnect_server(&id).await;
        Ok(McpServerStatus {
            id: config.id,
            name: config.name,
            state: "disabled".to_string(),
            error: None,
            tools: Vec::new(),
        })
    }
}
