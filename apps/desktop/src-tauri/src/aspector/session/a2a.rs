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
/// At `MAX_CONTENT_CHARS=8000` + overhead ≈ 8400 bytes/entry × 500 = ~4 MiB/session.
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
const fn entry_bytes(entry: &BlackboardEntry) -> usize {
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
const fn is_authorized_session_id(sanitized: &str) -> bool {
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
/// Optional `topic` filters to a single channel; `author` filters to one agent's
/// posts; `since_ms` is a cursor that returns only entries strictly newer than
/// the given epoch-milliseconds timestamp (agents pass the `timestampMs` of the
/// last entry they saw, so re-reads don't replay the whole board); `limit` caps
/// the result count.
#[tauri::command]
pub fn ai_blackboard_read(
    session_id: String,
    topic: Option<String>,
    limit: Option<usize>,
    author: Option<String>,
    since_ms: Option<i64>,
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
    let author_filter = author
        .map(|a| a.trim().to_lowercase())
        .filter(|a| !a.is_empty());
    let limit = limit.unwrap_or(50).clamp(1, MAX_ENTRIES_PER_SESSION);

    let mut selected: Vec<BlackboardEntry> = entries
        .iter()
        .rev()
        .filter(|entry| {
            topic_filter
                .as_ref()
                .is_none_or(|t| entry.topic.to_lowercase() == *t)
                && author_filter
                    .as_ref()
                    .is_none_or(|a| entry.author.to_lowercase() == *a)
                && since_ms.is_none_or(|cursor| entry.timestamp_ms > cursor)
        })
        .take(limit)
        .cloned()
        .collect();
    selected.reverse();
    Ok(selected)
}

/// Board activity summary for one author since a timestamp: `(post count, the
/// distinct topics they posted under)`. Used by the Task tool result so the
/// parent agent learns what a finished subagent published to the board without
/// re-reading (and re-paying for) the full entries.
///
/// The bound is INCLUSIVE (`>=`), unlike [`ai_blackboard_read`]'s strict `>`
#[allow(dead_code)]
/// cursor: `since_ms` here is the subagent's spawn time, and a post landing in
/// the same millisecond as the spawn is the subagent's own and must count.
/// (Pre-spawn collisions are impossible — the author id is minted at spawn.)
/// The read cursor is strict because callers pass the timestamp of the last
/// entry they already saw.
pub fn author_activity_since(
    session_id: &str,
    author: &str,
    since_ms: i64,
) -> (usize, Vec<String>) {
    let session_id = sanitize_session_id(session_id);
    if !is_authorized_session_id(&session_id) {
        return (0, Vec::new());
    }
    let Ok(guard) = board().lock() else {
        return (0, Vec::new());
    };
    let Some(entries) = guard.get(&session_id) else {
        return (0, Vec::new());
    };
    let mut count = 0usize;
    let mut topics: Vec<String> = Vec::new();
    for entry in entries {
        if entry.timestamp_ms >= since_ms && entry.author == author {
            count += 1;
            if !topics.contains(&entry.topic) {
                topics.push(entry.topic.clone());
            }
        }
    }
    (count, topics)
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

