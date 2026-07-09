use std::{
    path::PathBuf,
    process::Stdio,
};

use aspect_core::AspectEvent;
use tauri::{State, Emitter};
use tokio::time::{timeout, Duration};

use crate::{resolve_workspace_path_from_root, workspace_root, SharedState};
use aspect_agent_tools::{
    console_decode::decode_console_bytes,
    output_truncate::truncate_shell_output_flagged,
    process_kill::kill_process_tree,
    shell_command::shell_command,
    types::{AiShellClassification, AiShellResponse},
};

const AI_SHELL_DEFAULT_TIMEOUT_SECS: u64 = 120;
const AI_SHELL_MAX_TIMEOUT_SECS: u64 = 600;

#[tauri::command]
pub fn ai_shell_classify(command: String) -> AiShellClassification {
    let report = aspect_security::classify_shell_command(command.trim());
    AiShellClassification {
        blocked: report.blocked,
        warnings: report.warnings,
        read_only: report.read_only,
    }
}

#[tauri::command]
pub async fn ai_shell(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    command: String,
    cwd: Option<PathBuf>,
    timeout_secs: Option<u64>,
    tool_call_id: Option<String>,
) -> Result<AiShellResponse, String> {
    let root = workspace_root(&state)?;
    let cwd = match cwd {
        Some(path) => resolve_workspace_path_from_root(&root, &path, true)?,
        None => root.clone(),
    };
    if !cwd.is_dir() {
        return Err(format!("shell cwd is not a directory: {}", cwd.display()));
    }
    let command = command.trim().to_string();
    if command.is_empty() {
        return Err("shell command must not be empty".to_string());
    }

    let safety = aspect_security::classify_shell_command(&command);
    if let Some(reason) = safety.blocked {
        return Err(format!(
            "AspectIDE blocked this command for safety ({reason}). If this is genuinely intended, run it manually in the integrated terminal."
        ));
    }

    let timeout_secs = timeout_secs
        .unwrap_or(AI_SHELL_DEFAULT_TIMEOUT_SECS)
        .clamp(1, AI_SHELL_MAX_TIMEOUT_SECS);

    let started = std::time::Instant::now();
    let mut process = shell_command(&command);
    process.current_dir(dunce::simplified(&cwd));
    process.stdin(Stdio::null());
    process.stdout(Stdio::piped());
    process.stderr(Stdio::piped());
    process.kill_on_drop(true);

    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => return Err(format!("Failed to start shell command: {error}")),
    };
    let child_pid = child.id();
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();

    let shared_stdout: std::sync::Arc<tokio::sync::Mutex<Vec<u8>>> =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let shared_stderr: std::sync::Arc<tokio::sync::Mutex<Vec<u8>>> =
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let mirror = spawn_shell_mirror(
        app,
        &command,
        &dunce::simplified(&cwd).to_string_lossy(),
        std::sync::Arc::clone(&shared_stdout),
        std::sync::Arc::clone(&shared_stderr),
        tool_call_id,
    );

    let collect_stdout = std::sync::Arc::clone(&shared_stdout);
    let collect_stderr = std::sync::Arc::clone(&shared_stderr);
    let collect = async {
        use tokio::io::AsyncReadExt;
        const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;
        const PIPE_DRAIN_GRACE_SECS: u64 = 2;

        async fn stream_into(
            pipe: Option<&mut tokio::process::ChildStdout>,
            sink: &std::sync::Arc<tokio::sync::Mutex<Vec<u8>>>,
            cap: usize,
        ) {
            let Some(pipe) = pipe else { return };
            let mut chunk = [0u8; 16 * 1024];
            loop {
                match pipe.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut buffer = sink.lock().await;
                        if buffer.len() < cap {
                            let room = cap - buffer.len();
                            buffer.extend_from_slice(&chunk[..n.min(room)]);
                        }
                    }
                }
            }
        }

        let read_stdout = stream_into(stdout_pipe.as_mut(), &collect_stdout, MAX_CAPTURE_BYTES);
        let read_stderr = async {
            let Some(pipe) = stderr_pipe.as_mut() else { return };
            let mut chunk = [0u8; 16 * 1024];
            loop {
                match pipe.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut buffer = collect_stderr.lock().await;
                        if buffer.len() < MAX_CAPTURE_BYTES {
                            let room = MAX_CAPTURE_BYTES - buffer.len();
                            buffer.extend_from_slice(&chunk[..n.min(room)]);
                        }
                    }
                }
            }
        };
        let drain = async { tokio::join!(read_stdout, read_stderr) };
        let mut drain = Box::pin(drain);
        let status = tokio::select! {
            _ = drain.as_mut() => child.wait().await,
            status = child.wait() => {
                let _ = timeout(Duration::from_secs(PIPE_DRAIN_GRACE_SECS), drain.as_mut()).await;
                status
            }
        };
        status
    };

    let output_result = timeout(Duration::from_secs(timeout_secs), collect).await;
    let duration_ms = started.elapsed().as_millis();

    match output_result {
        Ok(Ok(status)) => {
            let stdout_buf = shared_stdout.lock().await.clone();
            let stderr_buf = shared_stderr.lock().await.clone();
            let (stdout, stdout_truncated) =
                truncate_shell_output_flagged(&decode_console_bytes(&stdout_buf));
            let (stderr, stderr_truncated) =
                truncate_shell_output_flagged(&decode_console_bytes(&stderr_buf));
            mirror.finish(format!(
                "\r\n\x1b[2m[exit {} in {duration_ms}ms]\x1b[0m\r\n",
                status
                    .code()
                    .map_or_else(|| "?".to_string(), |code| code.to_string())
            ));
            Ok(AiShellResponse {
                workspace_root: root,
                cwd,
                command,
                exit_code: status.code(),
                duration_ms,
                stdout,
                stderr,
                timed_out: false,
                warnings: safety.warnings,
                read_only: safety.read_only,
                stdout_truncated,
                stderr_truncated,
            })
        }
        Ok(Err(error)) => {
            mirror.finish(format!("\r\n\x1b[31m[failed to run: {error}]\x1b[0m\r\n"));
            Err(format!("Failed to run shell command: {error}"))
        }
        Err(_) => {
            kill_process_tree(child_pid).await;
            let _ = child.start_kill();
            mirror.finish(format!(
                "\r\n\x1b[31m[timeout after {timeout_secs}s - process tree killed]\x1b[0m\r\n"
            ));
            let partial_stdout = {
                let buf = shared_stdout.lock().await;
                decode_console_bytes(&buf)
            };
            let partial_stderr = {
                let buf = shared_stderr.lock().await;
                decode_console_bytes(&buf)
            };
            let (stdout, stdout_truncated) = truncate_shell_output_flagged(&partial_stdout);
            let (stderr_body, stderr_truncated) = truncate_shell_output_flagged(&partial_stderr);
            Ok(AiShellResponse {
                workspace_root: root,
                cwd,
                command,
                exit_code: None,
                duration_ms,
                stdout,
                stderr: if partial_stderr.is_empty() {
                    format!("Shell command timed out after {timeout_secs} seconds")
                } else {
                    format!(
                        "{stderr_body}\n---\nShell command timed out after {timeout_secs} seconds"
                    )
                },
                timed_out: true,
                warnings: safety.warnings,
                read_only: safety.read_only,
                stdout_truncated,
                stderr_truncated,
            })
        }
    }
}

struct ShellMirror {
    done: Option<tokio::sync::oneshot::Sender<String>>,
}

impl ShellMirror {
    fn finish(mut self, status_line: String) {
        if let Some(done) = self.done.take() {
            let _ = done.send(status_line);
        }
    }
}

const SHELL_MIRROR_POLL_MS: u64 = 90;

fn spawn_shell_mirror(
    app: tauri::AppHandle,
    command: &str,
    cwd_display: &str,
    stdout: std::sync::Arc<tokio::sync::Mutex<Vec<u8>>>,
    stderr: std::sync::Arc<tokio::sync::Mutex<Vec<u8>>>,
    tool_call_id: Option<String>,
) -> ShellMirror {
    let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<String>();
    let banner = format!(
        "\r\n\x1b[1;36m$\x1b[0m \x1b[1m{}\x1b[0m \x1b[2m({cwd_display})\x1b[0m\r\n",
        command.replace(['\r', '\n'], " ")
    );
    tokio::spawn(async move {
        let _ = app.emit(
            "aspect://event",
            AspectEvent::AiShellOutput {
                data: banner,
                tool_call_id: tool_call_id.clone(),
            },
        );
        let mut sent_stdout = 0usize;
        let mut sent_stderr = 0usize;
        loop {
            let status_line = tokio::select! {
                status = &mut done_rx => Some(status.unwrap_or_default()),
                () = tokio::time::sleep(std::time::Duration::from_millis(SHELL_MIRROR_POLL_MS)) => None,
            };
            emit_shell_mirror_delta(&app, &stdout, &mut sent_stdout, tool_call_id.as_deref()).await;
            emit_shell_mirror_delta(&app, &stderr, &mut sent_stderr, tool_call_id.as_deref()).await;
            if let Some(status_line) = status_line {
                if !status_line.is_empty() {
                    let _ = app.emit(
                        "aspect://event",
                        AspectEvent::AiShellOutput {
                            data: status_line,
                            tool_call_id: tool_call_id.clone(),
                        },
                    );
                }
                break;
            }
        }
    });
    ShellMirror {
        done: Some(done_tx),
    }
}

async fn emit_shell_mirror_delta(
    app: &tauri::AppHandle,
    sink: &std::sync::Arc<tokio::sync::Mutex<Vec<u8>>>,
    sent: &mut usize,
    tool_call_id: Option<&str>,
) {
    let chunk = {
        let buffer = sink.lock().await;
        if buffer.len() <= *sent {
            return;
        }
        let pending = &buffer[*sent..];
        let valid = match std::str::from_utf8(pending) {
            Ok(text) => text.len(),
            Err(error) => error.valid_up_to(),
        };
        if valid == 0 {
            return;
        }
        let text = String::from_utf8_lossy(&pending[..valid]).into_owned();
        *sent += valid;
        text
    };
    let _ = app.emit(
        "aspect://event",
        AspectEvent::AiShellOutput {
            data: chunk,
            tool_call_id: tool_call_id.map(str::to_string),
        },
    );
}
