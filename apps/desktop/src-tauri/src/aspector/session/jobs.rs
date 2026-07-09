#![allow(dead_code)]
//! Session-scoped background jobs for the native turn loop.
//!
//! Two kinds of detached work live here:
//! - **Background subagents** (`Task` with `background: true`): the Task call
//!   returns immediately and the subagent's modelв†”tool loop runs in a spawned
//!   task; the parent collects results later with `TaskWait` (same turn or a
//!   following one).
//! - **Background shell jobs** (`Shell` with `background: true`): the command
//!   runs detached, its live output still mirrors into the "AspectIDE AI" terminal
//!   tab; the model fetches the final result with `ShellOutput`.
//!
//! Everything is bounded: at most [`MAX_TASKS_PER_SESSION`] task records and
//! [`MAX_SHELL_JOBS_PER_SESSION`] shell records per session (oldest finished
//! evicted first), and stored shell output is clamped to
//! [`MAX_SHELL_RESULT_CHARS`]. `dispose_session` drops a session's records.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum JobStatus {
    Running,
    Done,
    Failed,
}

#[allow(dead_code)]
impl JobStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BackgroundTask {
    pub agent_id: String,
    pub call_id: String,
    pub description: String,
    pub subagent_type: String,
    pub started_ms: i64,
    pub status: JobStatus,
    /// Final summary (Done) or error text (Failed); empty while running.
    pub summary: String,
    pub board_posts: usize,
    pub board_topics: Vec<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BackgroundShellJob {
    pub job_id: String,
    pub command: String,
    pub started_ms: i64,
    pub status: JobStatus,
    /// Serialized `AiShellResponse` JSON (Done) or error text (Failed); empty
    /// while running. Clamped to [`MAX_SHELL_RESULT_CHARS`].
    pub result: String,
}

#[allow(dead_code)]
const MAX_TASKS_PER_SESSION: usize = 16;
#[allow(dead_code)]
const MAX_SHELL_JOBS_PER_SESSION: usize = 8;
#[allow(dead_code)]
const MAX_SHELL_RESULT_CHARS: usize = 60_000;

type TaskMap = HashMap<String, Vec<BackgroundTask>>;
type ShellMap = HashMap<String, Vec<BackgroundShellJob>>;

fn tasks() -> &'static Mutex<TaskMap> {
    static TASKS: OnceLock<Mutex<TaskMap>> = OnceLock::new();
    TASKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn shell_jobs() -> &'static Mutex<ShellMap> {
    static JOBS: OnceLock<Mutex<ShellMap>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[allow(dead_code)]
/// Evict oldest FINISHED entries above the cap; running entries are never
/// dropped (a background job must stay addressable until it settles).
fn evict_finished<T>(entries: &mut Vec<T>, cap: usize, is_running: impl Fn(&T) -> bool) {
    while entries.len() > cap {
        let Some(index) = entries.iter().position(|entry| !is_running(entry)) else {
            return;
        };
        entries.remove(index);
    }
}

// в”Ђв”Ђ Background subagent tasks в”Ђв”Ђ

pub fn register_task(session_id: &str, task: BackgroundTask) {
    let mut map = tasks().lock().expect("ai_jobs tasks lock");
    let entries = map.entry(session_id.to_string()).or_default();
    entries.push(task);
    evict_finished(entries, MAX_TASKS_PER_SESSION, |entry| {
        entry.status == JobStatus::Running
    });
}

pub fn complete_task(
    session_id: &str,
    agent_id: &str,
    status: JobStatus,
    summary: String,
    board_posts: usize,
    board_topics: Vec<String>,
) {
    let mut map = tasks().lock().expect("ai_jobs tasks lock");
    let Some(entries) = map.get_mut(session_id) else {
        return;
    };
    if let Some(task) = entries.iter_mut().find(|task| task.agent_id == agent_id) {
        task.status = status;
        task.summary = summary;
        task.board_posts = board_posts;
        task.board_topics = board_topics;
    }
}

pub fn list_tasks(session_id: &str) -> Vec<BackgroundTask> {
    tasks()
        .lock()
        .expect("ai_jobs tasks lock")
        .get(session_id)
        .cloned()
        .unwrap_or_default()
}

// в”Ђв”Ђ Background shell jobs в”Ђв”Ђ

pub fn register_shell_job(session_id: &str, job_id: &str, command: &str) {
    let mut map = shell_jobs().lock().expect("ai_jobs shell lock");
    let entries = map.entry(session_id.to_string()).or_default();
    entries.push(BackgroundShellJob {
        job_id: job_id.to_string(),
        command: command.to_string(),
        started_ms: chrono::Utc::now().timestamp_millis(),
        status: JobStatus::Running,
        result: String::new(),
    });
    evict_finished(entries, MAX_SHELL_JOBS_PER_SESSION, |entry| {
        entry.status == JobStatus::Running
    });
}

pub fn complete_shell_job(session_id: &str, job_id: &str, status: JobStatus, result: String) {
    let mut clamped = result;
    if clamped.len() > MAX_SHELL_RESULT_CHARS {
        // Keep the tail вЂ” for long build/test logs the end carries the verdict.
        let cut = clamped.len() - MAX_SHELL_RESULT_CHARS;
        let mut boundary = cut;
        while boundary < clamped.len() && !clamped.is_char_boundary(boundary) {
            boundary += 1;
        }
        clamped = format!("[вЂ¦{cut} bytes trimmed]\n{}", &clamped[boundary..]);
    }
    let mut map = shell_jobs().lock().expect("ai_jobs shell lock");
    let Some(entries) = map.get_mut(session_id) else {
        return;
    };
    if let Some(job) = entries.iter_mut().find(|job| job.job_id == job_id) {
        job.status = status;
        job.result = clamped;
    }
}

pub fn get_shell_job(session_id: &str, job_id: &str) -> Option<BackgroundShellJob> {
    shell_jobs()
        .lock()
        .expect("ai_jobs shell lock")
        .get(session_id)
        .and_then(|entries| entries.iter().find(|job| job.job_id == job_id).cloned())
}

pub fn list_shell_jobs(session_id: &str) -> Vec<BackgroundShellJob> {
    shell_jobs()
        .lock()
        .expect("ai_jobs shell lock")
        .get(session_id)
        .cloned()
        .unwrap_or_default()
}

/// Drop every record for a deleted session (wired into `ai_session_dispose`).
pub fn dispose_session(session_id: &str) {
    tasks()
        .lock()
        .expect("ai_jobs tasks lock")
        .remove(session_id);
    shell_jobs()
        .lock()
        .expect("ai_jobs shell lock")
        .remove(session_id);
}

