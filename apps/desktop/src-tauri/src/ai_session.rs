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
    goals().lock().ok()
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
    goals().lock().ok()
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
    todos().lock().ok()
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

pub fn get_todos(session_id: &str) -> Vec<SessionTodo> {
    todos().lock().ok()
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
        set_todos(&session, vec![
            SessionTodo { id: "1".into(), content: "Write tests".into(), status: "pending".into(), priority: "high".into(), notes: None },
        ]);
        let items = get_todos(&session);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].content, "Write tests");
        set_todos(&session, vec![]);
        assert!(get_todos(&session).is_empty());
    }
}
