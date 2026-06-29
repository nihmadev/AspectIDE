#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::path::{Path, PathBuf};

use chrono::Utc;
use lux_core::{AppError, AppResult, WorkspaceId, WorkspaceInfo};
use uuid::Uuid;

pub fn open_workspace(path: impl AsRef<Path>) -> AppResult<WorkspaceInfo> {
    let root = normalize_existing_directory(path.as_ref())?;
    let name = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Workspace")
        .to_string();

    // Derive a *stable* WorkspaceId from the canonical root path so the same
    // directory always maps to the same ID across restarts and reopens. Random
    // IDs (WorkspaceId::new) break any persistent state keyed by workspace
    // identity (AI memory, settings, telemetry).
    let id = WorkspaceId(stable_id_for_path(&root));

    Ok(WorkspaceInfo {
        id,
        name,
        root,
        opened_at: Utc::now(),
    })
}

/// Derive a deterministic UUID from a canonical workspace path using FNV-1a 128-bit
/// hashing — a trivially stable algorithm with no external dependencies.
///
/// We prepend the UUID `NameSpace_URL` namespace bytes (`6ba7b811-9dad-11d1-80b4-
/// 00c04fd430c8`) so the hash domain is isolated from any accidental collision
/// with UUIDs generated elsewhere.  UUID version/variant bits are set to v5
/// (name-based) format for standards-compliance even though our hash function
/// is FNV-1a rather than SHA-1.
fn stable_id_for_path(canonical_root: &Path) -> Uuid {
    // FNV-1a 128-bit constants (http://www.isthe.com/chongo/tech/comp/fnv/).
    const FNV_OFFSET: u128 = 144_066_263_297_769_815_596_495_629_667_062_367_629;
    const FNV_PRIME: u128 = 309_485_009_821_345_068_724_781_371;

    // UUID NameSpace_URL bytes as a fixed namespace prefix to scope our hashes.
    const NAMESPACE_BYTES: [u8; 16] = [
        0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30,
        0xc8,
    ];

    // Lowercase + forward-slash normalise the path so the same physical directory
    // always produces the same ID regardless of OS case rules or separator style.
    let path_str = canonical_root
        .to_string_lossy()
        .to_lowercase()
        .replace('\\', "/");

    let mut hash = FNV_OFFSET;
    for byte in NAMESPACE_BYTES.iter().chain(path_str.as_bytes().iter()) {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    // Apply UUID version 5 and RFC 4122 variant bits:
    //   version nibble  → bits 79-76  (AND 0x…_0FFF_…, OR 0x…_5000_…)
    //   variant top 2   → bits 63-62  (AND 0x…_3FFF_…, OR 0x…_8000_…)
    let bits = (hash & 0xFFFF_FFFF_FFFF_0FFF_3FFF_FFFF_FFFF_FFFF_u128)
        | 0x0000_0000_0000_5000_8000_0000_0000_0000_u128;
    Uuid::from_u128(bits)
}

pub fn normalize_existing_directory(path: &Path) -> AppResult<PathBuf> {
    let root = path.canonicalize()?;
    if !root.is_dir() {
        return Err(AppError::InvalidPath(format!(
            "{} is not a directory",
            root.display()
        )));
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_current_directory() {
        let workspace = open_workspace(".").expect("workspace opens");
        assert!(workspace.root.is_dir());
        assert!(!workspace.name.is_empty());
    }

    #[test]
    fn workspace_id_is_stable_across_opens() {
        // The same canonical path must always produce the same WorkspaceId so
        // persistent state (AI memory, settings) keyed by ID can be reconciled.
        let w1 = open_workspace(".").expect("first open");
        let w2 = open_workspace(".").expect("second open");
        assert_eq!(
            w1.id, w2.id,
            "WorkspaceId must be deterministic for the same canonical path"
        );
    }

    #[test]
    fn different_paths_produce_different_ids() {
        use std::env;
        let cwd = env::current_dir().expect("cwd");
        let parent = cwd.parent().expect("parent dir");
        // Only run the assertion when the parent is a real accessible directory.
        if parent.is_dir() {
            let w_cwd = open_workspace(&cwd).expect("cwd open");
            let w_parent = open_workspace(parent).expect("parent open");
            assert_ne!(
                w_cwd.id, w_parent.id,
                "different canonical paths must produce different WorkspaceIds"
            );
        }
    }
}
