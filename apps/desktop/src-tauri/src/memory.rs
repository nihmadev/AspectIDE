//! Tauri commands for per-project durable memory.
//!
//! Each workspace gets its own `SQLite` store under `<app_config>/memory/`, named
//! by the workspace folder plus a stable hash of its absolute path so two
//! same-named projects never collide. The open store is cached on [`AppState`]
//! and transparently reopened when the active workspace changes.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lux_memory::{
    Memory, MemoryPatch, MemoryStats, MemoryStore, NewMemory, ScoredMemory, SearchOptions,
};
use tauri::{AppHandle, Manager, State};

use crate::{workspace_root, SharedState};

/// Resolve the on-disk database path for `root`'s memory store, creating the
/// parent directory. The filename pairs a readable label with a deterministic
/// hash of the absolute path for collision-free per-project isolation.
fn memory_db_path(app: &AppHandle, root: &Path) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?
        .join("memory");
    std::fs::create_dir_all(&base).map_err(|error| error.to_string())?;
    let hash = stable_path_hash(&root.to_string_lossy());
    let label: String = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(40)
        .collect();
    Ok(base.join(format!("{label}-{hash:016x}.db")))
}

/// FNV-1a 64-bit hash. Unlike `std`'s `DefaultHasher`, its output is specified and
/// stable across Rust toolchain versions — required because this value is baked
/// into a persisted db filename, so a compiler upgrade must never orphan a
/// project's existing memory by changing the path.
fn stable_path_hash(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Run `op` against the active workspace's memory store, opening (or reopening on
/// a workspace switch) the `SQLite` backend as needed.
async fn with_memory<T: Send + 'static>(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    op: impl FnOnce(&MemoryStore) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let root = workspace_root(state)?;
    let path = memory_db_path(app, &root)?;
    // Resolve/open the store under a short async lock, then clone the Arc out so the
    // blocking SQLite work runs on a spawn_blocking thread — never on the async
    // executor and never while holding the tokio mutex (matches database.rs).
    let store = {
        let mut guard = state.memory.lock().await;
        if guard
            .as_ref()
            .is_none_or(|(open_path, _)| *open_path != path)
        {
            let opened = MemoryStore::open(&path).map_err(String::from)?;
            *guard = Some((path, Arc::new(Mutex::new(opened))));
        }
        guard.as_ref().expect("memory store set above").1.clone()
    };
    tokio::task::spawn_blocking(move || {
        let store = store
            .lock()
            .map_err(|_| "memory store lock poisoned".to_string())?;
        op(&store)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn memory_create(
    app: AppHandle,
    state: State<'_, SharedState>,
    input: NewMemory,
) -> Result<Memory, String> {
    with_memory(&app, &state, move |store| {
        store.create(input).map_err(String::from)
    })
    .await
}

#[tauri::command]
pub async fn memory_search(
    app: AppHandle,
    state: State<'_, SharedState>,
    query: String,
    options: Option<SearchOptions>,
) -> Result<Vec<ScoredMemory>, String> {
    let options = options.unwrap_or_default();
    with_memory(&app, &state, move |store| {
        store.search(&query, &options).map_err(String::from)
    })
    .await
}

#[tauri::command]
pub async fn memory_get(
    app: AppHandle,
    state: State<'_, SharedState>,
    id: String,
) -> Result<Option<Memory>, String> {
    with_memory(&app, &state, move |store| {
        store.get(&id).map_err(String::from)
    })
    .await
}

#[tauri::command]
pub async fn memory_update(
    app: AppHandle,
    state: State<'_, SharedState>,
    id: String,
    patch: MemoryPatch,
) -> Result<Memory, String> {
    with_memory(&app, &state, move |store| {
        store.update(&id, patch).map_err(String::from)
    })
    .await
}

#[tauri::command]
pub async fn memory_delete(
    app: AppHandle,
    state: State<'_, SharedState>,
    id: String,
) -> Result<bool, String> {
    with_memory(&app, &state, move |store| {
        store.delete(&id).map_err(String::from)
    })
    .await
}

#[tauri::command]
pub async fn memory_list(
    app: AppHandle,
    state: State<'_, SharedState>,
    options: Option<SearchOptions>,
) -> Result<Vec<Memory>, String> {
    let options = options.unwrap_or_default();
    with_memory(&app, &state, move |store| {
        store.list(&options).map_err(String::from)
    })
    .await
}

#[tauri::command]
pub async fn memory_stats(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<MemoryStats, String> {
    with_memory(&app, &state, move |store| {
        store.stats().map_err(String::from)
    })
    .await
}

#[tauri::command]
pub async fn memory_wipe(
    app: AppHandle,
    state: State<'_, SharedState>,
    category: Option<String>,
) -> Result<usize, String> {
    with_memory(&app, &state, move |store| {
        category.map_or_else(
            || store.wipe_all().map_err(String::from),
            |category| store.wipe_category(&category).map_err(String::from),
        )
    })
    .await
}
