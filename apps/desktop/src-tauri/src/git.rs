use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use lux_core::{GitDiff, GitStatus};
use serde::Serialize;
use tauri::State;
use tokio::sync::Mutex as AsyncMutex;

use super::{lock_error, SharedState};

/// HEAD-vs-working text for one file, powering the side-by-side diff view.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileDiff {
    pub path: String,
    pub head_text: String,
    pub working_text: String,
}

/// How long a freshly computed `status`/`diff` is reused before re-running git.
///
/// The IDE fans `status`/`diff` out from several places at once — the
/// filesystem-watch refresh, the command palette, and the AI runtime
/// (checkpoints, diagnostics, secret guard) — and a single AI turn that writes
/// many files produces a storm of overlapping calls. Each `git diff` alone
/// spawns three git processes (plus Windows helper procs), so without
/// coalescing the process count explodes. This window collapses a burst into a
/// single invocation while staying well under human-perceptible staleness; it
/// sits just above the frontend's 180 ms fs-refresh debounce so back-to-back
/// refreshes share one result.
const GIT_COALESCE_TTL: Duration = Duration::from_millis(300);

#[tauri::command]
pub async fn git_status(state: State<'_, SharedState>) -> Result<GitStatus, String> {
    let root = workspace_root(&state)?;
    status_coalescer()
        .get(root.clone(), move || {
            run_blocking(move || lux_git::status(root))
        })
        .await
}

#[tauri::command]
pub async fn git_diff(state: State<'_, SharedState>) -> Result<GitDiff, String> {
    let root = workspace_root(&state)?;
    diff_coalescer()
        .get(root.clone(), move || {
            run_blocking(move || lux_git::diff(root))
        })
        .await
}

// ── Mutations (stage / unstage / discard / commit / push / pull / branches) ──
// Each runs the blocking git op off-thread, then busts the status/diff coalescer
// caches for the root so the explorer, editor tabs, status bar, and AI runtime see
// the new state immediately instead of a ≤300ms-stale snapshot. lux-git recomputes
// and returns the fresh GitStatus, which the caller pushes straight into the store.

fn invalidate_git_caches(root: &Path) {
    status_coalescer().clear(root);
    diff_coalescer().clear(root);
}

#[tauri::command]
pub async fn git_stage(
    state: State<'_, SharedState>,
    paths: Vec<String>,
) -> Result<GitStatus, String> {
    let root = workspace_root(&state)?;
    let target = root.clone();
    let status = run_blocking(move || lux_git::stage(target, &paths)).await?;
    invalidate_git_caches(&root);
    Ok(status)
}

#[tauri::command]
pub async fn git_unstage(
    state: State<'_, SharedState>,
    paths: Vec<String>,
) -> Result<GitStatus, String> {
    let root = workspace_root(&state)?;
    let target = root.clone();
    let status = run_blocking(move || lux_git::unstage(target, &paths)).await?;
    invalidate_git_caches(&root);
    Ok(status)
}

#[tauri::command]
pub async fn git_discard(
    state: State<'_, SharedState>,
    paths: Vec<String>,
) -> Result<GitStatus, String> {
    let root = workspace_root(&state)?;
    let target = root.clone();
    let status = run_blocking(move || lux_git::discard(target, &paths)).await?;
    invalidate_git_caches(&root);
    Ok(status)
}

#[tauri::command]
pub async fn git_commit(
    state: State<'_, SharedState>,
    message: String,
) -> Result<GitStatus, String> {
    let trimmed = message.trim().to_string();
    if trimmed.is_empty() {
        return Err("A commit message is required.".to_string());
    }
    let root = workspace_root(&state)?;
    let target = root.clone();
    let status = run_blocking(move || lux_git::commit(target, &trimmed)).await?;
    invalidate_git_caches(&root);
    Ok(status)
}

#[tauri::command]
pub async fn git_push(state: State<'_, SharedState>) -> Result<GitStatus, String> {
    let root = workspace_root(&state)?;
    let target = root.clone();
    let status = run_blocking(move || lux_git::push(target)).await?;
    invalidate_git_caches(&root);
    Ok(status)
}

#[tauri::command]
pub async fn git_pull(state: State<'_, SharedState>) -> Result<GitStatus, String> {
    let root = workspace_root(&state)?;
    let target = root.clone();
    let status = run_blocking(move || lux_git::pull(target)).await?;
    invalidate_git_caches(&root);
    Ok(status)
}

#[tauri::command]
pub async fn git_branches(state: State<'_, SharedState>) -> Result<Vec<String>, String> {
    let root = workspace_root(&state)?;
    run_blocking(move || lux_git::branches(root)).await
}

#[tauri::command]
pub async fn git_checkout_branch(
    state: State<'_, SharedState>,
    name: String,
) -> Result<GitStatus, String> {
    let root = workspace_root(&state)?;
    let target = root.clone();
    let status = run_blocking(move || lux_git::checkout_branch(target, &name)).await?;
    invalidate_git_caches(&root);
    Ok(status)
}

#[tauri::command]
pub async fn git_create_branch(
    state: State<'_, SharedState>,
    name: String,
) -> Result<GitStatus, String> {
    let root = workspace_root(&state)?;
    let target = root.clone();
    let status = run_blocking(move || lux_git::create_branch(target, &name)).await?;
    invalidate_git_caches(&root);
    Ok(status)
}

#[tauri::command]
pub async fn git_file_diff(
    state: State<'_, SharedState>,
    path: String,
) -> Result<GitFileDiff, String> {
    let root = workspace_root(&state)?;
    let diff_path = path.clone();
    let (head_text, working_text) =
        run_blocking(move || lux_git::file_diff(root, &diff_path)).await?;
    Ok(GitFileDiff {
        path,
        head_text,
        working_text,
    })
}

async fn run_blocking<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> lux_core::AppResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
        .map_err(String::from)
}

fn status_coalescer() -> &'static Coalescer<GitStatus> {
    static CACHE: OnceLock<Coalescer<GitStatus>> = OnceLock::new();
    CACHE.get_or_init(Coalescer::default)
}

fn diff_coalescer() -> &'static Coalescer<GitDiff> {
    static CACHE: OnceLock<Coalescer<GitDiff>> = OnceLock::new();
    CACHE.get_or_init(Coalescer::default)
}

/// Per-root single-flight cache: concurrent callers for the same workspace
/// share one in-flight computation, and a result is reused for [`GIT_COALESCE_TTL`].
struct Coalescer<T> {
    slots: std::sync::Mutex<HashMap<PathBuf, Arc<Slot<T>>>>,
}

impl<T> Default for Coalescer<T> {
    fn default() -> Self {
        Self {
            slots: std::sync::Mutex::new(HashMap::new()),
        }
    }
}

struct Slot<T> {
    /// Serializes work for one root so only a single git process runs at a time;
    /// followers re-check the cache after acquiring it.
    gate: AsyncMutex<()>,
    cached: std::sync::Mutex<Option<(Instant, T)>>,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            gate: AsyncMutex::new(()),
            cached: std::sync::Mutex::new(None),
        }
    }
}

impl<T: Clone> Coalescer<T> {
    async fn get<F, Fut>(&self, root: PathBuf, produce: F) -> Result<T, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, String>>,
    {
        let slot = self.slot(root);

        if let Some(value) = slot.fresh() {
            return Ok(value);
        }

        // Hold the gate across the work so a burst collapses into one git run:
        // the winner computes and caches, everyone else falls through to the
        // re-check below and returns the just-stored value.
        let _guard = slot.gate.lock().await;
        if let Some(value) = slot.fresh() {
            return Ok(value);
        }

        let value = produce().await?;
        *slot.cached.lock().expect("git cache poisoned") = Some((Instant::now(), value.clone()));
        Ok(value)
    }

    fn slot(&self, root: PathBuf) -> Arc<Slot<T>> {
        self.slots
            .lock()
            .expect("git cache poisoned")
            .entry(root)
            .or_default()
            .clone()
    }

    /// Drop the cached value for `root` so the next read recomputes (called after a
    /// mutation rewrites the index/worktree).
    fn clear(&self, root: &Path) {
        if let Ok(mut slots) = self.slots.lock() {
            slots.remove(root);
        }
    }
}

impl<T: Clone> Slot<T> {
    fn fresh(&self) -> Option<T> {
        self.cached
            .lock()
            .expect("git cache poisoned")
            .as_ref()
            .filter(|(at, _)| at.elapsed() < GIT_COALESCE_TTL)
            .map(|(_, value)| value.clone())
    }
}

fn workspace_root(state: &State<'_, SharedState>) -> Result<PathBuf, String> {
    state
        .workspace
        .lock()
        .map_err(lock_error)?
        .as_ref()
        .map(|workspace| workspace.root.clone())
        .ok_or_else(|| "no workspace is open".to_string())
}
