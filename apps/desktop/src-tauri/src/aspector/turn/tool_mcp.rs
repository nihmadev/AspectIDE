use aspect_ai_core::*;

use super::approval::require_tool_approval;

pub async fn execute_mcp_proxy(
    app: &tauri::AppHandle,
    _state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
    is_automatic: bool,
) -> Result<String, String> {
    let args = tc.args.clone();
    let name = &tc.name;
    let rest = &name["mcp__".len()..];
    let (server, tool) = rest
        .split_once("__")
        .ok_or_else(|| format!("malformed MCP tool name: {name}"))?;
    let mcp_target = format!("{server}/{tool}");
    let preview = serde_json::to_string(&args).unwrap_or_default();
    let effective = if is_automatic { "full-access" } else { input.tool_approval_mode.as_str() };
    require_tool_approval(
        app, turn_id, tc, effective, interactive,
        "Mcp", &format!("Call MCP tool {mcp_target}"),
        &preview.chars().take(400).collect::<String>(),
        "execute", &input.tool_permission_rules, &mcp_target, false,
    ).await?;
    crate::network::mcp::call_tool(server, tool, args).await
}

pub async fn execute_mcp_manage(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
    is_automatic: bool,
) -> Result<String, String> {
    let args = tc.args.clone();
    let effective = if is_automatic { "full-access" } else { input.tool_approval_mode.as_str() };
    let action = json_str(&args, "action").to_lowercase();
    let id = json_str(&args, "id");

    if action != "list" {
        let preview = serde_json::to_string(&args).unwrap_or_default();
        require_tool_approval(
            app, turn_id, tc, effective, interactive,
            "McpManage", &format!("MCP {action} {id}"),
            &preview.chars().take(400).collect::<String>(),
            "execute", &input.tool_permission_rules,
            &format!("manage/{action}"), false,
        ).await?;
    }

    match action.as_str() {
        "list" => {
            let configs = crate::network::mcp::read_mcp_config(state);
            let live = crate::network::mcp::all_status().await;
            Ok(serde_json::json!({ "configured": configs, "live": live }).to_string())
        }
        "add" => {
            let id = id.trim();
            if id.is_empty() { return Err("McpManage add requires 'id'.".to_string()); }
            let command = json_str(&args, "command");
            if command.trim().is_empty() { return Err("McpManage add requires 'command'.".to_string()); }
            let server_args = json_str_array(&args, "args", 64);
            let env = args.get("env").and_then(|v| v.as_object())
                .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
                .unwrap_or_default();
            let enabled = args.get("enabled").and_then(serde_json::Value::as_bool).unwrap_or(true);
            let name = json_str_opt(&args, "name").unwrap_or_else(|| id.to_string());
            let config = crate::network::mcp::McpServerConfig {
                id: id.to_string(), name, command, args: server_args, env, enabled,
            };
            let status = crate::network::mcp::mcp_add(state.clone(), config).await?;
            serde_json::to_string(&status).map_err(|e| e.to_string())
        }
        "connect" | "restart" => {
            let configs = crate::network::mcp::read_mcp_config(state);
            let config = configs.into_iter().find(|c| c.id == id)
                .ok_or_else(|| format!("MCP server '{id}' not found"))?;
            let status = crate::network::mcp::connect_server(config).await?;
            serde_json::to_string(&status).map_err(|e| e.to_string())
        }
        "disconnect" => {
            crate::network::mcp::disconnect_server(&id).await;
            Ok(serde_json::json!({ "id": id, "state": "disconnected" }).to_string())
        }
        "enable" | "disable" => {
            let enabled = action == "enable";
            crate::network::mcp::mcp_enable(state.clone(), id.clone(), enabled).await?;
            Ok(serde_json::json!({ "id": id, "enabled": enabled }).to_string())
        }
        "remove" => {
            crate::network::mcp::mcp_remove(state.clone(), id.clone()).await?;
            Ok(serde_json::json!({ "id": id, "removed": true }).to_string())
        }
        other => Err(format!(
            "Unknown McpManage action '{other}'. Use list|add|connect|restart|disconnect|enable|disable|remove."
        )),
    }
}
