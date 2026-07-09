use aspect_core::AppResult;
use serde_json::Value;
use uuid::Uuid;

use crate::protocol::{parse_thread_info, DapEvent};

use super::{
    stopped_event_thread_id, DebugLifecycleRequest, DebugSessionManager, DebugSessionUpdate,
    ThreadState,
};

impl DebugSessionManager {
    #[allow(clippy::too_many_lines)]
    pub(super) async fn apply_event(
        &mut self,
        session_id: Uuid,
        event: DapEvent,
    ) -> AppResult<()> {
        if event.event.as_str() == "output" {
            let text = event
                .body
                .as_ref()
                .and_then(|b| b.get("output"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if !text.is_empty() {
                self.update_tx
                    .send(DebugSessionUpdate::Output {
                        session_id,
                        text: text.to_string(),
                    })
                    .ok();
            }
            return Ok(());
        }

        let request = self.with_session_mut(session_id, |session| {
            session.info.last_event = Some(event.event.clone());
            match event.event.as_str() {
                "initialized" => {
                    let paths = session
                        .breakpoints_by_path
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>();
                    Some(DebugLifecycleRequest::ConfigureBreakpoints(paths))
                }
                "stopped" => {
                    session.info.status = aspect_core::DebugSessionStatus::Paused;
                    if let Some(thread_id) = stopped_event_thread_id(event.body.as_ref()) {
                        session.info.active_thread_id = Some(thread_id);
                        session.thread_states.insert(thread_id, ThreadState::Paused);
                    }
                    None
                }
                "continued" => {
                    let thread_id = stopped_event_thread_id(event.body.as_ref());
                    let all_continued = event
                        .body
                        .as_ref()
                        .and_then(|b| b.get("allThreadsContinued"))
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    if all_continued || thread_id.is_none() {
                        session.thread_states.clear();
                        session.info.status = aspect_core::DebugSessionStatus::Running;
                    } else if let Some(tid) = thread_id {
                        session.thread_states.insert(tid, ThreadState::Running);
                        if !session
                            .thread_states
                            .values()
                            .any(|s| *s == ThreadState::Paused)
                        {
                            session.info.status = aspect_core::DebugSessionStatus::Running;
                        }
                    }
                    None
                }
                "thread" => {
                    if let Some(body) = event.body.as_ref() {
                        if let Some(reason) = body.get("reason").and_then(Value::as_str) {
                            match reason {
                                "started" => {
                                    if let Some(thread) = parse_thread_info(body) {
                                        session.threads.insert(thread.id, thread);
                                    }
                                }
                                "exited" => {
                                    if let Some(thread_id) =
                                        body.get("threadId").and_then(Value::as_u64)
                                    {
                                        session.threads.remove(&thread_id);
                                        session.thread_states.remove(&thread_id);
                                        if session.info.active_thread_id == Some(thread_id) {
                                            session.info.active_thread_id = None;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    None
                }
                "terminated" | "exited" => {
                    session.info.status = aspect_core::DebugSessionStatus::Stopped;
                    session.info.stopped_at.get_or_insert_with(chrono::Utc::now);
                    None
                }
                _ => None,
            }
        })?;

        if let Some(request) = request {
            match request {
                DebugLifecycleRequest::ConfigureBreakpoints(paths) => {
                    for path in paths {
                        self.send_breakpoints_for_path(session_id, &path).await?;
                    }
                    self.send_configuration_done(session_id).await?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn apply_non_initialized_event(
        &mut self,
        session_id: Uuid,
        event: &DapEvent,
    ) -> AppResult<()> {
        self.with_session_mut(session_id, |session| {
            session.info.last_event = Some(event.event.clone());
            match event.event.as_str() {
                "stopped" => {
                    session.info.status = aspect_core::DebugSessionStatus::Paused;
                    if let Some(thread_id) = stopped_event_thread_id(event.body.as_ref()) {
                        session.info.active_thread_id = Some(thread_id);
                        session.thread_states.insert(thread_id, ThreadState::Paused);
                    }
                }
                "continued" => {
                    let thread_id = stopped_event_thread_id(event.body.as_ref());
                    let all_continued = event
                        .body
                        .as_ref()
                        .and_then(|b| b.get("allThreadsContinued"))
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    if all_continued || thread_id.is_none() {
                        session.thread_states.clear();
                        session.info.status = aspect_core::DebugSessionStatus::Running;
                    } else if let Some(tid) = thread_id {
                        session.thread_states.insert(tid, ThreadState::Running);
                        if !session
                            .thread_states
                            .values()
                            .any(|s| *s == ThreadState::Paused)
                        {
                            session.info.status = aspect_core::DebugSessionStatus::Running;
                        }
                    }
                }
                "thread" => {
                    if let Some(body) = event.body.as_ref() {
                        if let Some(reason) = body.get("reason").and_then(Value::as_str) {
                            match reason {
                                "started" => {
                                    if let Some(thread) = parse_thread_info(body) {
                                        session.threads.insert(thread.id, thread);
                                    }
                                }
                                "exited" => {
                                    if let Some(thread_id) =
                                        body.get("threadId").and_then(Value::as_u64)
                                    {
                                        session.threads.remove(&thread_id);
                                        session.thread_states.remove(&thread_id);
                                        if session.info.active_thread_id == Some(thread_id) {
                                            session.info.active_thread_id = None;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                "terminated" | "exited" => {
                    session.info.status = aspect_core::DebugSessionStatus::Stopped;
                    session.info.stopped_at.get_or_insert_with(chrono::Utc::now);
                }
                _ => {}
            }
        })
    }
}
