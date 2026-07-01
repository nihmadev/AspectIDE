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
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
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
/// Hard cap on a single JSON-RPC line read from a server's stdout. A hostile or
/// buggy server can emit a multi-megabyte line; without a bound the reader buffers
/// it whole before any post-parse clamp, spiking memory or stalling parsing. On
/// overflow the connection is failed rather than parsed.
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
/// Provider function-name limit (OpenAI/Anthropic both cap at 64) applied to the
/// full namespaced `mcp__<id>__<tool>` name before it reaches a model request.
const MAX_TOOL_NAME_LEN: usize = 64;

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

/// Normalize a JSON-RPC response `id` back to the integer key we filed the pending
/// waiter under. We only ever *send* integer ids, but per JSON-RPC 2.0 the server
/// may legitimately echo that id back as a JSON string (`"7"`) rather than a number.
/// Matching only `as_u64` silently drops the string form — the pending call then never
/// resolves and hangs for the full request timeout even though its answer already
/// arrived. Accepts integer numbers (any JSON integer) and decimal-string ids; returns
/// `None` for notifications (no id), a null id, or a non-integer value.
fn normalize_response_id(id: Option<&Value>) -> Option<u64> {
    match id? {
        // Covers every JSON integer we could have sent (`as_u64` handles the full
        // range serde parses an integer literal into).
        Value::Number(number) => number.as_u64(),
        // A server that stringifies the echoed id (`"7"`); ignore surrounding space.
        Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    }
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
        let mut reader = BufReader::new(stdout);
        // Why this is the connection's terminal status once the loop ends: a clean
        // EOF means the server exited; an oversized line means a hostile/buggy server
        // flooded us and we fail the connection rather than buffer it whole.
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
                    // Correlate tolerantly: we always send integer ids, but a spec-
                    // compliant server may echo the id back as a JSON string ("1"),
                    // a float (1.0), etc. Matching only `as_u64` would drop those
                    // responses, stranding the caller until its ~30s timeout even
                    // though the answer already arrived. Normalize before lookup.
                    let Some(id) = normalize_response_id(message.get("id")) else {
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
                Ok(None) => break, // clean EOF
                Err(error) => {
                    exit_error = format!("MCP server stream error: {error}");
                    break;
                }
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

    // On a failed/timed-out handshake, don't keep a half-open server around: a hung
    // child plus its reader task would otherwise persist (consuming a process + a
    // pipe) until an explicit disconnect/reconnect. Mark dead, kill+reap the child,
    // drop the stdin/pending handles, and store an error-only status. Marking
    // `alive=false` first stops the reader's EOF handler from clobbering this status.
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
    let (stdin, pending, next_id, real_tool) = {
        let mut registry = registry().lock().await;
        let connection = registry
            .get_mut(server_id)
            .ok_or_else(|| format!("MCP server '{server_id}' is not connected"))?;
        if connection.state != "connected" {
            // Distinguish a server that never came up ("connecting"/"disconnected")
            // from one that came up and then died ("error"). The old blanket "is not
            // ready" reads like "still starting", so the model would just wait/retry a
            // crashed server forever. Point it at the actionable next step instead.
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
        // E31: the model may have been handed a sanitized alias for a tool whose real
        // name is not provider-safe. Map the requested name back to the server's real
        // tool name before the call so the sanitized alias stays invokable.
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
    // E33: a `tools/call` result with `isError: true` is an application-level failure
    // (the tool ran and reported it failed), not data. Returning it as `Ok` buries the
    // failure inside a success string ("[tool error] …") that the model can't reliably
    // distinguish from a tool that legitimately echoed that phrase. Surface it through
    // the actual error channel so the model's error handling fires.
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(format!("MCP tool '{tool}' reported an error: {flattened}"));
    }
    Ok(flattened)
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

/// Read one newline-delimited line, failing if it exceeds [`MAX_LINE_BYTES`]
/// before a newline arrives. Unlike `lines()`, which buffers an unbounded line into
/// memory, this caps the read so a server emitting a giant single line can't spike
/// memory — the connection is failed instead. Returns `Ok(None)` on clean EOF.
async fn read_capped_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Option<String>, String> {
    let mut buffer = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(|error| error.to_string())?;
        if available.is_empty() {
            // EOF: surface any trailing unterminated bytes, else signal end-of-stream.
            return if buffer.is_empty() {
                Ok(None)
            } else {
                Ok(Some(String::from_utf8_lossy(&buffer).into_owned()))
            };
        }
        if let Some(newline) = available.iter().position(|&byte| byte == b'\n') {
            buffer.extend_from_slice(&available[..newline]);
            reader.consume(newline + 1);
            return Ok(Some(String::from_utf8_lossy(&buffer).into_owned()));
        }
        let chunk = available.len();
        buffer.extend_from_slice(available);
        reader.consume(chunk);
        if buffer.len() > MAX_LINE_BYTES {
            return Err(format!(
                "JSON-RPC line exceeded {MAX_LINE_BYTES} bytes without a newline"
            ));
        }
    }
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
///
/// Each content part becomes one line: text parts contribute their raw text; every
/// other part (image/audio/resource/…) is serialized *whole* as JSON so the model
/// never receives a fragment sliced mid-token (E34). Any top-level `structuredContent`
/// is appended as a labelled JSON block rather than dropped. When the assembled body
/// exceeds [`MAX_RESULT_CHARS`] the clip happens on a whole-line boundary and a
/// machine-readable `[truncated: …]` note is appended so the model knows output was
/// cut instead of guessing (E30).
fn flatten_tool_result(result: &Value) -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for part in content {
            if part.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    lines.push(text.to_string());
                }
            } else {
                // Non-text part (image/resource/…): serialize the whole part as valid
                // JSON. `to_string` on a `serde_json::Value` cannot fail and never
                // produces a partial token, so the model always gets well-formed JSON.
                lines.push(part.to_string());
            }
        }
    }
    // `structuredContent` is a sibling of `content` (MCP 2025-06 machine-readable
    // payload). It was previously discarded entirely; keep it as a labelled, whole
    // JSON block so structured results aren't silently lost.
    if let Some(structured) = result.get("structuredContent") {
        if !structured.is_null() {
            lines.push(format!("[structuredContent] {structured}"));
        }
    }

    let mut out = if lines.is_empty() {
        // No content array at all: fall back to the raw result so nothing is lost.
        result.to_string()
    } else {
        lines.join("\n")
    };

    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        out = format!("[tool error] {out}");
    }

    clamp_result_body(out)
}

/// Clamp a flattened tool-result body to [`MAX_RESULT_CHARS`], appending a
/// machine-readable truncation note (mirrors `ai_read_file`'s `truncated` signal) so
/// the model can tell "this is the whole output" from "output was clipped". Clips on a
/// newline boundary when one exists within the cap so a JSON line isn't split
/// mid-token; only falls back to a char boundary if a single line already blows the cap.
fn clamp_result_body(body: String) -> String {
    let total = body.chars().count();
    if total <= MAX_RESULT_CHARS {
        return body;
    }
    let head: String = body.chars().take(MAX_RESULT_CHARS).collect();
    // Prefer to end on a whole line so we don't hand the model a truncated JSON object.
    let kept = match head.rfind('\n') {
        Some(newline) if newline > 0 => &head[..newline],
        _ => head.as_str(),
    };
    format!(
        "{kept}\n[truncated: output exceeded {MAX_RESULT_CHARS} chars ({total} total); \
         re-run the tool with narrower arguments to see the rest.]"
    )
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
            // A malformed tool name (spaces, dots, Unicode, or an over-long total)
            // violates provider function-name rules and would invalidate the ENTIRE
            // turn request. Previously such tools were dropped silently, making the
            // capability invisible to the model. Instead, derive a provider-safe alias
            // and keep the tool discoverable; `call_tool`/`resolve_tool_name` maps the
            // alias back to the server's real tool name on invoke. The id is already
            // `__`-free (see `is_valid_id`) and the alias segment is `__`-free, so the
            // dispatcher's split on the first `__` after the id stays correct.
            let plain = format!("mcp__{}__{}", connection.config.id, tool.name);
            let (namespaced, renamed) = if is_provider_safe_tool_name(&plain) {
                (plain, false)
            } else {
                let alias = format!(
                    "mcp__{}__{}",
                    connection.config.id,
                    sanitize_tool_segment(&connection.config.id, &tool.name)
                );
                // If even the sanitized alias can't be made safe (e.g. the server id
                // alone already consumes the length budget), only then skip — but say so.
                if !is_provider_safe_tool_name(&alias) {
                    tracing::warn!(
                        server = %connection.config.id,
                        tool = %tool.name,
                        "MCP tool name cannot be made provider-safe even after sanitizing; omitting"
                    );
                    continue;
                }
                tracing::info!(
                    server = %connection.config.id,
                    tool = %tool.name,
                    alias = %alias,
                    "exposing MCP tool under a sanitized alias (original name is not provider-safe)"
                );
                (alias, true)
            };
            let base_description = if tool.description.is_empty() {
                format!(
                    "MCP tool '{}' from server '{}'.",
                    tool.name, connection.config.name
                )
            } else {
                format!("[{}] {}", connection.config.name, tool.description)
            };
            // Surface the rename to the model so the true tool name isn't lost — the
            // server may reference it in its own docs/output.
            let description = if renamed {
                format!(
                    "{base_description} (Exposed under a sanitized name; the server's real tool \
                     name is '{}'.)",
                    tool.name
                )
            } else {
                base_description
            };
            defs.push(json!({
                "type": "function",
                "function": {
                    "name": namespaced,
                    "description": description,
                    "parameters": tool.input_schema,
                },
            }));
        }
    }
    defs
}

/// Whether a fully-namespaced tool name satisfies the providers' function-name
/// contract: non-empty, within [`MAX_TOOL_NAME_LEN`], and limited to the
/// `[A-Za-z0-9_-]` set both `OpenAI` and Anthropic accept. Keeps one bad MCP tool
/// from poisoning the whole tool-definitions array.
fn is_provider_safe_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_TOOL_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A short, deterministic hex tag derived from the real tool name. Used as a suffix so
/// two tools that sanitize/truncate to the same visible stem still get distinct,
/// reversible aliases (and so a truncated alias round-trips to exactly one real tool).
fn tool_name_tag(real: &str) -> String {
    // A tiny FNV-1a hash — no external deps, stable across runs, collision-resistant
    // enough for the few tools a single server exposes.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in real.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // 6 hex chars is plenty to disambiguate a handful of tools while leaving room in
    // the 64-char budget.
    format!("{:06x}", hash & 0xff_ffff)
}

/// Derive a provider-safe tool *segment* (the `<tool>` in `mcp__<id>__<tool>`) for a
/// tool whose real name isn't provider-safe. Replaces every disallowed character with
/// `_`, guarantees non-empty, and truncates to fit the 64-char namespaced budget while
/// reserving room for a `_<tag>` disambiguator so the alias stays reversible via
/// [`resolve_tool_name`]. `__` is collapsed to `_` so the dispatcher's `mcp__<id>__`
/// split can't be fooled by the alias.
fn sanitize_tool_segment(server_id: &str, real: &str) -> String {
    let mut cleaned = String::with_capacity(real.len());
    let mut prev_underscore = false;
    for ch in real.chars() {
        let mapped = if ch.is_ascii_alphanumeric() || ch == '-' {
            prev_underscore = false;
            ch
        } else {
            // Collapse runs of replacements (and literal "__") into a single "_" so the
            // segment stays `__`-free and the routing split remains unambiguous.
            if prev_underscore {
                continue;
            }
            prev_underscore = true;
            '_'
        };
        cleaned.push(mapped);
    }
    let cleaned = cleaned.trim_matches('_');
    let stem = if cleaned.is_empty() { "tool" } else { cleaned };

    let tag = tool_name_tag(real);
    // Budget for the full name is MAX_TOOL_NAME_LEN. Fixed overhead is
    // "mcp__" + id + "__" + "_" + tag. Whatever remains is for the stem.
    let fixed = "mcp__".len() + server_id.len() + "__".len() + 1 + tag.len();
    let stem_budget = MAX_TOOL_NAME_LEN.saturating_sub(fixed);
    let clipped_stem: String = stem.chars().take(stem_budget).collect();
    let clipped_stem = clipped_stem.trim_end_matches('_');
    if clipped_stem.is_empty() {
        // No room for any stem: fall back to just the tag (still unique + reversible).
        tag
    } else {
        format!("{clipped_stem}_{tag}")
    }
}

/// Map the tool name the model invoked back to the server's real tool name. The model
/// may call a tool by its real name (the common case) or by the sanitized alias we
/// exposed for a provider-unsafe name (see [`sanitize_tool_segment`]). Prefer an exact
/// real-name match; otherwise find the unique tool whose alias equals `requested`. Falls
/// back to the requested name unchanged so unknown names still reach the server (which
/// returns its own "unknown tool" error).
fn resolve_tool_name(server_id: &str, tools: &[McpToolInfo], requested: &str) -> String {
    if tools.iter().any(|tool| tool.name == requested) {
        return requested.to_string();
    }
    for tool in tools {
        if sanitize_tool_segment(server_id, &tool.name) == requested {
            return tool.name.clone();
        }
    }
    requested.to_string()
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
    // The advertised id rule (letters/digits/-/_, no "__") is enforced in
    // `connect_server`, but the disabled-add path never reaches that check — so an
    // invalid id would be persisted silently here and later mis-route (or drop) the
    // server's tools via the `mcp__<id>__<tool>` split once enabled. Reject before any
    // read/save so both the enabled and disabled add paths honor the rule consistently,
    // using connect_server's exact error message.
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

/// Enable or disable a server. Enabling connects it; disabling disconnects the
/// live session. Returns the server's live status so the UI reflects the result.
#[tauri::command]
pub async fn mcp_enable(
    state: tauri::State<'_, crate::SharedState>,
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
        // Previously this only persisted the flag, leaving an "enabled" server
        // disconnected until a separate reconnect/restart. Connect it now so the
        // tool is actually available; a connect failure surfaces as error status.
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

    #[test]
    fn rejects_provider_unsafe_tool_names() {
        assert!(is_provider_safe_tool_name("mcp__ctx7__search"));
        assert!(!is_provider_safe_tool_name("mcp__ctx7__has space"));
        assert!(!is_provider_safe_tool_name("mcp__ctx7__dot.name"));
        assert!(!is_provider_safe_tool_name("mcp__ctx7__naïve")); // non-ASCII
        assert!(!is_provider_safe_tool_name(&format!(
            "mcp__ctx7__{}",
            "x".repeat(64)
        )));
        assert!(!is_provider_safe_tool_name(""));
    }

    #[tokio::test]
    async fn capped_line_reads_until_newline_and_eof() {
        let data = b"first line\nsecond".to_vec();
        let mut reader = BufReader::new(&data[..]);
        assert_eq!(
            read_capped_line(&mut reader).await.unwrap().as_deref(),
            Some("first line")
        );
        // Trailing unterminated bytes are surfaced, then clean EOF.
        assert_eq!(
            read_capped_line(&mut reader).await.unwrap().as_deref(),
            Some("second")
        );
        assert_eq!(read_capped_line(&mut reader).await.unwrap(), None);
    }

    #[tokio::test]
    async fn capped_line_fails_on_oversized_line() {
        // A line longer than the cap with no newline must error, not buffer forever.
        let data = vec![b'a'; MAX_LINE_BYTES + 16];
        let mut reader = BufReader::new(&data[..]);
        assert!(read_capped_line(&mut reader).await.is_err());
    }

    // ── E29: tolerant JSON-RPC response-id correlation ──
    #[test]
    fn normalizes_response_ids_across_json_shapes() {
        // Integer id — the shape we send.
        assert_eq!(normalize_response_id(Some(&json!(7))), Some(7));
        // String id — a spec-compliant server that echoes our id back as a string must
        // still resolve, not hang for the full request timeout.
        assert_eq!(normalize_response_id(Some(&json!("7"))), Some(7));
        assert_eq!(normalize_response_id(Some(&json!(" 7 "))), Some(7));
        // Non-correlatable shapes are ignored (notification / bad id).
        assert_eq!(normalize_response_id(None), None);
        assert_eq!(normalize_response_id(Some(&Value::Null)), None);
        assert_eq!(normalize_response_id(Some(&json!("abc"))), None);
        assert_eq!(normalize_response_id(Some(&json!(7.5))), None);
        assert_eq!(normalize_response_id(Some(&json!(-1))), None);
    }

    // ── E31: provider-unsafe tool names get a reversible sanitized alias ──
    fn tool(name: &str) -> McpToolInfo {
        McpToolInfo {
            name: name.to_string(),
            description: String::new(),
            input_schema: json!({ "type": "object" }),
        }
    }

    #[test]
    fn sanitized_alias_is_provider_safe_and_reversible() {
        let server = "ctx7";
        let real = "search.docs v2"; // spaces + dot → not provider-safe
        let alias = sanitize_tool_segment(server, real);
        let namespaced = format!("mcp__{server}__{alias}");
        // The alias must be usable in a provider request…
        assert!(is_provider_safe_tool_name(&namespaced));
        assert!(!alias.contains("__")); // routing split stays unambiguous
                                        // …and must round-trip back to the real tool name on invoke.
        let tools = vec![tool(real), tool("plain_tool")];
        assert_eq!(resolve_tool_name(server, &tools, &alias), real);
        // An exact real-name call still resolves to itself.
        assert_eq!(
            resolve_tool_name(server, &tools, "plain_tool"),
            "plain_tool"
        );
        // An unknown name falls through unchanged (server reports its own error).
        assert_eq!(resolve_tool_name(server, &tools, "nope"), "nope");
    }

    #[test]
    fn overlong_tool_names_sanitize_within_budget_and_stay_distinct() {
        let server = "srv";
        let a = "x".repeat(200);
        let b = format!("{}y", "x".repeat(199));
        let alias_a = sanitize_tool_segment(server, &a);
        let alias_b = sanitize_tool_segment(server, &b);
        assert!(is_provider_safe_tool_name(&format!(
            "mcp__{server}__{alias_a}"
        )));
        assert!(is_provider_safe_tool_name(&format!(
            "mcp__{server}__{alias_b}"
        )));
        // Distinct real names must not collapse to the same alias (hash suffix).
        assert_ne!(alias_a, alias_b);
        let tools = vec![tool(&a), tool(&b)];
        assert_eq!(resolve_tool_name(server, &tools, &alias_a), a);
        assert_eq!(resolve_tool_name(server, &tools, &alias_b), b);
    }

    // ── E30: truncation carries a machine-readable note on a line boundary ──
    #[test]
    fn truncation_appends_machine_readable_note() {
        let big = "a".repeat(MAX_RESULT_CHARS + 500);
        let clamped = clamp_result_body(big);
        assert!(clamped.contains("[truncated:"));
        assert!(clamped.contains("re-run the tool"));
        assert!(clamped.chars().count() < MAX_RESULT_CHARS + 500);
    }

    #[test]
    fn short_output_is_not_annotated() {
        let out = clamp_result_body("small".to_string());
        assert_eq!(out, "small");
    }

    // ── E34: non-text parts serialized whole; structuredContent preserved ──
    #[test]
    fn non_text_parts_serialize_as_valid_json() {
        let result = json!({
            "content": [
                { "type": "text", "text": "caption" },
                { "type": "image", "data": "AAAA", "mimeType": "image/png" }
            ]
        });
        let flat = flatten_tool_result(&result);
        let (_caption, image_line) = flat.split_once('\n').expect("two lines");
        // The image line must be parseable JSON — never a byte-truncated fragment.
        let parsed: Value = serde_json::from_str(image_line).expect("valid JSON part");
        assert_eq!(parsed.get("type").and_then(Value::as_str), Some("image"));
    }

    #[test]
    fn structured_content_is_preserved() {
        let result = json!({
            "content": [{ "type": "text", "text": "ok" }],
            "structuredContent": { "rows": 3 }
        });
        let flat = flatten_tool_result(&result);
        assert!(flat.contains("[structuredContent]"));
        assert!(flat.contains("\"rows\":3"));
    }
}
