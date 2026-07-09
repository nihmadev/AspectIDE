use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::BufReader;
use tokio::process::{Child, ChildStdin};
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use tokio::time::timeout;

use crate::naming::resolve_tool_name;
use crate::protocol::{read_capped_line, send_notification, send_request, send_request_with_id};
use crate::tool_result::{flatten_tool_result, parse_tools};
use crate::types::{
    McpServerConfig, McpServerStatus, McpToolInfo, CONNECT_TIMEOUT_SECS, PROTOCOL_VERSION,
    REQUEST_TIMEOUT_SECS,
};
use crate::registry;

pub struct Connection {
    config: McpServerConfig,
    state: String,
    error: Option<String>,
    tools: Vec<McpToolInfo>,
    stdin: Option<Arc<AsyncMutex<ChildStdin>>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    next_id: u64,
    child: Option<Child>,
    alive: Arc<AtomicBool>,
}

impl Connection {
    pub fn config(&self) -> &McpServerConfig {
        &self.config
    }

    pub fn state(&self) -> &str {
        &self.state
    }

    pub fn tools(&self) -> &[McpToolInfo] {
        &self.tools
    }

    pub fn status(&self) -> McpServerStatus {
        McpServerStatus {
            id: self.config.id.clone(),
            name: self.config.name.clone(),
            state: self.state.clone(),
            error: self.error.clone(),
            tools: self.tools.clone(),
        }
    }
}

pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.contains("__")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub async fn connect_server(config: McpServerConfig) -> Result<McpServerStatus, String> {
    if !is_valid_id(&config.id) {
        return Err("invalid MCP server id (use letters, digits, - or _)".to_string());
    }
    if config.command.trim().is_empty() {
        return Err("MCP server command is required".to_string());
    }

    disconnect_server(&config.id).await;

    let mut command = tokio::process::Command::new(&config.command);
    command
        .args(&config.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    for (key, value) in &config.env {
        command.env(key, value);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn MCP server '{}': {error}", config.name))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "MCP server stdin unavailable".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "MCP server stdout unavailable".to_string())?;

    let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let stdin = Arc::new(AsyncMutex::new(stdin));

    let alive = Arc::new(AtomicBool::new(true));

    let reader_pending = pending.clone();
    let reader_alive = alive.clone();
    let reader_id = config.id.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut exit_error = "MCP server process exited".to_string();
        loop {
            match read_capped_line(&mut reader).await {
                Ok(Some(line)) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let Ok(message) = serde_json::from_str::<Value>(trimmed) else {
                        continue;
                    };
                    let Some(id) = crate::protocol::normalize_response_id(message.get("id")) else {
                        continue;
                    };
                    if let Some(sender) = reader_pending
                        .lock()
                        .ok()
                        .and_then(|mut map| map.remove(&id))
                    {
                        let _ = sender.send(message);
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    exit_error = format!("MCP server stream error: {error}");
                    break;
                }
            }
        }
        if let Ok(mut map) = reader_pending.lock() {
            map.clear();
        }
        if reader_alive.load(Ordering::SeqCst) {
            if let Some(connection) = registry().lock().await.get_mut(&reader_id) {
                if connection.alive.load(Ordering::SeqCst) {
                    connection.state = "error".to_string();
                    connection.error = Some(exit_error);
                }
            }
        }
    });

    let mut connection = Connection {
        config: config.clone(),
        state: "connecting".to_string(),
        error: None,
        tools: Vec::new(),
        stdin: Some(stdin.clone()),
        pending: pending.clone(),
        next_id: 1,
        child: Some(child),
        alive,
    };

    let init_params = json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": { "name": "AspectIDE", "version": env!("CARGO_PKG_VERSION") },
    });
    let handshake = async {
        send_request(
            &stdin,
            &pending,
            &mut connection.next_id,
            "initialize",
            init_params,
        )
        .await?;
        send_notification(&stdin, "notifications/initialized").await?;
        let tools_result = send_request(
            &stdin,
            &pending,
            &mut connection.next_id,
            "tools/list",
            json!({}),
        )
        .await?;
        Ok::<Vec<McpToolInfo>, String>(parse_tools(&tools_result))
    };

    let handshake_error = match timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS), handshake).await
    {
        Ok(Ok(tools)) => {
            connection.tools = tools;
            connection.state = "connected".to_string();
            None
        }
        Ok(Err(error)) => Some(error),
        Err(_) => Some("MCP handshake timed out".to_string()),
    };

    if let Some(error) = handshake_error {
        connection.alive.store(false, Ordering::SeqCst);
        if let Some(mut child) = connection.child.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        connection.stdin = None;
        if let Ok(mut map) = connection.pending.lock() {
            map.clear();
        }
        connection.tools.clear();
        connection.state = "error".to_string();
        connection.error = Some(error);
    }

    let status = connection.status();
    registry()
        .lock()
        .await
        .insert(config.id.clone(), connection);
    Ok(status)
}

pub async fn disconnect_server(id: &str) {
    let connection = registry().lock().await.remove(id);
    if let Some(mut connection) = connection {
        connection.alive.store(false, Ordering::SeqCst);
        if let Some(mut child) = connection.child.take() {
            let _ = child.start_kill();
        }
    }
}

pub async fn all_status() -> Vec<McpServerStatus> {
    registry()
        .lock()
        .await
        .values()
        .map(Connection::status)
        .collect()
}

pub async fn call_tool(
    server_id: &str,
    tool: &str,
    arguments: Value,
) -> Result<String, String> {
    let (stdin, pending, next_id, real_tool) = {
        let mut registry_guard = registry().lock().await;
        let connection = registry_guard
            .get_mut(server_id)
            .ok_or_else(|| format!("MCP server '{server_id}' is not connected"))?;
        if connection.state != "connected" {
            return Err(match connection.state.as_str() {
                "error" => {
                    let detail = connection
                        .error
                        .as_deref()
                        .unwrap_or("process exited unexpectedly");
                    format!(
                        "MCP server '{server_id}' has exited/crashed ({detail}); reconnect it \
                         (McpManage restart) before calling its tools again."
                    )
                }
                other => format!(
                    "MCP server '{server_id}' is still starting (state: {other}); wait for it to \
                     reach 'connected' before calling its tools."
                ),
            });
        }
        let stdin = connection
            .stdin
            .clone()
            .ok_or_else(|| "MCP server stdin closed".to_string())?;
        let real_tool = resolve_tool_name(server_id, &connection.tools, tool);
        let id = connection.next_id;
        connection.next_id += 2;
        (stdin, connection.pending.clone(), id, real_tool)
    };

    let params = json!({ "name": real_tool, "arguments": arguments });
    let result = timeout(
        Duration::from_secs(REQUEST_TIMEOUT_SECS),
        send_request_with_id(&stdin, &pending, next_id, "tools/call", params),
    )
    .await
    .map_err(|_| format!("MCP tool '{tool}' timed out"))??;
    let flattened = flatten_tool_result(&result);
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(format!("MCP tool '{tool}' reported an error: {flattened}"));
    }
    Ok(flattened)
}
