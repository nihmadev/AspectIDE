//! Agent-to-agent (A2A) coordination blackboard.
//!
//! A shared, per-chat-session message board that the main agent and every
//! subagent it spawns can post findings to and read from. Because all agents in
//! a session share the same `chatSessionId`, this gives them a durable common
//! scratch space to hand off discoveries, decisions, and partial results without
//! threading everything back through the parent's context window.
//!
//! State lives in the Rust runtime (the foundation), not the TypeScript layer,
//! so it survives across tool calls and is bounded/guarded centrally.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Hard caps to keep the board bounded regardless of agent behaviour.
const MAX_ENTRIES_PER_SESSION: usize = 500;
const MAX_SESSIONS: usize = 128;
const MAX_CONTENT_CHARS: usize = 8_000;
const MAX_TOPIC_CHARS: usize = 120;
const MAX_AUTHOR_CHARS: usize = 120;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlackboardEntry {
    pub id: String,
    pub author: String,
    pub topic: String,
    pub content: String,
    pub timestamp_ms: i64,
}

type Board = HashMap<String, Vec<BlackboardEntry>>;

fn board() -> &'static Mutex<Board> {
    static BOARD: OnceLock<Mutex<Board>> = OnceLock::new();
    BOARD.get_or_init(|| Mutex::new(HashMap::new()))
}

fn clamp_chars(value: &str, max: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let truncated: String = trimmed.chars().take(max).collect();
    format!("{truncated}…")
}

/// Post a message to a session's blackboard. Returns the stored entry.
#[tauri::command]
pub fn ai_blackboard_post(
    session_id: String,
    author: String,
    topic: String,
    content: String,
) -> Result<BlackboardEntry, String> {
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Err("blackboard post requires a session id".to_string());
    }
    let content = clamp_chars(&content, MAX_CONTENT_CHARS);
    if content.is_empty() {
        return Err("blackboard post requires non-empty content".to_string());
    }
    let entry = BlackboardEntry {
        id: uuid::Uuid::new_v4().to_string(),
        author: {
            let a = clamp_chars(&author, MAX_AUTHOR_CHARS);
            if a.is_empty() { "agent".to_string() } else { a }
        },
        topic: {
            let t = clamp_chars(&topic, MAX_TOPIC_CHARS);
            if t.is_empty() { "general".to_string() } else { t }
        },
        content,
        timestamp_ms: Utc::now().timestamp_millis(),
    };

    let mut guard = board().lock().map_err(|_| "blackboard lock poisoned".to_string())?;

    // Evict the least-recently-touched session if we hit the session cap.
    if !guard.contains_key(&session_id) && guard.len() >= MAX_SESSIONS {
        if let Some(victim) = oldest_session(&guard) {
            guard.remove(&victim);
        }
    }

    let entries = guard.entry(session_id).or_default();
    entries.push(entry.clone());
    if entries.len() > MAX_ENTRIES_PER_SESSION {
        let overflow = entries.len() - MAX_ENTRIES_PER_SESSION;
        entries.drain(0..overflow);
    }
    Ok(entry)
}

/// Read messages from a session's blackboard, newest first.
/// Optional `topic` filters to a single channel; `limit` caps the result count.
#[tauri::command]
pub fn ai_blackboard_read(
    session_id: String,
    topic: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<BlackboardEntry>, String> {
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Ok(Vec::new());
    }
    let guard = board().lock().map_err(|_| "blackboard lock poisoned".to_string())?;
    let Some(entries) = guard.get(&session_id) else {
        return Ok(Vec::new());
    };
    let topic_filter = topic
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty());
    let limit = limit.unwrap_or(50).clamp(1, MAX_ENTRIES_PER_SESSION);

    let mut selected: Vec<BlackboardEntry> = entries
        .iter()
        .rev()
        .filter(|entry| match &topic_filter {
            Some(t) => entry.topic.to_lowercase() == *t,
            None => true,
        })
        .take(limit)
        .cloned()
        .collect();
    selected.reverse();
    Ok(selected)
}

/// Clear a session's blackboard (chat clear / lifecycle reset).
#[tauri::command]
pub fn ai_blackboard_clear(session_id: String) -> Result<(), String> {
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Ok(());
    }
    let mut guard = board().lock().map_err(|_| "blackboard lock poisoned".to_string())?;
    guard.remove(&session_id);
    Ok(())
}

fn oldest_session(board: &Board) -> Option<String> {
    board
        .iter()
        .min_by_key(|(_, entries)| entries.last().map(|e| e.timestamp_ms).unwrap_or(0))
        .map(|(key, _)| key.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_and_read_roundtrip() {
        let session = format!("test-{}", uuid::Uuid::new_v4());
        ai_blackboard_post(session.clone(), "explorer".into(), "auth".into(), "found login in auth.rs".into()).unwrap();
        ai_blackboard_post(session.clone(), "reviewer".into(), "auth".into(), "token TTL is 30s".into()).unwrap();
        ai_blackboard_post(session.clone(), "explorer".into(), "ui".into(), "button in App.tsx".into()).unwrap();

        let all = ai_blackboard_read(session.clone(), None, None).unwrap();
        assert_eq!(all.len(), 3);

        let auth = ai_blackboard_read(session.clone(), Some("auth".into()), None).unwrap();
        assert_eq!(auth.len(), 2);
        assert_eq!(auth[0].content, "found login in auth.rs");

        ai_blackboard_clear(session.clone()).unwrap();
        assert!(ai_blackboard_read(session, None, None).unwrap().is_empty());
    }

    #[test]
    fn rejects_empty_content() {
        let session = format!("test-{}", uuid::Uuid::new_v4());
        assert!(ai_blackboard_post(session, "a".into(), "t".into(), "   ".into()).is_err());
    }

    #[test]
    fn caps_entries_per_session() {
        let session = format!("test-{}", uuid::Uuid::new_v4());
        for i in 0..(MAX_ENTRIES_PER_SESSION + 25) {
            ai_blackboard_post(session.clone(), "a".into(), "t".into(), format!("msg {i}")).unwrap();
        }
        let all = ai_blackboard_read(session, None, Some(MAX_ENTRIES_PER_SESSION)).unwrap();
        assert_eq!(all.len(), MAX_ENTRIES_PER_SESSION);
        // Oldest entries dropped; newest retained.
        assert!(all.last().unwrap().content.contains(&format!("{}", MAX_ENTRIES_PER_SESSION + 24)));
    }
}
