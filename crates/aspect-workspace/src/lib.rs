#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::path::{Path, PathBuf};

use chrono::Utc;
use aspect_core::{AppError, AppResult, WorkspaceId, WorkspaceInfo};
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
/// hashing вЂ” a trivially stable algorithm with no external dependencies.
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
    //   version nibble  в†’ bits 79-76  (AND 0xвЂ¦_0FFF_вЂ¦, OR 0xвЂ¦_5000_вЂ¦)
    //   variant top 2   в†’ bits 63-62  (AND 0xвЂ¦_3FFF_вЂ¦, OR 0xвЂ¦_8000_вЂ¦)
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

