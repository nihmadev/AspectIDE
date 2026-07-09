use lsp_types::PublishDiagnosticsParams;
use aspect_core::{AppError, AppResult};
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::mpsc,
};

use crate::{diagnostics_update_from_publish, DiagnosticsUpdate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspFrame {
    pub content: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum LspNotification {
    PublishDiagnostics(DiagnosticsUpdate),
    Other { method: String },
}

#[derive(Debug, Clone)]
pub struct LspResponse {
    pub id: u64,
    pub error: Option<String>,
    pub result: Option<Value>,
}

pub fn encode_lsp_message(value: &Value) -> AppResult<Vec<u8>> {
    let content = serde_json::to_vec(value)?;
    let mut message = format!("Content-Length: {}\r\n\r\n", content.len()).into_bytes();
    message.extend_from_slice(&content);
    Ok(message)
}

pub fn drain_lsp_frames(buffer: &mut Vec<u8>) -> AppResult<Vec<LspFrame>> {
    const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
    let mut frames = Vec::new();

    while let Some(header_end) = find_header_end(buffer) {
        let headers = std::str::from_utf8(&buffer[..header_end])
            .map_err(|error| AppError::Service(format!("invalid LSP header encoding: {error}")))?;
        let content_length = parse_content_length(headers)?;
        let frame_start = header_end + 4;
        let frame_end = match frame_start.checked_add(content_length) {
            Some(frame_end) if content_length <= MAX_FRAME_BYTES => frame_end,
            // Oversized or overflowing Content-Length: the header is poisoned.
            // Skip past it (resyncing to the next buffered header) instead of
            // returning Err, which would discard frames already parsed in this
            // batch and hang the callers awaiting their responses.
            _ => {
                resync_past_poisoned_header(buffer, header_end);
                continue;
            }
        };

        if buffer.len() < frame_end {
            break;
        }

        let content = buffer[frame_start..frame_end].to_vec();
        buffer.drain(..frame_end);
        frames.push(LspFrame { content });
    }

    Ok(frames)
}

pub fn parse_lsp_notification(frame: &LspFrame) -> AppResult<Option<LspNotification>> {
    let value: Value = serde_json::from_slice(&frame.content)?;
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return Ok(None);
    };

    if method != "textDocument/publishDiagnostics" {
        return Ok(Some(LspNotification::Other {
            method: method.to_string(),
        }));
    }

    let params = value.get("params").cloned().ok_or_else(|| {
        AppError::Service("publishDiagnostics notification is missing params".into())
    })?;
    let params: PublishDiagnosticsParams = serde_json::from_value(params)?;
    Ok(Some(LspNotification::PublishDiagnostics(
        diagnostics_update_from_publish(params),
    )))
}

pub fn parse_lsp_response(frame: &LspFrame) -> AppResult<Option<u64>> {
    let value: Value = serde_json::from_slice(&frame.content)?;
    Ok(parse_lsp_response_value(&value).map(|response| response.id))
}

pub async fn read_lsp_stdout<R>(
    mut stdout: R,
    diagnostics_tx: mpsc::UnboundedSender<DiagnosticsUpdate>,
    response_tx: mpsc::UnboundedSender<LspResponse>,
) where
    R: AsyncRead + Unpin,
{
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8192];

    loop {
        let read = match stdout.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        buffer.extend_from_slice(&chunk[..read]);

        let Ok(frames) = drain_lsp_frames(&mut buffer) else {
            const MARKER: &[u8] = b"content-length:";
            let skip = buffer
                .windows(MARKER.len())
                .position(|window| window.eq_ignore_ascii_case(MARKER))
                .map_or(buffer.len(), |pos| pos.max(1));
            buffer.drain(..skip);
            continue;
        };

        for frame in frames {
            if let Ok(Some(notification)) = parse_lsp_notification(&frame) {
                if let LspNotification::PublishDiagnostics(update) = notification {
                    let _ = diagnostics_tx.send(update);
                }
                continue;
            }

            if let Ok(value) = serde_json::from_slice::<Value>(&frame.content) {
                if let Some(response) = parse_lsp_response_value(&value) {
                    let _ = response_tx.send(response);
                }
            }
        }
    }
}

pub async fn drain_stderr<R>(mut stderr: R)
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 4096];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

fn parse_lsp_response_value(value: &Value) -> Option<LspResponse> {
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return None;
    }
    if value.get("method").is_some() {
        return None;
    }

    let id = value.get("id")?.as_u64()?;
    let error = value.get("error").map(lsp_error_to_string);
    let result = value.get("result").cloned();
    Some(LspResponse { id, error, result })
}

fn lsp_error_to_string(value: &Value) -> String {
    value
        .get("message")
        .and_then(Value::as_str)
        .map_or_else(|| value.to_string(), str::to_string)
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

/// Drops a poisoned header block (oversized/overflowing Content-Length),
/// resyncing to the next buffered `Content-Length` header if one exists and
/// otherwise clearing the buffer. Always advances by at least the header block,
/// so the drain loop keeps making forward progress.
fn resync_past_poisoned_header(buffer: &mut Vec<u8>, header_end: usize) {
    const MARKER: &[u8] = b"content-length:";
    let header_block_end = header_end + 4;
    let skip = buffer[header_block_end..]
        .windows(MARKER.len())
        .position(|window| window.eq_ignore_ascii_case(MARKER))
        .map_or(buffer.len(), |pos| header_block_end + pos);
    buffer.drain(..skip);
}

fn parse_content_length(headers: &str) -> AppResult<usize> {
    for line in headers.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse::<usize>().map_err(|error| {
                AppError::Service(format!("invalid LSP Content-Length: {error}"))
            });
        }
    }

    Err(AppError::Service(
        "LSP frame is missing Content-Length header".into(),
    ))
}

