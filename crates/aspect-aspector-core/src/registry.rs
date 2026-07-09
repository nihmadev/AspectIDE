use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};

use tokio::sync::oneshot;

use crate::types::{ApprovalDecision, QuestionAnswer};

fn approval_channels() -> &'static Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>> {
    static CHANNELS: OnceLock<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>> =
        OnceLock::new();
    CHANNELS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn question_channels() -> &'static Mutex<HashMap<String, oneshot::Sender<QuestionAnswer>>> {
    static CHANNELS: OnceLock<Mutex<HashMap<String, oneshot::Sender<QuestionAnswer>>>> =
        OnceLock::new();
    CHANNELS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cancelled_turns() -> &'static Mutex<CancelRegistry> {
    static CANCELLED: OnceLock<Mutex<CancelRegistry>> = OnceLock::new();
    CANCELLED.get_or_init(|| Mutex::new(CancelRegistry::default()))
}

#[derive(Default)]
struct CancelRegistry {
    ids: HashSet<String>,
    order: VecDeque<String>,
}

pub fn mark_turn_cancelled(turn_id: &str) {
    const CAP: usize = 256;
    if let Ok(mut reg) = cancelled_turns().lock() {
        if reg.ids.insert(turn_id.to_string()) {
            reg.order.push_back(turn_id.to_string());
        }
        while reg.order.len() > CAP {
            if let Some(oldest) = reg.order.pop_front() {
                reg.ids.remove(&oldest);
            }
        }
    }
}

pub fn is_turn_cancelled(turn_id: &str) -> bool {
    cancelled_turns()
        .lock()
        .is_ok_and(|reg| reg.ids.contains(turn_id))
}

pub fn clear_turn_cancelled(turn_id: &str) {
    if let Ok(mut reg) = cancelled_turns().lock() {
        if reg.ids.remove(turn_id) {
            reg.order.retain(|id| id != turn_id);
        }
    }
}

fn cancelled_subagents() -> &'static Mutex<CancelRegistry> {
    static CANCELLED: OnceLock<Mutex<CancelRegistry>> = OnceLock::new();
    CANCELLED.get_or_init(|| Mutex::new(CancelRegistry::default()))
}

pub fn mark_subagent_cancelled(call_id: &str) {
    const CAP: usize = 256;
    if let Ok(mut reg) = cancelled_subagents().lock() {
        if reg.ids.insert(call_id.to_string()) {
            reg.order.push_back(call_id.to_string());
        }
        while reg.order.len() > CAP {
            if let Some(oldest) = reg.order.pop_front() {
                reg.ids.remove(&oldest);
            }
        }
    }
}

pub fn is_subagent_cancelled(call_id: &str) -> bool {
    cancelled_subagents()
        .lock()
        .is_ok_and(|reg| reg.ids.contains(call_id))
}

pub fn clear_subagent_cancelled(call_id: &str) {
    if let Ok(mut reg) = cancelled_subagents().lock() {
        if reg.ids.remove(call_id) {
            reg.order.retain(|id| id != call_id);
        }
    }
}

fn pending_injections() -> &'static Mutex<HashMap<String, VecDeque<String>>> {
    static PENDING: OnceLock<Mutex<HashMap<String, VecDeque<String>>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

const MAX_INJECTIONS_PER_TURN: usize = 16;

fn live_turns() -> &'static Mutex<HashSet<String>> {
    static LIVE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashSet::new()))
}

pub struct LiveTurnGuard {
    key: String,
}

impl LiveTurnGuard {
    pub fn register(session_id: &str, turn_id: &str) -> Self {
        let key = format!("{session_id}:{turn_id}");
        if let Ok(mut live) = live_turns().lock() {
            live.insert(key.clone());
        }
        Self { key }
    }
}

impl Drop for LiveTurnGuard {
    fn drop(&mut self) {
        if let Ok(mut live) = live_turns().lock() {
            live.remove(&self.key);
        }
        if let Ok(mut map) = pending_injections().lock() {
            map.remove(&self.key);
        }
    }
}

pub fn enqueue_injection(session_id: &str, turn_id: &str, text: String) {
    if text.trim().is_empty() {
        return;
    }
    let key = format!("{session_id}:{turn_id}");
    let Ok(live) = live_turns().lock() else {
        return;
    };
    if !live.contains(&key) {
        return;
    }
    if let Ok(mut map) = pending_injections().lock() {
        let queue = map.entry(key).or_default();
        if queue.len() < MAX_INJECTIONS_PER_TURN {
            queue.push_back(text);
        }
    }
}

pub fn drain_injections(session_id: &str, turn_id: &str) -> Vec<String> {
    let key = format!("{session_id}:{turn_id}");
    if let Ok(mut map) = pending_injections().lock() {
        if let Some(queue) = map.get_mut(&key) {
            let drained: Vec<String> = queue.drain(..).collect();
            if !drained.is_empty() {
                map.remove(&key);
            }
            return drained;
        }
    }
    Vec::new()
}

pub fn clear_injections(session_id: &str, turn_id: &str) {
    let key = format!("{session_id}:{turn_id}");
    if let Ok(mut map) = pending_injections().lock() {
        map.remove(&key);
        map.remove(session_id);
    }
}

pub fn register_approval(turn_id: &str, request_id: &str) -> oneshot::Receiver<ApprovalDecision> {
    let (tx, rx) = oneshot::channel();
    let key = format!("{turn_id}:{request_id}");
    if let Ok(mut map) = approval_channels().lock() {
        map.insert(key, tx);
    }
    rx
}

pub fn resolve_approval(
    turn_id: &str,
    request_id: &str,
    decision: ApprovalDecision,
) -> Result<(), String> {
    let key = format!("{turn_id}:{request_id}");
    let sender = approval_channels()
        .lock()
        .map_err(|_| "approval lock poisoned".to_string())?
        .remove(&key)
        .ok_or_else(|| format!("no pending approval for {key}"))?;
    sender
        .send(decision)
        .map_err(|_| "approval receiver dropped".to_string())
}

pub fn cancel_approvals_for_turn(turn_id: &str) {
    if let Ok(mut map) = approval_channels().lock() {
        let prefix = format!("{turn_id}:");
        let keys: Vec<String> = map
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        for key in keys {
            if let Some(sender) = map.remove(&key) {
                let _ = sender.send(ApprovalDecision::Rejected);
            }
        }
    }
}

pub fn register_question(turn_id: &str, request_id: &str) -> oneshot::Receiver<QuestionAnswer> {
    let (tx, rx) = oneshot::channel();
    let key = format!("{turn_id}:{request_id}");
    if let Ok(mut map) = question_channels().lock() {
        map.insert(key, tx);
    }
    rx
}

pub fn resolve_question(
    turn_id: &str,
    request_id: &str,
    answer: QuestionAnswer,
) -> Result<(), String> {
    let key = format!("{turn_id}:{request_id}");
    let sender = question_channels()
        .lock()
        .map_err(|_| "question lock poisoned".to_string())?
        .remove(&key)
        .ok_or_else(|| format!("no pending question for {key}"))?;
    sender
        .send(answer)
        .map_err(|_| "question receiver dropped".to_string())
}

pub fn cancel_questions_for_turn(turn_id: &str) {
    if let Ok(mut map) = question_channels().lock() {
        let prefix = format!("{turn_id}:");
        let keys: Vec<String> = map
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        for key in keys {
            if let Some(sender) = map.remove(&key) {
                let _ = sender.send(QuestionAnswer {
                    answer: String::new(),
                    cancelled: true,
                });
            }
        }
    }
}
