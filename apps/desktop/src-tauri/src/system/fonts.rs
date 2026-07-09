//! System font-family enumeration for the appearance settings.
//!
//! Scans the OS font directories once (per process) via `fontdb`, collects the
//! distinct family names, and caches the sorted list — the settings font pickers
//! re-query freely without re-walking the font folders.

use std::sync::OnceLock;

static FONT_FAMILIES: OnceLock<Vec<String>> = OnceLock::new();

/// Distinct system font family names, sorted case-insensitively.
///
/// The first call walks the platform font directories on a blocking thread
/// (~50-300ms on a typical Windows install); later calls return the cached list.
#[tauri::command]
pub async fn list_system_font_families() -> Result<Vec<String>, String> {
    if let Some(cached) = FONT_FAMILIES.get() {
        return Ok(cached.clone());
    }
    let families = tauri::async_runtime::spawn_blocking(load_system_font_families)
        .await
        .map_err(|error| format!("font scan failed: {error}"))?;
    Ok(FONT_FAMILIES.get_or_init(|| families).clone())
}

fn load_system_font_families() -> Vec<String> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut families: Vec<String> = db
        .faces()
        .filter_map(|face| {
            face.families
                .first()
                .map(|(name, _)| name.trim().to_string())
        })
        // Hidden/system-internal families (macOS dot-prefixed) and empty names
        // would only add noise to a user-facing picker.
        .filter(|name| !name.is_empty() && !name.starts_with('.'))
        .collect();
    families.sort_by(|left, right| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| left.cmp(right))
    });
    families.dedup();
    families
}

