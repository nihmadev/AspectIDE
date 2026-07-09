use std::path::Path;

pub fn normalize_screenshot_path(raw: &str, root: Option<&Path>) -> String {
    let trimmed = raw.trim();
    let mut path = std::path::PathBuf::from(trimmed);
    if path.is_relative() {
        if let Some(root) = root {
            path = root.join(path);
        }
    }
    let ends_with_separator = trimmed.ends_with('/') || trimmed.ends_with('\\');
    if ends_with_separator || path.is_dir() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        path.push(format!("screenshot-{stamp}.png"));
    }
    let has_image_extension = matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp")
    );
    if !has_image_extension {
        let mut s = path.into_os_string();
        s.push(".png");
        path = std::path::PathBuf::from(s);
    }
    dunce::simplified(&path).to_string_lossy().into_owned()
}
