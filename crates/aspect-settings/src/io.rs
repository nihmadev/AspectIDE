use std::{
    ffi::OsString,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use aspect_core::AppResult;

const WORKSPACE_SETTINGS_DIR: &str = ".aspect";
const WORKSPACE_SETTINGS_FILE: &str = "settings.json";

/// Per-process sequence making each temporary file name unique (see [`write_atomic`]).
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Returns the path for workspace-scoped settings: `<root>/.aspect/settings.json`.
#[must_use]
pub fn workspace_settings_path(root: &Path) -> PathBuf {
    root.join(WORKSPACE_SETTINGS_DIR)
        .join(WORKSPACE_SETTINGS_FILE)
}

/// Writes `contents` to `path` atomically by streaming to a sibling temporary
/// file, flushing it to disk, then renaming over the target. Same-directory
/// rename replaces the destination atomically (Windows `MoveFileEx` with
/// `MOVEFILE_REPLACE_EXISTING`), so an interrupted write can never leave the
/// target truncated.
///
/// The temp file name is unique per writer (`<path>.<pid>.<seq>.tmp`): a fixed
/// `.tmp` would let two concurrent instances writing the same settings clobber
/// each other's in-progress file and install a torn result. Each writer now owns
/// its own scratch file and the rename makes last-write-win cleanly.
pub fn write_atomic(path: &Path, contents: &[u8]) -> AppResult<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = PathBuf::from(tmp);
    {
        let mut file = File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Moves a corrupt JSON file aside to `<path>.corrupt-<unix_ts>-<pid>-<seq>` so
/// a parse failure can never trigger silent, permanent data loss: the original
/// bytes are preserved for recovery, and the caller starts from a clean default
/// that will be written to the now-vacant original path.
///
/// Returns `true` if the rename succeeded (caller may write new data to `path`),
/// `false` if it failed (caller must treat the file as read-only to avoid clobbering
/// the still-present corrupt bytes).
pub fn quarantine_corrupt(path: &Path, error: &serde_json::Error) -> bool {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();

    let mut backup = path.as_os_str().to_owned();
    backup.push(OsString::from(format!(".corrupt-{ts}-{pid}-{seq}")));
    let backup = PathBuf::from(backup);

    match fs::rename(path, &backup) {
        Ok(()) => {
            eprintln!(
                "aspect-settings: {} is corrupt ({error}); backed up to {} and reset to defaults",
                path.display(),
                backup.display()
            );
            true
        }
        Err(rename_error) => {
            eprintln!(
                "aspect-settings: {} is corrupt ({error}) and could not be backed up ({rename_error}); \
                 leaving it untouched — writes this session will go to a recovery sibling",
                path.display()
            );
            false
        }
    }
}

/// Fresh sibling path used when the original settings file is corrupt and
/// could not be quarantined. Writes land here rather than clobbering the bad file.
pub fn recovery_path(path: &Path) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut name = path.as_os_str().to_owned();
    name.push(OsString::from(format!(".recovery-{ts}-{seq}")));
    PathBuf::from(name)
}
