use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::voice_input::{transcribe_local_blocking, VoiceTranscriptionRequest};

const MEDIA_TRANSCRIPT_MAX_BYTES: u64 = 48 * 1024 * 1024;
const FRAME_MAX_BYTES: u64 = 6 * 1024 * 1024;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMediaAiContextRequest {
    pub path: PathBuf,
    pub stt_command: Option<String>,
    pub stt_model_path: Option<PathBuf>,
    pub language: Option<String>,
    pub max_frames: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMediaAiContextResponse {
    transcript: Option<String>,
    frame_data_urls: Vec<String>,
    notes: Vec<String>,
}

pub fn build_media_ai_context(request: FileMediaAiContextRequest) -> FileMediaAiContextResponse {
    let ext = request
        .path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut notes = Vec::new();
    let transcript = if is_audio_extension(&ext) {
        match transcribe_media_file(&request) {
            Ok(text) => Some(text),
            Err(error) => {
                notes.push(format!("Speech-to-text unavailable: {error}"));
                None
            }
        }
    } else {
        None
    };
    let frame_data_urls = if is_video_extension(&ext) {
        match extract_video_frame_data_urls(&request.path, request.max_frames.unwrap_or(3)) {
            Ok(frames) => {
                if frames.is_empty() {
                    notes.push(
                        "Video frame snapshots unavailable. Install ffmpeg or set LUX_FFMPEG_COMMAND."
                            .to_string(),
                    );
                }
                frames
            }
            Err(error) => {
                notes.push(format!("Video frame extraction failed: {error}"));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    FileMediaAiContextResponse {
        transcript,
        frame_data_urls,
        notes,
    }
}

fn is_audio_extension(ext: &str) -> bool {
    matches!(
        ext,
        "mp3" | "wav" | "flac" | "ogg" | "oga" | "m4a" | "aac" | "opus" | "wma" | "aiff" | "aif"
    )
}

fn is_video_extension(ext: &str) -> bool {
    matches!(
        ext,
        "mp4" | "m4v" | "webm" | "mov" | "mkv" | "avi" | "wmv" | "mpeg" | "mpg" | "3gp" | "ogv"
    )
}

fn transcribe_media_file(request: &FileMediaAiContextRequest) -> Result<String, String> {
    let metadata = fs::metadata(&request.path).map_err(|error| error.to_string())?;
    if metadata.len() > MEDIA_TRANSCRIPT_MAX_BYTES {
        return Err(format!(
            "Media file is too large for local transcription ({} bytes)",
            metadata.len()
        ));
    }
    let bytes = fs::read(&request.path).map_err(|error| error.to_string())?;
    let mime_type = mime_type_for_media(&request.path);
    let audio_base64 = general_purpose::STANDARD.encode(bytes);
    transcribe_local_blocking(VoiceTranscriptionRequest {
        provider: "local".to_string(),
        audio_base64,
        mime_type,
        language: request.language.clone(),
        command: request.stt_command.clone(),
        model_path: request.stt_model_path.clone(),
    })
}

fn extract_video_frame_data_urls(path: &Path, max_frames: u8) -> Result<Vec<String>, String> {
    struct DirGuard(PathBuf);
    impl Drop for DirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    let max_frames = max_frames.clamp(1, 6);
    let ffmpeg = env::var("LUX_FFMPEG_COMMAND")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "ffmpeg".to_string());
    let output_dir = env::temp_dir().join(format!("lux-media-frames-{}", Uuid::new_v4()));
    fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
    let _guard = DirGuard(output_dir.clone());
    let output_pattern = output_dir.join("frame-%02d.jpg");
    let status = ffmpeg_command(&ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(path)
        .arg("-vf")
        .arg(format!("fps=1/{}", u32::from(max_frames).max(1)))
        .arg("-frames:v")
        .arg(max_frames.to_string())
        .arg(&output_pattern)
        .status()
        .map_err(|error| format!("Failed to run ffmpeg ({ffmpeg}): {error}"))?;
    let mut frames = Vec::new();
    if status.success() {
        let mut entries = fs::read_dir(&output_dir)
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|entry| entry.extension().is_some_and(|ext| ext == "jpg"))
            .collect::<Vec<_>>();
        entries.sort();
        for entry in entries.into_iter().take(usize::from(max_frames)) {
            let metadata = fs::metadata(&entry).map_err(|error| error.to_string())?;
            if metadata.len() > FRAME_MAX_BYTES {
                continue;
            }
            let bytes = fs::read(&entry).map_err(|error| error.to_string())?;
            frames.push(format!(
                "data:image/jpeg;base64,{}",
                general_purpose::STANDARD.encode(bytes)
            ));
        }
    }
    if !status.success() && frames.is_empty() {
        return Err(format!("ffmpeg exited with {status}"));
    }
    Ok(frames)
}

fn ffmpeg_command(template: &str) -> Command {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
    let mut command = if Path::new(template).is_file() {
        Command::new(template)
    } else if template.contains(' ') {
        let mut parts = template.split_whitespace();
        let mut command = Command::new(parts.next().unwrap_or("ffmpeg"));
        command.args(parts);
        command
    } else {
        Command::new(template)
    };
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

fn mime_type_for_media(path: &Path) -> String {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "ogg" | "oga" => "audio/ogg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "opus" => "audio/opus",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        _ => "application/octet-stream",
    }
    .to_string()
}
