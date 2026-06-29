use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::voice_input::{transcribe_local_blocking, VoiceTranscriptionRequest};

const MEDIA_TRANSCRIPT_MAX_BYTES: u64 = 48 * 1024 * 1024;
const FRAME_MAX_BYTES: u64 = 6 * 1024 * 1024;
/// Hard timeout for a single ffmpeg or ffprobe subprocess. Corrupt/huge/network-
/// backed files can stall indefinitely without this guard.
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest-edge cap applied to extracted video frames in ffmpeg. This prevents
/// 4K/8K source videos from injecting multi-megapixel frames into the AI context.
const FRAME_MAX_DIMENSION: u32 = 1280;
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

/// Run a subprocess with a wall-clock timeout. Spawns the child, then polls
/// every 100 ms until the deadline. On expiry the child is killed and an error
/// is returned so the caller can surface a bounded note rather than hanging.
fn run_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<std::process::ExitStatus, String> {
    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to spawn process: {e}"))?;
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Err(format!("process timed out after {}s", timeout.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Probe video duration in seconds via ffprobe with a timeout.
/// Returns `None` if ffprobe is unavailable or duration is indeterminate;
/// callers fall back to sequential near-beginning frame sampling.
fn probe_duration_secs(path: &Path, ffmpeg: &str) -> Option<f64> {
    // Derive ffprobe path: prefer the sibling binary next to the resolved ffmpeg.
    let ffprobe = {
        let p = Path::new(ffmpeg);
        if p.is_file() {
            p.parent()
                .and_then(|parent| {
                    let sibling = parent.join("ffprobe");
                    sibling
                        .is_file()
                        .then(|| sibling.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "ffprobe".to_string())
        } else {
            "ffprobe".to_string()
        }
    };

    // Spawn with stdout piped; enforce timeout via polling.
    let mut child = ffmpeg_command(&ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            "-nostdin",
        ])
        .arg(path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + FFMPEG_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().ok()? {
            if !status.success() {
                return None;
            }
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let output = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|d| d.is_finite() && *d > 0.0)
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

    // Build evenly-spaced seek timestamps. Probe duration first; fall back to
    // sequential near-beginning sampling when probing fails. Even spacing ensures
    // long screen recordings yield representative frames rather than only the first
    // few seconds (fixing the maxFrames-as-interval sampling bug).
    let seek_times: Vec<f64> = if max_frames == 1 {
        vec![0.0]
    } else {
        match probe_duration_secs(path, &ffmpeg) {
            Some(duration) => {
                // Sample at evenly-spaced positions across [0, duration).
                (0..usize::from(max_frames))
                    .map(|i| {
                        // Frame index is always tiny (< max_frames); widening to
                        // f64 via u32 is lossless and avoids a precision-loss cast.
                        let i = f64::from(u32::try_from(i).unwrap_or(u32::MAX));
                        duration * i / (f64::from(max_frames) - 1.0).max(1.0)
                    })
                    .map(|t| t.min(duration * 0.99)) // stay within the file
                    .collect()
            }
            None => {
                // Fallback: use the original fps=1/max_frames approach (near beginning).
                vec![] // empty signals the ffmpeg vf-filter fallback below
            }
        }
    };

    let output_dir = env::temp_dir().join(format!("lux-media-frames-{}", Uuid::new_v4()));
    fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
    let _guard = DirGuard(output_dir.clone());

    // Scale filter: cap longest edge at FRAME_MAX_DIMENSION while preserving
    // aspect ratio. "scale='min(W,1280):min(H,1280):force_original_aspect_ratio=decrease'"
    // is the canonical ffmpeg approach. We also add a JPEG quality argument.
    let scale_filter = format!(
        "scale='if(gt(iw,ih),min(iw,{FRAME_MAX_DIMENSION}),-2):if(gt(iw,ih),-2,min(ih,{FRAME_MAX_DIMENSION})),\
         scale=trunc(iw/2)*2:trunc(ih/2)*2'"
    );

    let mut all_frames: Vec<PathBuf> = Vec::new();

    if seek_times.is_empty() {
        // Fallback: sequential fps-based extraction (no duration available).
        let output_pattern = output_dir.join("frame-%02d.jpg");
        let combined_filter = format!("fps=1/{},{}", u32::from(max_frames).max(1), scale_filter);
        let mut cmd = ffmpeg_command(&ffmpeg);
        cmd.arg("-nostdin")
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-y")
            .arg("-i")
            .arg(path)
            .arg("-vf")
            .arg(&combined_filter)
            .arg("-frames:v")
            .arg(max_frames.to_string())
            .arg("-q:v")
            .arg("3")
            .arg(&output_pattern);
        let status = run_with_timeout(cmd, FFMPEG_TIMEOUT)
            .map_err(|error| format!("ffmpeg timed out or failed: {error}"))?;

        if status.success() {
            let mut entries = fs::read_dir(&output_dir)
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "jpg"))
                .collect::<Vec<_>>();
            entries.sort();
            all_frames.extend(entries.into_iter().take(usize::from(max_frames)));
        } else if all_frames.is_empty() {
            return Err(format!("ffmpeg exited with {status}"));
        }
    } else {
        // Evenly-spaced mode: one ffmpeg invocation per timestamp with -ss seek.
        for (index, &seek) in seek_times.iter().enumerate() {
            let output_path = output_dir.join(format!("frame-{:02}.jpg", index + 1));
            let mut cmd = ffmpeg_command(&ffmpeg);
            cmd.arg("-nostdin")
                .arg("-hide_banner")
                .arg("-loglevel")
                .arg("error")
                .arg("-y")
                .arg("-ss")
                .arg(format!("{seek:.3}"))
                .arg("-i")
                .arg(path)
                .arg("-vf")
                .arg(&scale_filter)
                .arg("-frames:v")
                .arg("1")
                .arg("-q:v")
                .arg("3") // JPEG quality 2–5 is good; 3 balances size/quality
                .arg(&output_path);
            // Ignore per-frame errors; collect whatever succeeds.
            if run_with_timeout(cmd, FFMPEG_TIMEOUT).is_ok_and(|s| s.success())
                && output_path.exists()
            {
                all_frames.push(output_path);
            }
        }
    }

    let mut frames = Vec::new();
    for entry in &all_frames {
        let metadata = fs::metadata(entry).map_err(|e| e.to_string())?;
        if metadata.len() > FRAME_MAX_BYTES {
            continue; // skip oversized frames (should be rare after scaling)
        }
        let bytes = fs::read(entry).map_err(|e| e.to_string())?;
        frames.push(format!(
            "data:image/jpeg;base64,{}",
            general_purpose::STANDARD.encode(bytes)
        ));
    }
    Ok(frames)
}

fn ffmpeg_command(template: &str) -> Command {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
    // `mut` is only exercised by the Windows `creation_flags` call below.
    #[cfg_attr(not(windows), allow(unused_mut))]
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
