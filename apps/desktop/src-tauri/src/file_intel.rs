use std::{fs, path::PathBuf};

use base64::{engine::general_purpose, Engine as _};
use lux_core::{FileFormatSupport, FileInspection, FileInspectionOptions};
use serde::Serialize;
use tauri::State;

use super::{
    media_intel::{build_media_ai_context, FileMediaAiContextRequest, FileMediaAiContextResponse},
    resolve_workspace_path, SharedState,
};

/// Inline data-URL cap for IPC asset previews. Kept deliberately low: the bytes are
/// base64-encoded (≈ +33%) into a string and shipped over Tauri IPC, so an 80 MiB
/// file ballooned to ~107 MiB on the wire and could freeze both ends and starve AI
/// tooling memory. Larger assets should stream through a file/custom protocol rather
/// than be inlined; this cap bounds the inline path. See followups for streaming.
const FILE_ASSET_MAX_BYTES: u64 = 12 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAssetResponse {
    path: PathBuf,
    mime_type: String,
    data_url: String,
    size: u64,
}

#[tauri::command]
#[allow(
    clippy::unnecessary_wraps,
    reason = "Tauri commands use Result for a stable IPC error ABI"
)]
pub fn file_supported_formats() -> Result<Vec<FileFormatSupport>, String> {
    Ok(lux_file_intel::supported_formats())
}

#[tauri::command]
pub async fn file_inspect(
    state: State<'_, SharedState>,
    path: PathBuf,
    options: Option<FileInspectionOptions>,
) -> Result<FileInspection, String> {
    let path = resolve_workspace_path(&state, &path)?;
    tokio::task::spawn_blocking(move || {
        lux_file_intel::inspect_file(path, &options.unwrap_or_default())
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(String::from)
}

#[tauri::command]
pub async fn file_media_ai_context(
    state: State<'_, SharedState>,
    request: FileMediaAiContextRequest,
) -> Result<FileMediaAiContextResponse, String> {
    let path = resolve_workspace_path(&state, &request.path)?;
    tokio::task::spawn_blocking(move || {
        build_media_ai_context(FileMediaAiContextRequest { path, ..request })
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn file_asset_data(
    state: State<'_, SharedState>,
    path: PathBuf,
) -> Result<FileAssetResponse, String> {
    let path = resolve_workspace_path(&state, &path)?;
    tokio::task::spawn_blocking(move || -> Result<FileAssetResponse, String> {
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err("path is not a file".to_string());
        }
        if metadata.len() > FILE_ASSET_MAX_BYTES {
            return Err(format!(
                "file is too large for inline preview: {} bytes",
                metadata.len()
            ));
        }
        let bytes = fs::read(&path).map_err(|error| error.to_string())?;
        let mime_type = mime_type_for_path(&path);
        let encoded = general_purpose::STANDARD.encode(bytes);
        Ok(FileAssetResponse {
            path,
            data_url: format!("data:{mime_type};base64,{encoded}"),
            mime_type,
            size: metadata.len(),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Read a user-picked file from ANY absolute path into an inline data URL for a
/// chat attachment. Chosen via the native OS file dialog, so access is
/// user-authorized; unlike `file_asset_data` it is NOT restricted to the
/// workspace (the picker can select files anywhere). Bounded by the same size cap.
#[tauri::command]
pub async fn read_external_file(path: PathBuf) -> Result<FileAssetResponse, String> {
    tokio::task::spawn_blocking(move || -> Result<FileAssetResponse, String> {
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err("path is not a file".to_string());
        }
        if metadata.len() > FILE_ASSET_MAX_BYTES {
            return Err(format!(
                "file is too large to attach: {} bytes",
                metadata.len()
            ));
        }
        let bytes = fs::read(&path).map_err(|error| error.to_string())?;
        let mime_type = mime_type_for_path(&path);
        let encoded = general_purpose::STANDARD.encode(bytes);
        Ok(FileAssetResponse {
            path,
            data_url: format!("data:{mime_type};base64,{encoded}"),
            mime_type,
            size: metadata.len(),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn file_open_external(
    state: State<'_, SharedState>,
    path: PathBuf,
) -> Result<(), String> {
    let path = resolve_workspace_path(&state, &path)?;
    tokio::task::spawn_blocking(move || open_external(path))
        .await
        .map_err(|error| error.to_string())?
}

fn open_external(path: PathBuf) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        std::process::Command::new("cmd")
            .arg("/C")
            .arg("start")
            .arg("")
            .arg(path)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

fn mime_type_for_path(path: &std::path::Path) -> String {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
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
        "avi" => "video/x-msvideo",
        _ => "application/octet-stream",
    }
    .to_string()
}
