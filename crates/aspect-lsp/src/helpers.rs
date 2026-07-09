use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    task::{Context, Poll},
};

use aspect_core::LanguageServerInfo;
use tokio::process::Command;

use crate::types::LanguageServerDefinition;

#[cfg(windows)]
pub fn hide_process_window(command: &mut Command) {
    command.creation_flags(crate::types::CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub const fn hide_process_window(_command: &mut Command) {}

/// Build a PATH value with `dirs` (that exist) prepended ahead of the inherited
/// PATH. Returns None when there is nothing to prepend, so the child just inherits
/// the parent PATH unchanged.
pub fn prepend_path_dirs(dirs: &[PathBuf]) -> Option<std::ffi::OsString> {
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let mut parts: Vec<PathBuf> = dirs.iter().filter(|dir| dir.is_dir()).cloned().collect();
    if parts.is_empty() {
        return None;
    }
    parts.extend(std::env::split_paths(&existing));
    std::env::join_paths(parts).ok()
}

pub fn session_language_id(language_id: &str) -> &str {
    match language_id {
        "javascript" | "javascriptreact" | "typescriptreact" => "typescript",
        other => other,
    }
}

/// Drive every future in `futures` to completion CONCURRENTLY on the current task,
/// returning their outputs in input order. A dependency-free stand-in for
/// `futures::future::join_all`: each poll sweeps the not-yet-ready futures, so all
/// their await points (the per-server LSP round-trips) interleave instead of
/// running one-at-a-time. Used by `workspace_symbols` to fan out per-server
/// requests; the wakers chain through so a single combined poll wakes on any
/// child's readiness.
pub async fn join_all<F: Future>(futures: Vec<F>) -> Vec<F::Output> {
    let mut pinned: Vec<Pin<Box<F>>> = futures.into_iter().map(Box::pin).collect();
    let mut outputs: Vec<Option<F::Output>> = (0..pinned.len()).map(|_| None).collect();
    let mut pending: Vec<usize> = (0..pinned.len()).collect();

    std::future::poll_fn(move |cx: &mut Context<'_>| {
        pending.retain(|&index| match pinned[index].as_mut().poll(cx) {
            Poll::Ready(value) => {
                outputs[index] = Some(value);
                false
            }
            Poll::Pending => true,
        });
        if pending.is_empty() {
            Poll::Ready(
                outputs
                    .iter_mut()
                    .map(|slot| slot.take().unwrap())
                    .collect(),
            )
        } else {
            Poll::Pending
        }
    })
    .await
}

impl From<&LanguageServerInfo> for LanguageServerDefinition {
    fn from(server: &LanguageServerInfo) -> Self {
        Self {
            language_id: server.language_id.clone(),
            command: server.command.clone(),
            args: server.args.clone(),
            workspace_root: server.workspace_root.clone(),
            extra_path_dirs: Vec::new(),
        }
    }
}

