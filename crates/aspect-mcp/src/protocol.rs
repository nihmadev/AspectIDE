use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt};
use tokio::process::ChildStdin;
use tokio::sync::{oneshot, Mutex as AsyncMutex};

use crate::types::MAX_LINE_BYTES;

pub fn normalize_response_id(id: Option<&Value>) -> Option<u64> {
    match id? {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    }
}

pub async fn send_request(
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

pub async fn send_request_with_id(
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

pub async fn send_notification(
    stdin: &Arc<AsyncMutex<ChildStdin>>,
    method: &str,
) -> Result<(), String> {
    let payload = json!({ "jsonrpc": "2.0", "method": method });
    write_line(stdin, &payload).await
}

pub async fn read_capped_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Option<String>, String> {
    let mut buffer = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(|error| error.to_string())?;
        if available.is_empty() {
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

pub async fn write_line(stdin: &Arc<AsyncMutex<ChildStdin>>, payload: &Value) -> Result<(), String> {
    let mut line = serde_json::to_string(payload).map_err(|error| error.to_string())?;
    line.push('\n');
    let mut guard = stdin.lock().await;
    guard
        .write_all(line.as_bytes())
        .await
        .map_err(|error| format!("MCP write failed: {error}"))?;
    guard.flush().await.map_err(|error| error.to_string())
}

