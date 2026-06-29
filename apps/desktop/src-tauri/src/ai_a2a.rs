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
/// Maximum byte length for a caller-supplied session ID string. Long IDs waste
/// memory in the hash map key and can indicate abuse; truncate silently.
const MAX_SESSION_ID_BYTES: usize = 256;
/// Minimum byte length for a session ID that is allowed to *establish* a board.
/// The blackboard's authorization boundary is the unguessability of the session
/// id (chat session ids are `crypto.randomUUID()` v4 — 122 bits of entropy, 36
/// chars). Enforcing a floor here means the backend itself rejects trivially
/// guessable / enumerable namespaces (e.g. `"default"`, `"1"`, `"chat"`) instead
/// of trusting the caller to supply an opaque token. Real ids (UUIDs ≥ 36 chars,
/// `test-<uuid>` ≥ 41 chars) comfortably clear this; short squattable ids do not.
const MIN_SESSION_ID_BYTES: usize = 16;
/// Per-session resident byte budget (approximate, based on content + topic +
/// author UTF-8 byte lengths). Prevents runaway agent loops from consuming
/// hundreds of MiB with 8 k-char entries × 500 entries × many sessions.
/// At MAX_CONTENT_CHARS=8000 + overhead ≈ 8400 bytes/entry × 500 = ~4 MiB/session.
const MAX_SESSION_BYTES: usize = 4 * 1024 * 1024; // 4 MiB
/// Global byte budget across all sessions. Evict by LRU when exceeded.
const MAX_GLOBAL_BYTES: usize = 64 * 1024 * 1024; // 64 MiB

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

/// Approximate resident byte weight for a single entry (author + topic + content
/// UTF-8 byte lengths plus a fixed per-entry struct overhead).
fn entry_bytes(entry: &BlackboardEntry) -> usize {
    const STRUCT_OVERHEAD: usize = 64; // id (uuid str) + timestamp i64 + misc
    entry.author.len() + entry.topic.len() + entry.content.len() + STRUCT_OVERHEAD
}

/// Total byte weight for all entries in one session's list.
fn session_bytes(entries: &[BlackboardEntry]) -> usize {
    entries.iter().map(entry_bytes).sum()
}

/// Total byte weight across all sessions.
fn global_bytes(board: &Board) -> usize {
    board.values().map(|entries| session_bytes(entries)).sum()
}

/// Evict the LRU (oldest last-write timestamp) session from the board.
fn evict_oldest(guard: &mut Board) {
    if let Some(victim) = oldest_session(guard) {
        guard.remove(&victim);
    }
}

/// Clamp a caller-supplied session ID to a safe length to prevent hash-map key
/// bloat and abuse. Truncation snaps down to the nearest UTF-8 char boundary so
/// multi-byte characters (CJK, emoji) straddling the byte cap never panic the
/// `str` slice below (UUIDs and typical ASCII IDs are well under the limit).
fn sanitize_session_id(session_id: &str) -> String {
    let trimmed = session_id.trim();
    if trimmed.len() <= MAX_SESSION_ID_BYTES {
        return trimmed.to_string();
    }
    // `str` slicing panics on a non-char-boundary index. Use char_indices() to
    // find the start byte of the last char that begins at or before the cap and
    // cut there, guaranteeing a valid boundary and intact UTF-8.
    let end = trimmed
        .char_indices()
        .map(|(idx, _)| idx)
        .take_while(|&idx| idx <= MAX_SESSION_ID_BYTES)
        .last()
        .unwrap_or(0);
    trimmed[..end].to_string()
}

/// Authorization guard for the blackboard: a session id may only address a board
/// if it is long enough to be an opaque, unguessable token (see
/// [`MIN_SESSION_ID_BYTES`]). The blackboard has no other server-side trust
/// boundary, so this rejects short/enumerable namespaces that a hostile or buggy
/// caller could squat to read or clear another session's agent findings.
/// `sanitized` must already have passed through [`sanitize_session_id`].
fn is_authorized_session_id(sanitized: &str) -> bool {
    sanitized.len() >= MIN_SESSION_ID_BYTES
}

/// Post a message to a session's blackboard. Returns the stored entry.
#[tauri::command]
pub fn ai_blackboard_post(
    session_id: String,
    author: String,
    topic: String,
    content: String,
) -> Result<BlackboardEntry, String> {
    // Clamp session_id to prevent hash-map key bloat from arbitrarily long strings.
    let session_id = sanitize_session_id(&session_id);
    if !is_authorized_session_id(&session_id) {
        // Reject short/guessable namespaces so the board's only trust boundary —
        // the unguessable session token — cannot be squatted (finding: A2A
        // blackboard authorization). Empty ids fail this too.
        return Err("blackboard post requires an opaque session id".to_string());
    }
    let content = clamp_chars(&content, MAX_CONTENT_CHARS);
    if content.is_empty() {
        return Err("blackboard post requires non-empty content".to_string());
    }
    let entry = BlackboardEntry {
        id: uuid::Uuid::new_v4().to_string(),
        author: {
            let a = clamp_chars(&author, MAX_AUTHOR_CHARS);
            if a.is_empty() {
                "agent".to_string()
            } else {
                a
            }
        },
        topic: {
            let t = clamp_chars(&topic, MAX_TOPIC_CHARS);
            if t.is_empty() {
                "general".to_string()
            } else {
                t
            }
        },
        content,
        timestamp_ms: Utc::now().timestamp_millis(),
    };

    let mut guard = board()
        .lock()
        .map_err(|_| "blackboard lock poisoned".to_string())?;

    // Evict LRU sessions when we hit either the session count cap or the global
    // byte budget. Per-session byte caps are enforced after insert below.
    if !guard.contains_key(&session_id) && guard.len() >= MAX_SESSIONS {
        evict_oldest(&mut guard);
    }
    while global_bytes(&guard) + entry_bytes(&entry) > MAX_GLOBAL_BYTES && !guard.is_empty() {
        evict_oldest(&mut guard);
    }

    let entries = guard.entry(session_id).or_default();
    entries.push(entry.clone());

    // Enforce per-session entry count cap (oldest entries dropped first).
    if entries.len() > MAX_ENTRIES_PER_SESSION {
        let overflow = entries.len() - MAX_ENTRIES_PER_SESSION;
        entries.drain(0..overflow);
    }
    // Enforce per-session byte budget: drop oldest entries until within budget.
    // This handles the case where a few very large entries consume the allowance.
    while session_bytes(entries) > MAX_SESSION_BYTES && entries.len() > 1 {
        entries.remove(0);
    }

    Ok(entry)
}

/// Read messages from a session's blackboard: returns the most recent `limit`
/// matching entries in chronological (oldest-first) order, so the newest is last.
/// Optional `topic` filters to a single channel; `limit` caps the result count.
#[tauri::command]
pub fn ai_blackboard_read(
    session_id: String,
    topic: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<BlackboardEntry>, String> {
    let session_id = sanitize_session_id(&session_id);
    if !is_authorized_session_id(&session_id) {
        // A guessable/short id can never read another session's findings.
        return Ok(Vec::new());
    }
    let guard = board()
        .lock()
        .map_err(|_| "blackboard lock poisoned".to_string())?;
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
        .filter(|entry| {
            topic_filter
                .as_ref()
                .is_none_or(|t| entry.topic.to_lowercase() == *t)
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
    let session_id = sanitize_session_id(&session_id);
    if !is_authorized_session_id(&session_id) {
        // A guessable/short id can never clear another session's findings.
        return Ok(());
    }
    let mut guard = board()
        .lock()
        .map_err(|_| "blackboard lock poisoned".to_string())?;
    guard.remove(&session_id);
    Ok(())
}

fn oldest_session(board: &Board) -> Option<String> {
    board
        .iter()
        .min_by_key(|(_, entries)| entries.last().map_or(0, |e| e.timestamp_ms))
        .map(|(key, _)| key.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_and_read_roundtrip() {
        let session = format!("test-{}", uuid::Uuid::new_v4());
        ai_blackboard_post(
            session.clone(),
            "explorer".into(),
            "auth".into(),
            "found login in auth.rs".into(),
        )
        .unwrap();
        ai_blackboard_post(
            session.clone(),
            "reviewer".into(),
            "auth".into(),
            "token TTL is 30s".into(),
        )
        .unwrap();
        ai_blackboard_post(
            session.clone(),
            "explorer".into(),
            "ui".into(),
            "button in App.tsx".into(),
        )
        .unwrap();

        let all = ai_blackboard_read(session.clone(), None, None).unwrap();
        assert_eq!(all.len(), 3);

        let auth = ai_blackboard_read(session.clone(), Some("auth".into()), None).unwrap();
        assert_eq!(auth.len(), 2);
        assert_eq!(auth[0].content, "found login in auth.rs");

        ai_blackboard_clear(session.clone()).unwrap();
        assert!(ai_blackboard_read(session, None, None).unwrap().is_empty());
    }

    #[test]
    fn sanitizes_multibyte_session_id_without_panic() {
        // Each 'あ' is 3 UTF-8 bytes, so a char boundary straddles the byte cap.
        // The buggy byte-level slice at MAX_SESSION_ID_BYTES would panic here.
        let long_id = "あ".repeat(MAX_SESSION_ID_BYTES); // 3 bytes each, well over cap
        let sanitized = sanitize_session_id(&long_id);
        assert!(sanitized.len() <= MAX_SESSION_ID_BYTES);
        // Truncation must land on a char boundary, leaving only whole 'あ' chars.
        assert!(!sanitized.is_empty());
        assert!(sanitized.chars().all(|c| c == 'あ'));
    }

    #[test]
    fn rejects_empty_content() {
        let session = format!("test-{}", uuid::Uuid::new_v4());
        assert!(ai_blackboard_post(session, "a".into(), "t".into(), "   ".into()).is_err());
    }

    #[test]
    fn rejects_guessable_short_session_id() {
        // Short/enumerable namespaces must not be able to establish a board, and
        // must read as empty / clear as a no-op (authorization boundary).
        for guessable in ["default", "chat", "1", "session", ""] {
            assert!(
                ai_blackboard_post(
                    guessable.into(),
                    "a".into(),
                    "t".into(),
                    "secret finding".into(),
                )
                .is_err(),
                "post must reject guessable id {guessable:?}"
            );
            assert!(ai_blackboard_read(guessable.into(), None, None)
                .unwrap()
                .is_empty());
            assert!(ai_blackboard_clear(guessable.into()).is_ok());
        }
    }

    #[test]
    fn accepts_uuid_session_id() {
        // A real chat session id (crypto.randomUUID, 36 chars) clears the floor.
        let session = uuid::Uuid::new_v4().to_string();
        assert!(session.len() >= MIN_SESSION_ID_BYTES);
        let entry = ai_blackboard_post(
            session.clone(),
            "explorer".into(),
            "auth".into(),
            "found it".into(),
        )
        .unwrap();
        assert_eq!(entry.content, "found it");
        assert_eq!(ai_blackboard_read(session, None, None).unwrap().len(), 1);
    }

    #[test]
    fn caps_entries_per_session() {
        let session = format!("test-{}", uuid::Uuid::new_v4());
        for i in 0..(MAX_ENTRIES_PER_SESSION + 25) {
            ai_blackboard_post(session.clone(), "a".into(), "t".into(), format!("msg {i}"))
                .unwrap();
        }
        let all = ai_blackboard_read(session, None, Some(MAX_ENTRIES_PER_SESSION)).unwrap();
        assert_eq!(all.len(), MAX_ENTRIES_PER_SESSION);
        // Oldest entries dropped; newest retained.
        assert!(all
            .last()
            .unwrap()
            .content
            .contains(&format!("{}", MAX_ENTRIES_PER_SESSION + 24)));
    }
}
