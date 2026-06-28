//! Real-time Model Context Protocol (MCP) client.
//!
//! Connects to user-configured MCP servers over the stdio transport (newline-
//! delimited JSON-RPC 2.0), performs the `initialize` handshake, lists their tools,
//! and proxies `tools/call`. Connected servers' tools are surfaced to the agent
//! (namespaced `mcp__<server>__<tool>`) and callable live during a turn.
//!
//! Design: one spawned child per server. A background task reads stdout line by
//! line and routes each JSON-RPC response to the matching request via a oneshot in
//! a shared pending-map; notifications are ignored. The connection registry lives
//! on a process-global so the agent turn loop (which has no `AppState` handle deep
//! in the tool dispatch) can reach it without threading state everywhere.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use tokio::time::timeout;

/// Settings key (user scope) holding the MCP server configuration array.
pub const MCP_SERVERS_KEY: &str = "ai.mcp.servers";

const PROTOCOL_VERSION: &str = "2024-11-05";
const REQUEST_TIMEOUT_SECS: u64 = 30;
const CONNECT_TIMEOUT_SECS: u64 = 20;
/// Hard cap on a tool result body so a misbehaving server can't flood the turn.
const MAX_RESULT_CHARS: usize = 60_000;

/// One configured MCP server (persisted in settings).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

const fn default_true() -> bool {
    true
}

/// A tool exposed by a connected server.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's arguments (passed straight to the model).
    pub input_schema: Value,
}

/// Live status of one server, returned to the UI + used to build agent tools.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStatus {
    pub id: String,
    pub name: String,
    /// `connected` | `connecting` | `error` | `disconnected`.
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub tools: Vec<McpToolInfo>,
}

struct Connection {
    config: McpServerConfig,
    state: String,
    error: Option<String>,
    tools: Vec<McpToolInfo>,
    stdin: Option<Arc<AsyncMutex<ChildStdin>>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    next_id: u64,
    child: Option<Child>,
    /// Cleared when this connection is torn down/replaced, so its (still-draining)
    /// reader task can't mark a freshly-reconnected connection as errored.
    alive: Arc<AtomicBool>,
}

impl Connection {
    fn status(&self) -> McpServerStatus {
        McpServerStatus {
            id: self.config.id.clone(),
            name: self.config.name.clone(),
            state: self.state.clone(),
            error: self.error.clone(),
            tools: self.tools.clone(),
        }
    }
}

fn registry() -> &'static AsyncMutex<HashMap<String, Connection>> {
    static REGISTRY: OnceLock<AsyncMutex<HashMap<String, Connection>>> = OnceLock::new();
    REGISTRY.get_or_init(|| AsyncMutex::new(HashMap::new()))
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        // No "__": the agent tool name is mcp__<id>__<tool>, split on the first "__"
        // after the id, so an id containing "__" would mis-split the routing.
        && !id.contains("__")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Connect (or reconnect) a single server: spawn, handshake, list tools.
pub async fn connect_server(config: McpServerConfig) -> Result<McpServerStatus, String> {
    if !is_valid_id(&config.id) {
        return Err("invalid MCP server id (use letters, digits, - or _)".to_string());
    }
    if config.command.trim().is_empty() {
        return Err("MCP server command is required".to_string());
    }

    // Tear down any existing connection for this id first.
    disconnect_server(&config.id).await;

    let mut command = tokio::process::Command::new(&config.command);
    command
        .args(&config.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // Windows: launch the stdio MCP server with no console window. Without this an
    // MCP server whose command is a console app (node, python, a .cmd shim) flashes
    // a cmd window every time it starts — every other child-spawn site in Lux already
    // sets this flag; MCP was the one that missed it.
    #[cfg(windows)]
    command.creation_flags(crate::ai_tools::CREATE_NO_WINDOW);
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

    // Background reader: route each JSON-RPC response to its waiter by id. When the
    // stream ends (server process exited) and this connection is still the live one,
    // flip its status to "error" so the UI/agent see a dead server instead of a
    // stale "connected".
    let reader_pending = pending.clone();
    let reader_alive = alive.clone();
    let reader_id = config.id.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(message) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            let Some(id) = message.get("id").and_then(Value::as_u64) else {
                continue; // notification or malformed — ignore.
            };
            if let Some(sender) = reader_pending
                .lock()
                .ok()
                .and_then(|mut map| map.remove(&id))
            {
                let _ = sender.send(message);
            }
        }
        // Stream ended: the server will never answer any in-flight request. Drop
        // every pending sender so awaiting callers fail fast ("server closed")
        // instead of blocking until their per-request timeout, and so the map
        // can't strand senders for the rest of the connection's lifetime.
        if let Ok(mut map) = reader_pending.lock() {
            map.clear();
        }
        if reader_alive.load(Ordering::SeqCst) {
            if let Some(connection) = registry().lock().await.get_mut(&reader_id) {
                if connection.alive.load(Ordering::SeqCst) {
                    connection.state = "error".to_string();
                    connection.error = Some("MCP server process exited".to_string());
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

    // Handshake: initialize → initialized → tools/list.
    let init_params = json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": { "name": "Lux IDE", "version": env!("CARGO_PKG_VERSION") },
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

    match timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS), handshake).await {
        Ok(Ok(tools)) => {
            connection.tools = tools;
            connection.state = "connected".to_string();
        }
        Ok(Err(error)) => {
            connection.state = "error".to_string();
            connection.error = Some(error);
        }
        Err(_) => {
            connection.state = "error".to_string();
            connection.error = Some("MCP handshake timed out".to_string());
        }
    }

    let status = connection.status();
    registry()
        .lock()
        .await
        .insert(config.id.clone(), connection);
    Ok(status)
}

/// Disconnect + kill a server if connected. Idempotent.
pub async fn disconnect_server(id: &str) {
    // Take the connection out under the lock, then release the guard before
    // touching the child so the registry mutex isn't held across the kill.
    let connection = registry().lock().await.remove(id);
    if let Some(mut connection) = connection {
        // Mark dead first so the reader task's EOF handler can't resurrect/clobber a
        // connection that is being torn down or replaced.
        connection.alive.store(false, Ordering::SeqCst);
        if let Some(mut child) = connection.child.take() {
            let _ = child.start_kill();
        }
    }
}

/// Snapshot of every known server's live status.
pub async fn all_status() -> Vec<McpServerStatus> {
    registry()
        .lock()
        .await
        .values()
        .map(Connection::status)
        .collect()
}

/// Call a tool on a connected server. Returns the flattened text result.
pub async fn call_tool(server_id: &str, tool: &str, arguments: Value) -> Result<String, String> {
    // Clone the handles out under the lock, then release it before the (slow) call
    // so concurrent tool calls to different servers don't serialize on the registry.
    let (stdin, pending, next_id) = {
        let mut registry = registry().lock().await;
        let connection = registry
            .get_mut(server_id)
            .ok_or_else(|| format!("MCP server '{server_id}' is not connected"))?;
        if connection.state != "connected" {
            return Err(format!("MCP server '{server_id}' is not ready"));
        }
        let stdin = connection
            .stdin
            .clone()
            .ok_or_else(|| "MCP server stdin closed".to_string())?;
        let id = connection.next_id;
        connection.next_id += 2;
        (stdin, connection.pending.clone(), id)
    };

    let params = json!({ "name": tool, "arguments": arguments });
    let result = timeout(
        Duration::from_secs(REQUEST_TIMEOUT_SECS),
        send_request_with_id(&stdin, &pending, next_id, "tools/call", params),
    )
    .await
    .map_err(|_| format!("MCP tool '{tool}' timed out"))??;
    Ok(flatten_tool_result(&result))
}

async fn send_request(
    stdin: &Arc<AsyncMutex<ChildStdin>>,
    pending: &Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    next_id: &mut u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let id = *next_id;
    *next_id += 1;
    send_request_with_id(stdin, pending, id, method, params).await
}

async fn send_request_with_id(
    stdin: &Arc<AsyncMutex<ChildStdin>>,
    pending: &Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    request_id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let (tx, rx) = oneshot::channel();
    pending
        .lock()
        .map_err(|_| "MCP pending map poisoned".to_string())?
        .insert(request_id, tx);

    let payload = json!({ "jsonrpc": "2.0", "id": request_id, "method": method, "params": params });
    if let Err(error) = write_line(stdin, &payload).await {
        pending
            .lock()
            .ok()
            .and_then(|mut map| map.remove(&request_id));
        return Err(error);
    }

    let message = rx
        .await
        .map_err(|_| format!("MCP server closed before answering '{method}'"))?;
    if let Some(error) = message.get("error") {
        let msg = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown MCP error");
        return Err(format!("MCP '{method}' error: {msg}"));
    }
    Ok(message.get("result").cloned().unwrap_or(Value::Null))
}

async fn send_notification(
    stdin: &Arc<AsyncMutex<ChildStdin>>,
    method: &str,
) -> Result<(), String> {
    let payload = json!({ "jsonrpc": "2.0", "method": method });
    write_line(stdin, &payload).await
}

async fn write_line(stdin: &Arc<AsyncMutex<ChildStdin>>, payload: &Value) -> Result<(), String> {
    let mut line = serde_json::to_string(payload).map_err(|error| error.to_string())?;
    line.push('\n');
    let mut guard = stdin.lock().await;
    guard
        .write_all(line.as_bytes())
        .await
        .map_err(|error| format!("MCP write failed: {error}"))?;
    guard.flush().await.map_err(|error| error.to_string())
}

fn parse_tools(result: &Value) -> Vec<McpToolInfo> {
    result
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| {
                    let name = tool.get("name").and_then(Value::as_str)?.to_string();
                    Some(McpToolInfo {
                        name,
                        description: tool
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        input_schema: tool
                            .get("inputSchema")
                            .cloned()
                            .unwrap_or_else(|| json!({ "type": "object" })),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Flatten an MCP `tools/call` result (`content: [{type:text,text}]`) to a string.
fn flatten_tool_result(result: &Value) -> String {
    let mut out = String::new();
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for part in content {
            if part.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(text);
                }
            } else {
                // Non-text part (image/resource): include a compact JSON marker.
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&part.to_string());
            }
        }
    }
    if out.is_empty() {
        out = result.to_string();
    }
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        out = format!("[tool error] {out}");
    }
    if out.chars().count() > MAX_RESULT_CHARS {
        let truncated: String = out.chars().take(MAX_RESULT_CHARS).collect();
        out = format!("{truncated}\n…[truncated]");
    }
    out
}

/// Connected servers' tools as `OpenAI` function-tool definitions, namespaced
/// `mcp__<serverId>__<tool>` so the turn-loop dispatch can route a call back to the
/// owning server. Live: reflects whatever is connected at turn-build time.
pub async fn agent_tool_definitions() -> Vec<Value> {
    let registry = registry().lock().await;
    let mut defs = Vec::new();
    for connection in registry.values() {
        if connection.state != "connected" {
            continue;
        }
        for tool in &connection.tools {
            let description = if tool.description.is_empty() {
                format!(
                    "MCP tool '{}' from server '{}'.",
                    tool.name, connection.config.name
                )
            } else {
                format!("[{}] {}", connection.config.name, tool.description)
            };
            defs.push(json!({
                "type": "function",
                "function": {
                    "name": format!("mcp__{}__{}", connection.config.id, tool.name),
                    "description": description,
                    "parameters": tool.input_schema,
                },
            }));
        }
    }
    defs
}

// ── Tauri command surface ──

/// Read the persisted MCP server configs from user settings.
pub fn read_mcp_config(state: &tauri::State<'_, crate::SharedState>) -> Vec<McpServerConfig> {
    let Ok(guard) = state.settings.lock() else {
        return Vec::new();
    };
    let Some(store) = guard.as_ref() else {
        return Vec::new();
    };
    let Some(setting) = store.get(lux_core::SettingsScope::User, MCP_SERVERS_KEY) else {
        return Vec::new();
    };
    serde_json::from_value(setting.value).unwrap_or_default()
}

/// Connect every enabled configured server; returns the live status of all of them.
#[tauri::command]
pub async fn mcp_connect_all(
    state: tauri::State<'_, crate::SharedState>,
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

/// Connect (or reconnect) a single server passed straight from the UI.
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

/// Add or replace a server config, persist it, then connect if enabled.
#[tauri::command]
pub async fn mcp_add(
    state: tauri::State<'_, crate::SharedState>,
    config: McpServerConfig,
) -> Result<McpServerStatus, String> {
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

/// Delete a server config by id and disconnect it. Idempotent.
#[tauri::command]
pub async fn mcp_remove(
    state: tauri::State<'_, crate::SharedState>,
    id: String,
) -> Result<(), String> {
    disconnect_server(&id).await;
    let mut configs = read_mcp_config(&state);
    configs.retain(|c| c.id != id);
    save_mcp_config(&state, &configs)
}

/// Enable or disable a server. Disabling also disconnects the live session.
#[tauri::command]
pub async fn mcp_enable(
    state: tauri::State<'_, crate::SharedState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut configs = read_mcp_config(&state);
    let config = configs
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("MCP server '{id}' not found"))?;
    config.enabled = enabled;
    save_mcp_config(&state, &configs)?;
    if !enabled {
        disconnect_server(&id).await;
    }
    Ok(())
}

/// Persist MCP server configs back to user settings. Internal helper.
fn save_mcp_config(
    state: &tauri::State<'_, crate::SharedState>,
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
            lux_core::SettingsScope::User,
            MCP_SERVERS_KEY.to_string(),
            value,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_text_content() {
        let result = json!({ "content": [{ "type": "text", "text": "hello" }, { "type": "text", "text": "world" }] });
        assert_eq!(flatten_tool_result(&result), "hello\nworld");
    }

    #[test]
    fn marks_error_results() {
        let result = json!({ "isError": true, "content": [{ "type": "text", "text": "boom" }] });
        assert!(flatten_tool_result(&result).starts_with("[tool error] boom"));
    }

    #[test]
    fn parses_tools_list() {
        let result = json!({ "tools": [{ "name": "search", "description": "d", "inputSchema": { "type": "object" } }] });
        let tools = parse_tools(&result);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "search");
    }

    #[test]
    fn rejects_bad_ids() {
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("has space"));
        assert!(is_valid_id("ctx7-server_1"));
    }
}
