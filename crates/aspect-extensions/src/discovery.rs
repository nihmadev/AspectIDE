// Extension discovery: scans root directories for manifests, sorts
// deterministically, and reports duplicate IDs as blocked/invalid entries.
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use aspect_core::{AppResult, ExtensionActivationBlocked, ExtensionInfo, ExtensionStatus};

use crate::{manifest::read_extension_info, MANIFEST_FILE};

pub fn discover_extensions(root: impl AsRef<Path>) -> AppResult<Vec<ExtensionInfo>> {
    discover_extensions_in_roots([root.as_ref()])
}

/// F6 fix: collect all manifests first, sort directories deterministically so
/// the first (alphabetically) occurrence wins, then report later occurrences
/// as blocked conflicts rather than silently dropping them.
pub fn discover_extensions_in_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> AppResult<Vec<ExtensionInfo>> {
    // Phase 1: collect all (extension_root, manifest_path) pairs.
    let mut raw: Vec<(PathBuf, PathBuf)> = Vec::new();
    for root in roots {
        let root = root.as_ref();
        if !root.exists() {
            continue;
        }
        let mut entries: Vec<PathBuf> = fs::read_dir(root)?
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect();
        // Sort so winner is predictable regardless of FS order.
        entries.sort();
        for extension_root in entries {
            let manifest_path = extension_root.join(MANIFEST_FILE);
            if manifest_path.exists() {
                raw.push((extension_root, manifest_path));
            }
        }
    }

    // Phase 2: read infos; track first-seen ID в†’ position.
    let mut first_seen: HashSet<String> = HashSet::new();
    let mut infos: Vec<ExtensionInfo> = Vec::new();
    let mut conflicts: Vec<ExtensionInfo> = Vec::new();

    for (extension_root, manifest_path) in raw {
        let info = read_extension_info(extension_root, manifest_path);
        if first_seen.contains(&info.id) {
            // Duplicate: mark as invalid with a conflict reason.
            let mut conflicted = info;
            conflicted.status = ExtensionStatus::Invalid;
            conflicted.error = Some(format!(
                "duplicate extension id '{}' conflicts with an earlier extension at another path",
                conflicted.id
            ));
            conflicts.push(conflicted);
        } else {
            first_seen.insert(info.id.clone());
            infos.push(info);
        }
    }

    // Merge conflicts back so callers see all entries.
    infos.extend(conflicts);
    infos.sort_by(|l, r| {
        l.name
            .to_lowercase()
            .cmp(&r.name.to_lowercase())
            .then_with(|| l.id.cmp(&r.id))
    });
    Ok(infos)
}

pub fn blocked_extension(extension: &ExtensionInfo, reason: &str) -> ExtensionActivationBlocked {
    ExtensionActivationBlocked {
        id: extension.id.clone(),
        name: extension.name.clone(),
        version: extension.version.clone(),
        root: extension.root.clone(),
        manifest_path: extension.manifest_path.clone(),
        wasm_module: extension.wasm_module.clone(),
        contribution_points: extension.contribution_points.clone(),
        commands: extension.commands.clone(),
        reason: reason.to_string(),
    }
}
