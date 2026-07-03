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

#[cfg(test)]
mod tests {
    use super::load_system_font_families;

    #[test]
    fn system_font_scan_returns_sorted_unique_families() {
        let families = load_system_font_families();
        // Any desktop OS ships at least a handful of fonts; an empty list would
        // mean the scan silently broke and the pickers degrade to default-only.
        assert!(
            !families.is_empty(),
            "expected at least one system font family"
        );
        for pair in families.windows(2) {
            let left = pair[0].to_lowercase();
            let right = pair[1].to_lowercase();
            assert!(
                left < right || (left == right && pair[0] < pair[1]),
                "families must be sorted and deduplicated: {:?} !< {:?}",
                pair[0],
                pair[1]
            );
        }
        assert!(
            families.iter().all(|name| !name.starts_with('.')),
            "hidden families must be filtered"
        );
    }
}
