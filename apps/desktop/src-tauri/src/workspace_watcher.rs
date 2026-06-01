use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

use lux_core::LuxEvent;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, State};
use tokio::{
    sync::mpsc::UnboundedReceiver,
    time::{sleep, Duration},
};

use super::{emit_event, lock_error, SharedState};

const WATCH_DEBOUNCE_MS: u64 = 120;
const WATCH_MAX_BATCHED_PATHS: usize = 512;
const WATCH_EXCLUDED_COMPONENTS: &[&str] = &[
    ".git",
    ".next",
    ".turbo",
    ".vite",
    "coverage",
    "dist",
    "node_modules",
    "target",
];

pub type WorkspaceWatcher = RecommendedWatcher;

pub fn start(app: &AppHandle, state: &State<'_, SharedState>, root: PathBuf) -> Result<(), String> {
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

    tauri::async_runtime::spawn(forward_fs_events(app.clone(), root, rx));
    Ok(())
}

pub fn stop(state: &State<'_, SharedState>) -> Result<(), String> {
    *state.workspace_watcher.lock().map_err(lock_error)? = None;
    Ok(())
}

async fn forward_fs_events(app: AppHandle, root: PathBuf, mut rx: UnboundedReceiver<PathBuf>) {
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

        for path in paths {
            let _ = emit_event(&app, LuxEvent::FsChanged { path });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_accepts_root_and_nested_workspace_paths() {
        let root = Path::new("C:/work/project");

        assert!(is_publishable_watch_path(
            root,
            Path::new("C:/work/project")
        ));
        assert!(is_publishable_watch_path(
            root,
            Path::new("C:/work/project/src/main.rs")
        ));
    }

    #[test]
    fn watcher_rejects_sibling_paths() {
        let root = Path::new("C:/work/project");

        assert!(!is_publishable_watch_path(
            root,
            Path::new("C:/work/project-old/src/main.rs")
        ));
        assert!(!is_publishable_watch_path(
            root,
            Path::new("C:/work/project2/src/main.rs")
        ));
    }

    #[test]
    fn watcher_rejects_generated_directories() {
        let root = Path::new("C:/work/project");

        assert!(!is_publishable_watch_path(
            root,
            Path::new("C:/work/project/.git/index")
        ));
        assert!(!is_publishable_watch_path(
            root,
            Path::new("C:/work/project/node_modules/pkg/index.js")
        ));
        assert!(!is_publishable_watch_path(
            root,
            Path::new("C:/work/project/target/debug/app.exe")
        ));
    }
}
