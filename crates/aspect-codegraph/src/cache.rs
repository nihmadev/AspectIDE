//! Persistent on-disk parse cache for the workspace code graph.
//!
//! The dominant cost of [`crate::index::Index::build`] is tree-sitter parsing
//! every source file. On a giant workspace вЂ” a monorepo of hundreds of nested
//! projects вЂ” that is a multi-second, all-cores-pegged operation, and without a
//! cache it is paid in full on *every* workspace open. This module turns that into
//! a one-time price: after a build, each file's parse output is written to disk
//! keyed by its path and a cheap `(size, mtime)` fingerprint, and the next open
//! reuses every entry whose fingerprint still matches вЂ” reparsing only the files
//! that actually changed. Warm opens become "stat everything + parse the diff"
//! instead of "read and parse everything".
//!
//! The graph itself is deliberately **not** cached. It is rebuilt from the
//! (mostly reused) per-file parses, which is linear in symbol count and cheap next
//! to parsing вЂ” and rebuilding keeps node ids, resolution, and the CSR adjacency
//! always consistent with the current file set, so there is no separate graph
//! format to keep in lock-step or to invalidate when a single file changes.
//!
//! ## Validation
//! A file is reused when its on-disk `(size, mtime)` equals the cached value вЂ”
//! the same heuristic build systems and rust-analyzer rely on. A `stat` is orders
//! of magnitude cheaper than a read+parse, and the only false-"unchanged" case (a
//! same-size edit that also preserves the mtime) is not a concern for a structural
//! source graph, which self-heals on the next real edit. A git checkout rewrites
//! mtimes, so a branch switch correctly reparses the files it touched.
//!
//! Caveat: on coarse-granularity filesystems (FAT/exFAT в‰€ 2 s, some SMB/NFS mounts)
//! a same-size edit landing within one mtime tick can be missed until a
//! size-changing edit. NTFS/APFS/ext4 (the default workspace volumes) have
//! sub-millisecond mtimes and are unaffected; a manual rebuild is the escape hatch
//! on the rare coarse volume.
//!
//! ## Safety
//! * **Version-gated** вЂ” [`CACHE_VERSION`] is bumped whenever the on-disk layout,
//!   the parse output, or a grammar changes, so a stale-format cache is ignored
//!   rather than misread into a wrong graph.
//! * **Root-gated** вЂ” the cache records the root it was built for; a mismatch is
//!   ignored (defends against a copied or relocated `.aspect`).
//! * **Corruption-tolerant** вЂ” any I/O or decode error yields `None` (в†’ a full
//!   rebuild), never a panic.
//! * **Atomic writes** вЂ” the cache is written to a sibling temp file and renamed
//!   into place, so a crash mid-write cannot leave a torn file behind.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::lang::Lang;
use crate::parse::ParsedFile;

/// On-disk cache schema version; an older cache is ignored, not misread.
///
/// **Bump on any change** to the cache layout, the [`ParsedFile`] shape, the
/// `MAX_FILE_BYTES`/admission policy, or a tree-sitter grammar *binary* вЂ” otherwise
/// a stale cache would be replayed into an out-of-date graph.
///
/// Edits to the in-repo `tags` queries do **not** need a manual bump: they are
/// folded into [`query_fingerprint`], which gates the cache automatically. Grammar
/// crate upgrades are not observable at runtime, so pin them (`=x.y.z`) and bump
/// this when they change.
pub const CACHE_VERSION: u32 = 2;

/// A fingerprint of the parse inputs that must invalidate the *whole* cache when
/// they change вЂ” currently the tree-sitter `tags` query sources, which are edited
/// in-repo to tune symbol extraction. Stable FNV-1a over every language's query, so
/// a query edit auto-invalidates the cache with no manual [`CACHE_VERSION`] bump.
/// (Hand-rolled rather than `DefaultHasher` so the value is stable across Rust
/// releases and platforms вЂ” a cache fingerprint must be reproducible.)
fn query_fingerprint() -> u64 {
    /// One FNV-1a round over `bytes`, then a length delimiter so concatenating two
    /// inputs can't alias a different split of one input.
    fn mix(mut hash: u64, bytes: &[u8]) -> u64 {
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
        hash ^= 0xff;
        hash.wrapping_mul(PRIME)
    }
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    let mut hash = OFFSET;
    for lang in Lang::ALL {
        hash = mix(hash, lang.name().as_bytes());
        hash = mix(hash, lang.tags_query().as_bytes());
    }
    hash
}

/// A cheap change-detection fingerprint: file size plus last-modified time in
/// nanoseconds since the Unix epoch. Equality means "treat as unchanged".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMeta {
    pub size: u64,
    pub mtime_ns: u64,
}

impl FileMeta {
    /// Fingerprint a path from its filesystem metadata. `None` when the file cannot
    /// be stat'd or its mtime is unavailable вЂ” such a path is neither reused nor
    /// admitted (it may have just been deleted, or sit on a filesystem without
    /// modification times).
    #[must_use]
    pub fn of(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        // A pre-epoch mtime (`duration_since` errors) is "unknown" вЂ” treat it like a
        // missing mtime and refuse to fingerprint, rather than collapsing it to `0`
        // (which would alias a genuine epoch-0 mtime and risk a false reuse).
        let mtime_ns = u64::try_from(
            metadata
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_nanos(),
        )
        .unwrap_or(u64::MAX);
        Some(Self {
            size: metadata.len(),
            mtime_ns,
        })
    }
}

/// Cached parses keyed by absolute path, the form the build's reuse pass consumes:
/// a path hit whose [`FileMeta`] still matches disk yields its [`ParsedFile`]
/// without re-reading or re-parsing the file.
pub type PriorCache = FxHashMap<PathBuf, (FileMeta, ParsedFile)>;

// в”Ђв”Ђ On-disk representation в”Ђв”Ђ
//
// Two variants of the same shape: an owned form for decoding (`load`) and a
// borrowed form for encoding (`save`) so writing the cache never clones a single
// `ParsedFile`.

#[derive(Deserialize)]
struct DiskEntry {
    path: PathBuf,
    meta: FileMeta,
    parsed: ParsedFile,
}

#[derive(Deserialize)]
struct DiskCache {
    version: u32,
    query_fp: u64,
    root: PathBuf,
    entries: Vec<DiskEntry>,
}

#[derive(Serialize)]
struct DiskEntryRef<'a> {
    path: &'a Path,
    meta: FileMeta,
    parsed: &'a ParsedFile,
}

#[derive(Serialize)]
struct DiskCacheRef<'a> {
    version: u32,
    query_fp: u64,
    root: &'a Path,
    entries: Vec<DiskEntryRef<'a>>,
}

/// Load the cache at `cache_path` for `root`, or `None` if it can't be used.
///
/// Returns the per-file parses when the file is present, current-version, built
/// with the same `tags` queries, and was built for `root`. Any absence, version
/// skew, query-fingerprint change, root mismatch, or decode error yields `None` вЂ”
/// the caller then falls back to a full build.
#[must_use]
pub fn load(cache_path: &Path, root: &Path) -> Option<PriorCache> {
    let bytes = std::fs::read(cache_path).ok()?;
    let disk: DiskCache = postcard::from_bytes(&bytes).ok()?;
    if disk.version != CACHE_VERSION || disk.query_fp != query_fingerprint() || disk.root != root {
        return None;
    }
    let mut map = PriorCache::default();
    map.reserve(disk.entries.len());
    for entry in disk.entries {
        map.insert(entry.path, (entry.meta, entry.parsed));
    }
    Some(map)
}

/// Persist `entries` as the cache for `root`, written atomically.
///
/// Each entry is `(path, fingerprint, parse)`. The bytes are encoded to a *unique*
/// sibling temp file and renamed over `cache_path` (so a crash mid-write can't
/// leave a torn file, and two writers for the same root never collide on one temp);
/// the parent directory is created if missing, and a scoped `.gitignore` is dropped
/// next to the cache so the blob is never accidentally committed.
///
/// A non-UTF-8 path can't be encoded by serde, so such entries are skipped rather
/// than failing the whole save вЂ” one oddly-named file never disables caching for
/// the entire workspace.
pub fn save<'a>(
    cache_path: &Path,
    root: &Path,
    entries: impl Iterator<Item = (&'a Path, FileMeta, &'a ParsedFile)>,
) -> std::io::Result<()> {
    let disk = DiskCacheRef {
        version: CACHE_VERSION,
        query_fp: query_fingerprint(),
        root,
        entries: entries
            .filter(|(path, _, _)| path.to_str().is_some())
            .map(|(path, meta, parsed)| DiskEntryRef { path, meta, parsed })
            .collect(),
    };
    let bytes = postcard::to_allocvec(&disk).map_err(std::io::Error::other)?;

    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)?;
        write_cache_gitignore(parent);
    }
    // A unique temp path per writer: a fixed `.tmp` would let two concurrent saves
    // for the same root (e.g. reopening the already-open workspace) clobber one
    // another's in-progress write. The rename stays on the same volume вЂ” atomic,
    // replace-existing on Windows вЂ” so each writer installs its own complete file
    // and last-write-wins cleanly.
    let mut tmp = cache_path.as_os_str().to_owned();
    tmp.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = PathBuf::from(tmp);
    // Best-effort cleanup of our own temp on any failure before the rename commits.
    if let Err(error) = std::fs::write(&tmp, &bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&tmp, cache_path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    Ok(())
}

/// Per-process sequence making each temp-file name unique (see [`save`]).
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Best-effort `.gitignore` (`*`) in the cache directory so the machine-specific,
/// absolute-path-laden cache blob can't be committed. Scoped to the cache dir only,
/// so sibling `.aspect` content (project skills/extensions) is untouched. Never
/// created if already present; failures are ignored.
fn write_cache_gitignore(cache_dir: &Path) {
    let gitignore = cache_dir.join(".gitignore");
    if !gitignore.exists() {
        let _ = std::fs::write(&gitignore, "# Machine-local code-graph cache.\n*\n");
    }
}

