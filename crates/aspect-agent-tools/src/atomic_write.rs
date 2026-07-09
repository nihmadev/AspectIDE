use std::path::Path;

pub async fn ai_atomic_write(path: &Path, bytes: Vec<u8>) -> Result<(), String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || aspect_editor::atomic_write(&path, &bytes))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
