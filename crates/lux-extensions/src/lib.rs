use std::{
    fs,
    path::{Path, PathBuf},
};

use lux_core::{
    AppError, AppResult, ExtensionContributionKind, ExtensionContributionPoint, ExtensionInfo,
    ExtensionManifest, ExtensionStatus,
};

const MANIFEST_FILE: &str = "lux-extension.json";

pub fn discover_extensions(root: impl AsRef<Path>) -> AppResult<Vec<ExtensionInfo>> {
    let root = root.as_ref();
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut extensions = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_dir() {
            continue;
        }

        let extension_root = entry.path();
        let manifest_path = extension_root.join(MANIFEST_FILE);
        if !manifest_path.exists() {
            continue;
        }

        extensions.push(read_extension_info(extension_root, manifest_path));
    }

    extensions.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(extensions)
}

pub fn validate_manifest(manifest: &ExtensionManifest, extension_root: &Path) -> AppResult<()> {
    if manifest.id.trim().is_empty() {
        return Err(AppError::Service("extension id cannot be empty".into()));
    }
    if manifest.name.trim().is_empty() {
        return Err(AppError::Service("extension name cannot be empty".into()));
    }
    let wasm_path = extension_root.join(&manifest.wasm_module);
    if !wasm_path.exists() {
        return Err(AppError::NotFound(format!(
            "WASM module does not exist: {}",
            wasm_path.display()
        )));
    }
    Ok(())
}

fn read_extension_info(extension_root: PathBuf, manifest_path: PathBuf) -> ExtensionInfo {
    match read_manifest(&manifest_path) {
        Ok(manifest) => match validate_manifest(&manifest, &extension_root) {
            Ok(()) => manifest_to_info(
                extension_root,
                manifest_path,
                manifest,
                ExtensionStatus::Discovered,
                None,
            ),
            Err(error) => manifest_to_info(
                extension_root,
                manifest_path,
                manifest,
                ExtensionStatus::Invalid,
                Some(error.to_string()),
            ),
        },
        Err(error) => ExtensionInfo {
            id: extension_root
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| "invalid-extension".to_string()),
            name: extension_root
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| "Invalid extension".to_string()),
            version: "0.0.0".to_string(),
            wasm_module: PathBuf::new(),
            contributes: Vec::new(),
            contribution_points: Vec::new(),
            status: ExtensionStatus::Invalid,
            error: Some(error.to_string()),
            root: extension_root,
            manifest_path,
        },
    }
}

fn read_manifest(path: &Path) -> AppResult<ExtensionManifest> {
    let contents = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&contents)?)
}

fn manifest_to_info(
    extension_root: PathBuf,
    manifest_path: PathBuf,
    manifest: ExtensionManifest,
    status: ExtensionStatus,
    error: Option<String>,
) -> ExtensionInfo {
    let contribution_points = contribution_points_for_manifest(&manifest);
    ExtensionInfo {
        id: manifest.id,
        name: manifest.name,
        version: manifest.version,
        wasm_module: extension_root.join(manifest.wasm_module),
        contributes: manifest.contributes,
        contribution_points,
        status,
        error,
        root: extension_root,
        manifest_path,
    }
}

pub fn contribution_points_for_manifest(
    manifest: &ExtensionManifest,
) -> Vec<ExtensionContributionPoint> {
    let mut points = manifest
        .contributes
        .iter()
        .filter_map(|value| contribution_point(value))
        .collect::<Vec<_>>();
    points.sort_by(|left, right| left.id.cmp(&right.id));
    points.dedup_by(|left, right| left.id == right.id);
    points
}

fn contribution_point(value: &str) -> Option<ExtensionContributionPoint> {
    let id = value.trim();
    if id.is_empty() {
        return None;
    }

    Some(ExtensionContributionPoint {
        id: id.to_string(),
        kind: contribution_kind(id),
    })
}

fn contribution_kind(id: &str) -> ExtensionContributionKind {
    match id {
        "commands" => ExtensionContributionKind::Commands,
        "themes" => ExtensionContributionKind::Themes,
        "keybindings" => ExtensionContributionKind::Keybindings,
        "languages" => ExtensionContributionKind::Languages,
        "grammars" => ExtensionContributionKind::Grammars,
        "snippets" => ExtensionContributionKind::Snippets,
        "views" => ExtensionContributionKind::Views,
        "menus" => ExtensionContributionKind::Menus,
        "settings" | "configuration" => ExtensionContributionKind::Settings,
        "debuggers" => ExtensionContributionKind::Debuggers,
        "tasks" => ExtensionContributionKind::Tasks,
        "problemMatchers" => ExtensionContributionKind::ProblemMatchers,
        _ => ExtensionContributionKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn contribution_points_are_deduplicated_and_classified() {
        let manifest = ExtensionManifest {
            id: "lux.rust".to_string(),
            name: "Rust Tools".to_string(),
            version: "0.1.0".to_string(),
            wasm_module: PathBuf::from("extension.wasm"),
            contributes: vec![
                "commands".to_string(),
                "languages".to_string(),
                "commands".to_string(),
                "custom.point".to_string(),
                " ".to_string(),
            ],
        };

        let points = contribution_points_for_manifest(&manifest);

        assert_eq!(points.len(), 3);
        assert_eq!(
            points[0],
            ExtensionContributionPoint {
                id: "commands".to_string(),
                kind: ExtensionContributionKind::Commands,
            }
        );
        assert_eq!(
            points[1],
            ExtensionContributionPoint {
                id: "custom.point".to_string(),
                kind: ExtensionContributionKind::Unknown,
            }
        );
        assert_eq!(
            points[2],
            ExtensionContributionPoint {
                id: "languages".to_string(),
                kind: ExtensionContributionKind::Languages,
            }
        );
    }

    #[test]
    fn discover_extensions_marks_valid_manifests_as_discovered_not_active() {
        let root = unique_temp_dir("lux-extension-discovery");
        let extension_root = root.join("rust-tools");
        fs::create_dir_all(&extension_root).expect("extension root should be created");
        fs::write(extension_root.join("extension.wasm"), b"\0asm")
            .expect("wasm module should be written");
        fs::write(
            extension_root.join(MANIFEST_FILE),
            r#"{
                "id": "lux.rust",
                "name": "Rust Tools",
                "version": "0.1.0",
                "wasm_module": "extension.wasm",
                "contributes": ["commands", "themes"]
            }"#,
        )
        .expect("manifest should be written");

        let extensions = discover_extensions(&root).expect("extensions should be discovered");

        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0].status, ExtensionStatus::Discovered);
        assert_eq!(extensions[0].contribution_points.len(), 2);
        assert_eq!(
            extensions[0].contribution_points[0].kind,
            ExtensionContributionKind::Commands
        );
        assert_eq!(
            extensions[0].contribution_points[1].kind,
            ExtensionContributionKind::Themes
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn discover_extensions_keeps_manifest_contributions_for_invalid_extensions() {
        let root = unique_temp_dir("lux-extension-invalid");
        let extension_root = root.join("broken-tools");
        fs::create_dir_all(&extension_root).expect("extension root should be created");
        fs::write(
            extension_root.join(MANIFEST_FILE),
            r#"{
                "id": "lux.broken",
                "name": "Broken Tools",
                "version": "0.1.0",
                "wasm_module": "missing.wasm",
                "contributes": ["debuggers"]
            }"#,
        )
        .expect("manifest should be written");

        let extensions = discover_extensions(&root).expect("extensions should be discovered");

        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0].status, ExtensionStatus::Invalid);
        assert_eq!(
            extensions[0].contribution_points[0].kind,
            ExtensionContributionKind::Debuggers
        );
        assert!(extensions[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("WASM module does not exist"));

        let _ = fs::remove_dir_all(root);
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nonce}"))
    }
}
