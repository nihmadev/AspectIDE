use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use base64::Engine;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const LOCAL_STT_COMMAND_ENV: &str = "ASPECT_STT_COMMAND";
const LOCAL_STT_MODEL_ENV: &str = "ASPECT_STT_MODEL";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Hard wall-clock cap for a single STT invocation. A misconfigured/hung STT binary
/// must never wedge the worker (blocking path) or leak a child indefinitely.
const STT_TIMEOUT_SECS: u64 = 120;
/// Per-stream capture cap so a chatty STT tool can't balloon memory.
const STT_MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceInputProviderStatus {
    provider: String,
    available: bool,
    detail: String,
    command: Option<String>,
    model_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTranscriptionRequest {
    pub provider: String,
    pub audio_base64: String,
    pub mime_type: String,
    pub language: Option<String>,
    pub command: Option<String>,
    pub model_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTranscriptionResult {
    text: String,
}

pub fn status(
    provider: String,
    command: Option<String>,
    model_path: Option<PathBuf>,
) -> VoiceInputProviderStatus {
    match provider.as_str() {
        "native-webview" => VoiceInputProviderStatus {
            provider,
            available: true,
            detail: "Native WebView speech recognition is checked in the frontend runtime"
                .to_string(),
            command: None,
            model_path: None,
        },
        "local" => local_status(command, model_path),
        unknown => VoiceInputProviderStatus {
            provider: unknown.to_string(),
            available: false,
            detail: "Unknown voice input provider".to_string(),
            command: None,
            model_path: None,
        },
    }
}

pub fn transcribe_local_blocking(request: VoiceTranscriptionRequest) -> Result<String, String> {
    if request.provider != "local" {
        return Err("only local voice transcription is supported by this command".to_string());
    }
    let status = local_status(request.command.clone(), request.model_path.clone());
    if !status.available {
        return Err(status.detail);
    }
    let command = status
        .command
        .ok_or_else(|| "Local STT command is not configured".to_string())?;
    let audio = base64::engine::general_purpose::STANDARD
        .decode(request.audio_base64.as_bytes())
        .map_err(|error| format!("Invalid recorded audio: {error}"))?;
    if audio.is_empty() {
        return Err("Recorded audio is empty".to_string());
    }
    run_local_stt_command_blocking(
        &command,
        &audio,
        &request.mime_type,
        request.language.as_deref(),
        status.model_path.as_deref(),
    )
}

pub async fn transcribe_local(
    request: VoiceTranscriptionRequest,
) -> Result<VoiceTranscriptionResult, String> {
    if request.provider != "local" {
        return Err("only local voice transcription is supported by this command".to_string());
    }

    let status = local_status(request.command.clone(), request.model_path.clone());
    if !status.available {
        return Err(status.detail);
    }

    let command = status
        .command
        .ok_or_else(|| "Local STT command is not configured".to_string())?;
    let audio = base64::engine::general_purpose::STANDARD
        .decode(request.audio_base64.as_bytes())
        .map_err(|error| format!("Invalid recorded audio: {error}"))?;
    if audio.is_empty() {
        return Err("Recorded audio is empty".to_string());
    }

    let text = run_local_stt_command(
        &command,
        &audio,
        &request.mime_type,
        request.language.as_deref(),
        status.model_path.as_deref(),
    )
    .await?;
    Ok(VoiceTranscriptionResult { text })
}

fn local_status(command: Option<String>, model_path: Option<PathBuf>) -> VoiceInputProviderStatus {
    let command = command.and_then(non_empty_string).or_else(|| {
        env::var(LOCAL_STT_COMMAND_ENV)
            .ok()
            .and_then(non_empty_string)
    });
    let model_path = model_path.or_else(|| env_path(LOCAL_STT_MODEL_ENV));

    let Some(command_value) = command else {
        return VoiceInputProviderStatus {
            provider: "local".to_string(),
            available: false,
            detail: format!("Set {LOCAL_STT_COMMAND_ENV} or AI settings Local STT command"),
            command: None,
            model_path,
        };
    };

    let Some(executable) = first_command_token(&command_value) else {
        return VoiceInputProviderStatus {
            provider: "local".to_string(),
            available: false,
            detail: "Local STT command is empty".to_string(),
            command: Some(command_value),
            model_path,
        };
    };

    if !command_token_available(&executable) {
        return VoiceInputProviderStatus {
            provider: "local".to_string(),
            available: false,
            detail: format!("Local STT executable not found: {executable}"),
            command: Some(command_value),
            model_path,
        };
    }

    if let Some(model) = &model_path {
        if !model.exists() {
            return VoiceInputProviderStatus {
                provider: "local".to_string(),
                available: false,
                detail: format!("Local STT model path does not exist: {}", model.display()),
                command: Some(command_value),
                model_path,
            };
        }
    }

    VoiceInputProviderStatus {
        provider: "local".to_string(),
        available: true,
        detail: "Local STT command is configured".to_string(),
        command: Some(command_value),
        model_path,
    }
}

/// RAII guard that removes the scratch audio file on drop, so every early return /
/// error / timeout path cleans it up (no leaked temp files).
struct TempAudioGuard(PathBuf);

impl Drop for TempAudioGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn run_local_stt_command(
    command_template: &str,
    audio: &[u8],
    mime_type: &str,
    language: Option<&str>,
    model_path: Option<&Path>,
) -> Result<String, String> {
    let audio_path = write_temp_audio(audio, mime_type)?;
    let _audio_guard = TempAudioGuard(audio_path.clone());
    let command_line = render_stt_command(
        command_template,
        &audio_path,
        mime_type,
        language,
        model_path,
    );
    let mut command = local_stt_shell_command(&command_line);
    // Capture pipes (drained concurrently) and keep the child handle so a timeout can
    // actually kill the tree instead of detaching an orphaned STT process.
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    command.kill_on_drop(true);

    let mut child = command
        .spawn()
        .map_err(|error| format!("Local STT command failed to start: {error}"))?;
    let child_pid = child.id();
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let collect = async {
        let (out, err) = tokio::join!(
            read_stt_stream(stdout_pipe.as_mut()),
            read_stt_stream(stderr_pipe.as_mut()),
        );
        let status = child.wait().await;
        (status, out, err)
    };

    let Ok((status, out, err)) =
        tokio::time::timeout(Duration::from_secs(STT_TIMEOUT_SECS), collect).await
    else {
        kill_stt_process_tree(child_pid).await;
        return Err(format!(
            "Local STT command timed out after {STT_TIMEOUT_SECS} seconds"
        ));
    };
    let status = status.map_err(|error| format!("Local STT command failed: {error}"))?;
    parse_stt_output(Ok(std::process::Output {
        status,
        stdout: out,
        stderr: err,
    }))
}

/// Drain one async child pipe to EOF, capping the buffer at [`STT_MAX_CAPTURE_BYTES`]
/// (kept reading past the cap so the pipe never blocks the child).
async fn read_stt_stream<R>(pipe: Option<&mut R>) -> Vec<u8>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut captured = Vec::new();
    let Some(pipe) = pipe else {
        return captured;
    };
    // Heap-allocated so it isn't part of the awaited future's stack frame.
    let mut scratch = vec![0u8; 16 * 1024];
    loop {
        match pipe.read(&mut scratch).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                if captured.len() < STT_MAX_CAPTURE_BYTES {
                    let room = STT_MAX_CAPTURE_BYTES - captured.len();
                    captured.extend_from_slice(&scratch[..read.min(room)]);
                }
            }
        }
    }
    captured
}

/// Kill the timed-out STT process and everything it spawned (the shell plus the STT
/// binary it launched), so nothing survives the call.
async fn kill_stt_process_tree(pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    #[cfg(windows)]
    {
        let mut command = tokio::process::Command::new("taskkill");
        command
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW);
        let _ = command.output().await;
    }
    #[cfg(not(windows))]
    {
        let _ = tokio::process::Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{pid}"))
            .output()
            .await;
    }
}

fn run_local_stt_command_blocking(
    command_template: &str,
    audio: &[u8],
    mime_type: &str,
    language: Option<&str>,
    model_path: Option<&Path>,
) -> Result<String, String> {
    let audio_path = write_temp_audio(audio, mime_type)?;
    let _audio_guard = TempAudioGuard(audio_path.clone());
    let command_line = render_stt_command(
        command_template,
        &audio_path,
        mime_type,
        language,
        model_path,
    );
    let mut command = local_stt_shell_command_blocking(&command_line);
    // Null stdin + captured pipes so the child can't block on input and a deadline can
    // kill a hung tool — the blocking path previously had NO timeout at all.
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("Local STT command failed to start: {error}"))?;

    // Drain pipes on threads so a chatty tool can't deadlock on a full pipe, while the
    // main thread enforces the deadline via `try_wait` and kills the tree on timeout.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_handle = std::thread::spawn(move || read_stt_stream_blocking(stdout_pipe));
    let stderr_handle = std::thread::spawn(move || read_stt_stream_blocking(stderr_pipe));

    let deadline = std::time::Instant::now() + Duration::from_secs(STT_TIMEOUT_SECS);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    kill_stt_process_tree_blocking(child.id());
                    return Err(format!(
                        "Local STT command timed out after {STT_TIMEOUT_SECS} seconds"
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(format!("Local STT command failed: {error}")),
        }
    };
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    parse_stt_output(Ok(std::process::Output {
        status,
        stdout,
        stderr,
    }))
}

/// Blocking-pipe drainer mirroring [`read_stt_stream`] for the synchronous path.
fn read_stt_stream_blocking<R: std::io::Read>(pipe: Option<R>) -> Vec<u8> {
    let mut captured = Vec::new();
    let Some(mut pipe) = pipe else {
        return captured;
    };
    let mut scratch = [0u8; 16 * 1024];
    loop {
        match pipe.read(&mut scratch) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                if captured.len() < STT_MAX_CAPTURE_BYTES {
                    let room = STT_MAX_CAPTURE_BYTES - captured.len();
                    captured.extend_from_slice(&scratch[..read.min(room)]);
                }
            }
        }
    }
    captured
}

// `std::process::Child::id` returns a bare `u32` (unlike tokio's `Option<u32>`).
#[cfg(windows)]
fn kill_stt_process_tree_blocking(pid: u32) {
    use std::os::windows::process::CommandExt;
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

#[cfg(not(windows))]
fn kill_stt_process_tree_blocking(pid: u32) {
    // Negative pid targets the whole process group (the shell leads it via
    // `process_group(0)`), so the STT binary it spawned dies too.
    let _ = Command::new("kill")
        .arg("-KILL")
        .arg(format!("-{pid}"))
        .output();
}

fn parse_stt_output(output: Result<std::process::Output, String>) -> Result<String, String> {
    let output = output?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("Local STT command exited with {}", output.status)
        } else {
            stderr
        });
    }
    let transcript = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if transcript.is_empty() {
        return Err("Local STT command returned no transcript".to_string());
    }
    Ok(transcript)
}

fn local_stt_shell_command_blocking(command_line: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("cmd");
        command.arg("/C").arg(command_line);
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new("sh");
        command.arg("-lc").arg(command_line);
        // Own process group so a timeout can group-kill the shell + STT binary.
        command.process_group(0);
        command
    }
}

fn render_stt_command(
    command_template: &str,
    audio_path: &Path,
    mime_type: &str,
    language: Option<&str>,
    model_path: Option<&Path>,
) -> String {
    let audio_placeholder = format!("{{{}}}", "audio");
    let mime_placeholder = format!("{{{}}}", "mime");
    let language_placeholder = format!("{{{}}}", "language");
    let model_placeholder = format!("{{{}}}", "model");
    let audio = shell_quote(&audio_path.to_string_lossy());
    let mime = shell_quote(mime_type);
    let language = shell_quote(language.unwrap_or("auto"));
    let model = model_path
        .map(|path| shell_quote(&path.to_string_lossy()))
        .unwrap_or_default();
    let mut command = command_template
        .replace(&audio_placeholder, &audio)
        .replace(&mime_placeholder, &mime)
        .replace(&language_placeholder, &language)
        .replace(&model_placeholder, &model);
    if !command_template.contains(&audio_placeholder) {
        command.push(' ');
        command.push_str(&audio);
    }
    command
}

fn write_temp_audio(audio: &[u8], mime_type: &str) -> Result<PathBuf, String> {
    let extension = audio_extension_for_mime(mime_type);
    let path = env::temp_dir().join(format!("aspect-stt-{}.{}", Uuid::new_v4(), extension));
    std::fs::write(&path, audio)
        .map_err(|error| format!("Failed to write recorded audio: {error}"))?;
    Ok(path)
}

fn audio_extension_for_mime(mime_type: &str) -> &'static str {
    if mime_type.contains("wav") {
        "wav"
    } else if mime_type.contains("ogg") {
        "ogg"
    } else if mime_type.contains("mp4") || mime_type.contains("m4a") {
        "m4a"
    } else {
        "webm"
    }
}

fn local_stt_shell_command(command_line: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let mut command = tokio::process::Command::new("cmd");
        command.arg("/C").arg(command_line);
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(windows))]
    {
        let mut command = tokio::process::Command::new("sh");
        command.arg("-c").arg(command_line);
        // Own process group so a timeout can group-kill the shell + STT binary.
        command.process_group(0);
        command
    }
}

fn shell_quote(value: &str) -> String {
    #[cfg(windows)]
    {
        format!("\"{}\"", value.replace('"', "\\\""))
    }
    #[cfg(not(windows))]
    {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn non_empty_string(value: impl AsRef<str>) -> Option<String> {
    let value = value.as_ref();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name).and_then(|value| {
        if value.to_string_lossy().trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(value))
        }
    })
}

fn first_command_token(command: &str) -> Option<String> {
    let trimmed = command.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    let mut chars = trimmed.chars();
    let first = chars.next()?;
    if first == '"' || first == '\'' {
        let end = trimmed[1..].find(first)? + 1;
        return Some(trimmed[1..end].to_string());
    }
    Some(trimmed.split_whitespace().next()?.to_string())
}

fn command_token_available(token: &str) -> bool {
    let path = Path::new(token);
    if path.is_absolute() || token.contains('/') || token.contains('\\') {
        return executable_candidate_exists(path);
    }

    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|directory| executable_candidate_exists(&directory.join(token)))
}

fn executable_candidate_exists(path: &Path) -> bool {
    if path.is_file() {
        return true;
    }
    #[cfg(windows)]
    {
        if path.extension().is_none() {
            let extensions = env::var_os("PATHEXT").map_or_else(
                || ".COM;.EXE;.BAT;.CMD".to_string(),
                |value| value.to_string_lossy().to_string(),
            );
            return extensions
                .split(';')
                .map(|extension| extension.trim().trim_start_matches('.'))
                .filter(|extension| !extension.is_empty())
                .any(|extension| path.with_extension(extension).is_file());
        }
    }
    false
}

