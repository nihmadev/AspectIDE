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

fn read_files() -> &'static Mutex<HashMap<String, std::collections::HashSet<String>>> {
    static READ_FILES: OnceLock<Mutex<HashMap<String, std::collections::HashSet<String>>>> =
        OnceLock::new();
    READ_FILES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Normalizes a path key so different spellings of the same file collapse to one
/// entry. Uses the lexical-absolute form; the caller passes the already
/// workspace-resolved path, so this only lower-cases drive/separators on Windows.
fn read_key(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Marks a file as read within a session (called after a successful Read/InspectFile).
pub fn mark_file_read(session_id: &str, path: &std::path::Path) {
    if let Ok(mut map) = read_files().lock() {
        map.entry(session_id.trim().to_string())
            .or_default()
            .insert(read_key(path));
    }
}

/// True when the file has been read in this session.
pub fn was_file_read(session_id: &str, path: &std::path::Path) -> bool {
    read_files()
        .lock()
        .ok()
        .and_then(|map| {
            map.get(session_id.trim())
                .map(|set| set.contains(&read_key(path)))
        })
        .unwrap_or(false)
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

/// Tauri command: release a disposed chat session's native goals/todos/read set.
/// Called from the frontend when a chat session is deleted.
#[tauri::command]
pub fn ai_session_dispose(session_id: String) {
    clear_session(&session_id);
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

    #[test]
    fn read_tracking_roundtrip_and_isolation() {
        let session = format!("test-read-{}", uuid::Uuid::new_v4());
        let other = format!("test-read-other-{}", uuid::Uuid::new_v4());
        let path = std::path::Path::new("/work/src/main.rs");

        assert!(
            !was_file_read(&session, path),
            "unread file must report false"
        );
        mark_file_read(&session, path);
        assert!(
            was_file_read(&session, path),
            "marked file must report true"
        );
        // Read state is per-session.
        assert!(
            !was_file_read(&other, path),
            "read state must not leak across sessions"
        );

        clear_read_files(&session);
        assert!(
            !was_file_read(&session, path),
            "cleared session forgets reads"
        );
    }

    #[test]
    fn read_tracking_normalizes_path_separators() {
        let session = format!("test-read-norm-{}", uuid::Uuid::new_v4());
        mark_file_read(&session, std::path::Path::new("C:\\work\\a.rs"));
        // The same file addressed with forward slashes resolves to the same key.
        assert!(was_file_read(
            &session,
            std::path::Path::new("C:/work/a.rs")
        ));
    }
}
