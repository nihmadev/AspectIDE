use std::{
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use base64::Engine;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const LOCAL_STT_COMMAND_ENV: &str = "LUX_STT_COMMAND";
const LOCAL_STT_MODEL_ENV: &str = "LUX_STT_MODEL";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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
    provider: String,
    audio_base64: String,
    mime_type: String,
    language: Option<String>,
    command: Option<String>,
    model_path: Option<PathBuf>,
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

    run_local_stt_command(
        &command,
        &audio,
        &request.mime_type,
        request.language.as_deref(),
        status.model_path.as_deref(),
    )
    .await
    .map(|text| VoiceTranscriptionResult { text })
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

async fn run_local_stt_command(
    command_template: &str,
    audio: &[u8],
    mime_type: &str,
    language: Option<&str>,
    model_path: Option<&Path>,
) -> Result<String, String> {
    let audio_path = write_temp_audio(audio, mime_type)?;
    let command_line = render_stt_command(
        command_template,
        &audio_path,
        mime_type,
        language,
        model_path,
    );
    let mut command = local_stt_shell_command(&command_line);
    let output_result = tokio::time::timeout(Duration::from_mins(2), command.output())
        .await
        .map_err(|_| "Local STT command timed out after 120 seconds".to_string());
    let _ = std::fs::remove_file(&audio_path);
    let output =
        output_result?.map_err(|error| format!("Local STT command failed to start: {error}"))?;

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
    let path = env::temp_dir().join(format!("lux-stt-{}.{}", Uuid::new_v4(), extension));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_stt_command_token_supports_quoted_executable() {
        assert_eq!(
            first_command_token("\"C:/Program Files/stt/whisper-cli.exe\" -m model -f {audio}")
                .as_deref(),
            Some("C:/Program Files/stt/whisper-cli.exe")
        );
        assert_eq!(
            first_command_token("whisper-cli -m {model} -f {audio}").as_deref(),
            Some("whisper-cli")
        );
        assert_eq!(first_command_token("   "), None);
    }

    #[test]
    fn local_stt_command_rendering_appends_audio_when_placeholder_is_absent() {
        let rendered = render_stt_command(
            "whisper-cli --json",
            Path::new("C:/tmp/voice.webm"),
            "audio/webm",
            Some("ru-RU"),
            None,
        );
        assert!(rendered.contains("whisper-cli --json"));
        assert!(rendered.contains("voice.webm"));

        let rendered_with_placeholders = render_stt_command(
            "whisper-cli -m {model} -f {audio} -l {language} --mime {mime}",
            Path::new("C:/tmp/voice.webm"),
            "audio/webm",
            Some("ru-RU"),
            Some(Path::new("C:/models/ggml.bin")),
        );
        assert!(rendered_with_placeholders.contains("ggml.bin"));
        assert!(rendered_with_placeholders.contains("ru-RU"));
        assert!(rendered_with_placeholders.contains("audio/webm"));
    }
}
