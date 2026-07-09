use aspect_ai_core::*;

use super::approval::require_tool_approval;

pub async fn execute_ssh_connect(
    app: &tauri::AppHandle,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
    is_automatic: bool,
    state: &tauri::State<'_, crate::SharedState>,
) -> Result<String, String> {
    let args = tc.args.clone();
    let host = json_str(&args, "host");
    let port = json_usize(&args, "port", 22);
    let user = json_str_opt(&args, "username");
    let auth = json_str_opt(&args, "auth");
    let effective = if is_automatic { "full-access" } else { "always" };
    require_tool_approval(
        app, turn_id, tc, effective, interactive,
        "SshConnect", &format!("SSH to {host}:{port}"),
        &format!("ssh {}@{host}", user.as_deref().unwrap_or("user")),
        "execute", &[], &format!("ssh:{host}"), false,
    ).await?;
    let result = crate::network::ssh::ssh_connect(
        state.clone(), host.to_string(), user.clone(), Some(port as u16), auth.clone(), None,
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_ssh_exec(
    app: &tauri::AppHandle,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
    is_automatic: bool,
    state: &tauri::State<'_, crate::SharedState>,
) -> Result<String, String> {
    let args = tc.args.clone();
    let host = json_str(&args, "host");
    let command = json_str(&args, "command");
    let effective = if is_automatic { "full-access" } else { "always" };
    require_tool_approval(
        app, turn_id, tc, effective, interactive,
        "SshExec", &format!("SSH exec on {host}"),
        &command.chars().take(400).collect::<String>(),
        "execute", &[], &format!("ssh:{host}"), false,
    ).await?;
    let result = crate::network::ssh::ssh_exec(state.clone(), host.to_string(), command.to_string(), None, None).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_ssh_transfer(
    app: &tauri::AppHandle,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
    is_automatic: bool,
    state: &tauri::State<'_, crate::SharedState>,
) -> Result<String, String> {
    let args = tc.args.clone();
    let host = json_str(&args, "host");
    let direction = json_str_opt(&args, "direction").unwrap_or("download".to_string());
    let remote = json_str(&args, "remotePath");
    let local = json_str_opt(&args, "localPath").unwrap_or("".to_string());
    let effective = if is_automatic { "full-access" } else { "always" };
    require_tool_approval(
        app, turn_id, tc, effective, interactive,
        "SshTransfer", &format!("SSH {} {remote}", direction),
        &format!("{} {remote} -> {local}", direction),
        "execute", &[], &format!("ssh:{host}"), false,
    ).await?;
    let transfer_dir = aspect_ai_core::json_helpers::parse_transfer_direction(&direction);
    let result = crate::network::ssh::ssh_transfer(
        state.clone(), host.to_string(), transfer_dir, local.to_string(), remote.to_string(), None,
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_ssh_list(
    state: &tauri::State<'_, crate::SharedState>,
) -> Result<String, String> {
    let result = crate::network::ssh::ssh_list(state.clone()).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_ssh_disconnect(
    state: &tauri::State<'_, crate::SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let host = json_str(&args, "host");
    let result: crate::network::ssh::SshDisconnectResult = crate::network::ssh::ssh_disconnect(state.clone(), Some(host.to_string()), None)?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}
