use std::path::PathBuf;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

use crate::resolver::MAX_IMAGE_BYTES;
use crate::types::{AgentBrowserReadImageRequest, AgentBrowserReadImageResponse};
use crate::validate::mime_type_for_path;

pub async fn read_image(
    request: AgentBrowserReadImageRequest,
) -> Result<AgentBrowserReadImageResponse, String> {
    let raw_path = request.path.trim();
    if raw_path.is_empty() {
        return Err("Image path is required.".to_string());
    }

    let path = tokio::fs::canonicalize(raw_path)
        .await
        .map_err(|error| format!("Invalid image path: {error}"))?;

    let approved_roots = {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Ok(cwd) = std::env::current_dir() {
            if let Ok(real) = std::fs::canonicalize(&cwd) {
                roots.push(real);
            } else {
                roots.push(cwd);
            }
        }
        roots.push(std::env::temp_dir());
        roots
    };

    let in_allowed_root = approved_roots.iter().any(|root| path.starts_with(root));

    if !in_allowed_root {
        return Err(format!(
            "Access denied: path '{}' is outside approved directories.",
            path.display()
        ));
    }

    if !path.exists() {
        return Err(format!("Screenshot file not found: {}", path.display()));
    }
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|error| format!("Failed to read screenshot metadata: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("Screenshot path is not a file: {}", path.display()));
    }
    if metadata.len() > MAX_IMAGE_BYTES as u64 {
        return Err(format!(
            "Screenshot exceeds maximum size ({} bytes > {} bytes)",
            metadata.len(),
            MAX_IMAGE_BYTES
        ));
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|error| format!("Failed to read screenshot: {error}"))?;
    let looks_like_image = bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || (bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP");
    if !looks_like_image {
        return Err("File is not a recognized image format.".to_string());
    }
    let byte_count = bytes.len();
    let mime_type = mime_type_for_path(&path);
    let encoded = BASE64_STANDARD.encode(bytes);
    Ok(AgentBrowserReadImageResponse {
        path: path.display().to_string(),
        data_url: format!("data:{mime_type};base64,{encoded}"),
        bytes: byte_count,
        mime_type,
    })
}
