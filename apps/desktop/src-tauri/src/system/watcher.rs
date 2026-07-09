use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use aspect_core::AspectEvent;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, State};
use tokio::{
    sync::mpsc::UnboundedReceiver,
    time::{sleep, Duration},
};

use crate::{emit_event, lock_error, SharedState};

const WATCH_DEBOUNCE_MS: u64 = 120;
const WATCH_MAX_BATCHED_PATHS: usize = 512;
const WATCH_EXCLUDED_COMPONENTS: &[&str] = &[
    ".git",
    // IDE-managed metadata (parse cache, generated visualizations, …). Mirrors how
    // the code-graph walk skips hidden dirs; without this, every cache write would
    // wake the watcher and spawn a wasted incremental pass.
    ".aspect",
    ".next",
    ".turbo",
    ".vite",
    "coverage",
    "dist",
    "node_modules",
    "target",
];

pub type WorkspaceWatcher = RecommendedWatcher;

/// `generation` is the workspace generation captured at start time. Every batch
/// this watcher dispatches is tagged with it, so once a newer workspace bumps the
/// generation, this (now-stale) watcher's late events are discarded instead of
/// merging old-workspace paths into the new index (M5).
pub fn start(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    root: PathBuf,
    generation: u64,
) -> Result<(), String> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<PathBuf>();
    let watch_root = root.clone();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let event = match result {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(%error, "workspace file watcher event failed");
                return;
            }
        };

        if !is_mutating_watch_event(event.kind) {
            return;
        }

        for path in event.paths {
            if is_publishable_watch_path(&watch_root, &path) {
                let _ = tx.send(normalize_watch_event_path(&watch_root, path));
            }
        }
    })
    .map_err(|error| error.to_string())?;

    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|error| error.to_string())?;
    *state.workspace_watcher.lock().map_err(lock_error)? = Some(watcher);

    let state_for_watcher = state.inner().clone();
    tauri::async_runtime::spawn(forward_fs_events(
        app.clone(),
        state_for_watcher,
        root,
        rx,
        generation,
    ));
    Ok(())
}

pub fn stop(state: &State<'_, SharedState>) -> Result<(), String> {
    *state.workspace_watcher.lock().map_err(lock_error)? = None;
    Ok(())
}

async fn forward_fs_events(
    app: AppHandle,
    state: Arc<crate::AppState>,
    root: PathBuf,
    mut rx: UnboundedReceiver<PathBuf>,
    generation: u64,
) {
    while let Some(first_path) = rx.recv().await {
        let mut paths = BTreeSet::new();
        let mut collapsed = false;
        push_watch_path(&root, first_path, &mut paths, &mut collapsed);

        sleep(Duration::from_millis(WATCH_DEBOUNCE_MS)).await;

        while let Ok(path) = rx.try_recv() {
            push_watch_path(&root, path, &mut paths, &mut collapsed);
        }

        if collapsed {
            paths.clear();
            paths.insert(root.clone());
        }

        for path in &paths {
            let _ = emit_event(&app, AspectEvent::FsChanged { path: path.clone() });
        }

        // Drive a single coalesced code-graph update for the whole batch, tagged
        // with THIS watcher's captured generation (not the current one) so events
        // that arrive after a workspace switch are discarded rather than merged
        // into the new workspace's index (M5).
        if collapsed {
            // The batch overflowed and individual paths were dropped — a per-file
            // update can't know what changed, so rebuild the whole index.
            crate::services::code_graph::handle_fs_collapse(&app, &state, &root, generation);
        } else {
            crate::services::code_graph::handle_fs_batch(
                &app,
                &state,
                paths.iter().cloned().collect(),
                generation,
            );
        }
    }
}

fn push_watch_path(
    root: &Path,
    path: PathBuf,
    paths: &mut BTreeSet<PathBuf>,
    collapsed: &mut bool,
) {
    if !is_publishable_watch_path(root, &path) {
        return;
    }
    if paths.len() >= WATCH_MAX_BATCHED_PATHS {
        *collapsed = true;
        return;
    }
    paths.insert(normalize_watch_event_path(root, path));
}

fn is_mutating_watch_event(kind: EventKind) -> bool {
    !kind.is_access() && !matches!(kind, EventKind::Other)
}

fn is_publishable_watch_path(root: &Path, path: &Path) -> bool {
    let path = normalize_watch_event_path(root, path.to_path_buf());
    path_is_within_root(root, &path) && !has_excluded_watch_component(root, &path)
}

fn normalize_watch_event_path(root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() || looks_like_windows_absolute_path(&path) {
        path
    } else {
        root.join(path)
    }
}

fn looks_like_windows_absolute_path(path: &Path) -> bool {
    let raw = path.to_string_lossy();
    let Some((drive, rest)) = raw.split_once(':') else {
        return false;
    };

    drive.len() == 1
        && drive
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic())
        && (rest.starts_with('/') || rest.starts_with('\\'))
}

fn path_is_within_root(root: &Path, path: &Path) -> bool {
    let normalized_root = normalize_watch_path_for_compare(root);
    let normalized_path = normalize_watch_path_for_compare(path);
    normalized_path == normalized_root
        || normalized_path.starts_with(&format!("{normalized_root}/"))
}

fn has_excluded_watch_component(root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative.components().any(|component| match component {
        Component::Normal(name) => {
            let name = name.to_string_lossy().to_ascii_lowercase();
            WATCH_EXCLUDED_COMPONENTS.contains(&name.as_str())
        }
        _ => false,
    })
}

fn normalize_watch_path_for_compare(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

