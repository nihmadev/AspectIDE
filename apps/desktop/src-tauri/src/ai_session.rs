//! Native session state — Stage 4 of the TS→Rust migration.
//!
//! Per-session goals and todo lists, managed entirely in Rust.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

// ── Goals ──

fn goals() -> &'static Mutex<HashMap<String, String>> {
    static GOALS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    GOALS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[tauri::command]
pub fn ai_session_goal_get(session_id: String) -> String {
    goals()
        .lock()
        .ok()
        .and_then(|g| g.get(session_id.trim()).cloned())
        .unwrap_or_default()
}

#[tauri::command]
pub fn ai_session_goal_set(session_id: String, goal: String) {
    if let Ok(mut g) = goals().lock() {
        let trimmed = goal.trim().to_string();
        if trimmed.is_empty() {
            g.remove(session_id.trim());
        } else {
            g.insert(session_id.trim().to_string(), trimmed);
        }
    }
}

pub fn get_goal(session_id: &str) -> String {
    goals()
        .lock()
        .ok()
        .and_then(|g| g.get(session_id.trim()).cloned())
        .unwrap_or_default()
}

pub fn set_goal(session_id: &str, goal: &str) {
    if let Ok(mut g) = goals().lock() {
        let trimmed = goal.trim().to_string();
        if trimmed.is_empty() {
            g.remove(session_id.trim());
        } else {
            g.insert(session_id.trim().to_string(), trimmed);
        }
    }
}

// ── Todos ──

fn todos() -> &'static Mutex<HashMap<String, Vec<SessionTodo>>> {
    static TODOS: OnceLock<Mutex<HashMap<String, Vec<SessionTodo>>>> = OnceLock::new();
    TODOS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTodo {
    pub id: String,
    pub content: String,
    pub status: String,
    pub priority: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[tauri::command]
pub fn ai_session_todos_get(session_id: String) -> Vec<SessionTodo> {
    todos()
        .lock()
        .ok()
        .and_then(|t| t.get(session_id.trim()).cloned())
        .unwrap_or_default()
}

#[tauri::command]
pub fn ai_session_todos_set(session_id: String, items: Vec<SessionTodo>) {
    if let Ok(mut t) = todos().lock() {
        if items.is_empty() {
            t.remove(session_id.trim());
        } else {
            t.insert(session_id.trim().to_string(), items);
        }
    }
}

#[cfg(test)]
pub fn get_todos(session_id: &str) -> Vec<SessionTodo> {
    todos()
        .lock()
        .ok()
        .and_then(|t| t.get(session_id.trim()).cloned())
        .unwrap_or_default()
}

pub fn set_todos(session_id: &str, items: Vec<SessionTodo>) {
    if let Ok(mut t) = todos().lock() {
        if items.is_empty() {
            t.remove(session_id.trim());
        } else {
            t.insert(session_id.trim().to_string(), items);
        }
    }
}

// ── Read-before-edit tracking ──
//
// The native turn-loop records every file a turn has inspected (`Read` /
// `InspectFile`) so edit tools (`StrReplace`, overwrite `Write`) can require the
// model to read a file before mutating it — the standard guard that stops blind
// edits against stale assumptions. Keyed per session; paths are stored in their
// resolved (canonical workspace) form so `./foo`, `foo`, and an absolute path
// all match the same entry.
//
// Each entry also captures a cheap fingerprint of the file at read time (mtime +
// size). `was_file_read` re-stats the backing file and reports the read as stale
// when the fingerprint changed, so an edit is never authorized against a file that
// was mutated *after* the model read it — by a checkpoint restore mid-turn, an
// external editor, or a concurrent process. A path read but since-changed must be
// re-read, exactly like one never read at all.

/// Cheap fingerprint of a file's on-disk state at read time. Compared on edit to
/// detect a backing file that changed since the model last saw it. `None` for a
/// field means the platform/filesystem didn't report it; a missing signal never
/// counts as a match, so the guard fails safe (treats the read as stale).
#[derive(Clone, Copy, PartialEq, Eq)]
struct ReadStamp {
    /// Modified time as (seconds, nanos) since the epoch, when available.
    mtime: Option<(i64, u32)>,
    /// File length in bytes, when available.
    len: Option<u64>,
}

impl ReadStamp {
    /// Stat `path` into a fingerprint. Returns `None` when the file can't be
    /// stat'd (deleted/inaccessible) so the caller can treat it as not-read.
    fn capture(path: &std::path::Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        let mtime = meta.modified().ok().map(|time| {
            match time.duration_since(std::time::UNIX_EPOCH) {
                Ok(delta) => (
                    i64::try_from(delta.as_secs()).unwrap_or(i64::MAX),
                    delta.subsec_nanos(),
                ),
                // Pre-epoch mtime: encode as a negative second so it still compares.
                Err(err) => (
                    -i64::try_from(err.duration().as_secs()).unwrap_or(i64::MAX),
                    err.duration().subsec_nanos(),
                ),
            }
        });
        Some(Self {
            mtime,
            len: Some(meta.len()),
        })
    }

    /// True only when both fingerprints carry the same observed mtime. A missing
    /// mtime on either side is treated as "changed" (fail safe → force a re-read)
    /// rather than silently authorizing the edit.
    fn matches(self, current: Self) -> bool {
        match (self.mtime, current.mtime) {
            (Some(a), Some(b)) => a == b && self.len == current.len,
            _ => false,
        }
    }
}

fn read_files() -> &'static Mutex<HashMap<String, HashMap<String, ReadStamp>>> {
    static READ_FILES: OnceLock<Mutex<HashMap<String, HashMap<String, ReadStamp>>>> =
        OnceLock::new();
    READ_FILES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Normalizes a path key so different spellings of the same file collapse to one
/// entry. Uses the lexical-absolute form; the caller passes the already
/// workspace-resolved path, so this only lower-cases drive/separators on Windows.
fn read_key(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Marks a file as read within a session (called after a successful Read/InspectFile),
/// fingerprinting its current on-disk state so a later edit can detect intervening
/// changes. A file that can't be stat'd is recorded with an empty fingerprint, which
/// never matches on the edit check — so the model is asked to re-read it.
pub fn mark_file_read(session_id: &str, path: &std::path::Path) {
    let stamp = ReadStamp::capture(path).unwrap_or(ReadStamp {
        mtime: None,
        len: None,
    });
    if let Ok(mut map) = read_files().lock() {
        map.entry(session_id.trim().to_string())
            .or_default()
            .insert(read_key(path), stamp);
    }
}

/// True when the file was read in this session **and** its backing file is unchanged
/// since that read (same mtime + size). A since-modified file reports `false` so the
/// edit guard forces a fresh read against current contents.
pub fn was_file_read(session_id: &str, path: &std::path::Path) -> bool {
    let recorded = read_files().lock().ok().and_then(|map| {
        map.get(session_id.trim())
            .and_then(|set| set.get(&read_key(path)).copied())
    });
    let Some(recorded) = recorded else {
        return false;
    };
    // Re-stat now; a deleted/inaccessible file (None) never matches.
    ReadStamp::capture(path).is_some_and(|current| recorded.matches(current))
}

/// Clears a session's read set.
///
/// Must be called at the start of every turn so reads from a previous turn
/// do not authorize edits against files whose on-disk state may have changed.
/// Also useful after a checkpoint restore where prior reads are stale.
pub fn clear_read_files(session_id: &str) {
    if let Ok(mut map) = read_files().lock() {
        map.remove(session_id.trim());
    }
}

// ── Lifecycle cleanup ──
//
// The goals/todos/read-files maps are process-global and keyed by session id, so
// without an explicit purge they grow for the life of the process as sessions are
// opened and closed. These clear the native state when the frontend disposes a
// chat session or closes the workspace, matching the JS-side session teardown.

/// Forget all native state for a single session.
fn clear_session(session_id: &str) {
    let key = session_id.trim();
    if let Ok(mut g) = goals().lock() {
        g.remove(key);
    }
    if let Ok(mut t) = todos().lock() {
        t.remove(key);
    }
    if let Ok(mut r) = read_files().lock() {
        r.remove(key);
    }
}

/// Forget native state for every session (workspace close / shutdown).
pub fn clear_all() {
    if let Ok(mut g) = goals().lock() {
        g.clear();
    }
    if let Ok(mut t) = todos().lock() {
        t.clear();
    }
    if let Ok(mut r) = read_files().lock() {
        r.clear();
    }
}

/// Tauri command: release a disposed chat session's native goals/todos/read set
/// plus its background-job records. Called from the frontend when a chat session
/// is deleted.
#[tauri::command]
pub fn ai_session_dispose(session_id: String) {
    clear_session(&session_id);
    crate::ai_jobs::dispose_session(&session_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_roundtrip() {
        let session = format!("test-goal-{}", uuid::Uuid::new_v4());
        assert!(get_goal(&session).is_empty());
        set_goal(&session, "Build auth module");
        assert_eq!(get_goal(&session), "Build auth module");
        set_goal(&session, "");
        assert!(get_goal(&session).is_empty());
    }

    #[test]
    fn todos_roundtrip() {
        let session = format!("test-todo-{}", uuid::Uuid::new_v4());
        assert!(get_todos(&session).is_empty());
        set_todos(
            &session,
            vec![SessionTodo {
                id: "1".into(),
                content: "Write tests".into(),
                status: "pending".into(),
                priority: "high".into(),
                notes: None,
            }],
        );
        let items = get_todos(&session);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].content, "Write tests");
        set_todos(&session, vec![]);
        assert!(get_todos(&session).is_empty());
    }

    /// Create a uniquely-named temp directory for a read-tracking test and return
    /// it so the caller can drop a real file into it (the guard now stats files).
    fn temp_dir() -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lux-read-test-{}", uuid::Uuid::new_v4().simple()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_tracking_roundtrip_and_isolation() {
        let session = format!("test-read-{}", uuid::Uuid::new_v4());
        let other = format!("test-read-other-{}", uuid::Uuid::new_v4());
        let dir = temp_dir();
        let path = dir.join("main.rs");
        std::fs::write(&path, "fn main() {}").unwrap();

        assert!(
            !was_file_read(&session, &path),
            "unread file must report false"
        );
        mark_file_read(&session, &path);
        assert!(
            was_file_read(&session, &path),
            "marked file must report true"
        );
        // Read state is per-session.
        assert!(
            !was_file_read(&other, &path),
            "read state must not leak across sessions"
        );

        clear_read_files(&session);
        assert!(
            !was_file_read(&session, &path),
            "cleared session forgets reads"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_key_normalizes_path_separators() {
        // The read key is separator-agnostic so `./foo`, `foo`, and an absolute path
        // (and back-slash vs forward-slash spellings) collapse to one entry. Tested at
        // the key level because `was_file_read` also stats the file, and a back-slash
        // path is not a real file on non-Windows hosts.
        assert_eq!(
            read_key(std::path::Path::new("C:\\work\\a.rs")),
            "C:/work/a.rs"
        );
        assert_eq!(
            read_key(std::path::Path::new("C:/work/a.rs")),
            "C:/work/a.rs"
        );
    }

    #[test]
    fn read_is_stale_when_backing_file_changes_after_read() {
        let session = format!("test-read-stale-{}", uuid::Uuid::new_v4());
        let dir = temp_dir();
        let path = dir.join("edit-me.rs");
        std::fs::write(&path, "v1").unwrap();
        mark_file_read(&session, &path);
        assert!(
            was_file_read(&session, &path),
            "freshly read file must authorize an edit"
        );

        // Simulate a checkpoint restore / external edit landing AFTER the read.
        // The new content is a different length, so the fingerprint differs even on
        // a filesystem whose mtime resolution is too coarse to advance here.
        std::fs::write(&path, "v2 — changed on disk after the read").unwrap();
        assert!(
            !was_file_read(&session, &path),
            "a file modified since the read must require a fresh read"
        );

        // Re-reading the new contents re-authorizes the edit.
        mark_file_read(&session, &path);
        assert!(
            was_file_read(&session, &path),
            "re-reading the changed file restores authorization"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleted_file_is_not_considered_read() {
        let session = format!("test-read-del-{}", uuid::Uuid::new_v4());
        let dir = temp_dir();
        let path = dir.join("gone.rs");
        std::fs::write(&path, "temp").unwrap();
        mark_file_read(&session, &path);
        std::fs::remove_file(&path).unwrap();
        assert!(
            !was_file_read(&session, &path),
            "a deleted file can never satisfy read-before-edit"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
