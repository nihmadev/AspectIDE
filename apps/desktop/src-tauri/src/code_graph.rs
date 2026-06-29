//! Managed code-graph indexing — build, incremental update, and status for the
//! [`lux_codegraph`] crate. Progress events stream on `lux://code-graph`; the
//! index lives directly in [`AppState`] as a `tokio::sync::Mutex` option so the
//! AI tools and watcher can access it without plumbing another Arc.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use super::SharedState;

const GRAPH_EVENT: &str = "lux://code-graph";

/// Location of the persistent parse cache for `root`:
/// `<root>/.lux/cache/code-graph.bin`. `.lux` is hidden, so [`lux_codegraph`]'s own
/// walk skips it — the cache never becomes a graph node or retriggers a rebuild.
///
/// Building through [`lux_codegraph::Index::build_cached`] against this path makes a
/// warm open reuse every unchanged file's parse and reparse only what changed, so a
/// workspace with thousands of nested projects pays the full tree-sitter cost once
/// and is near-instant on every subsequent open.
fn cache_path(root: &Path) -> PathBuf {
    root.join(".lux").join("cache").join("code-graph.bin")
}

/// Build the index for `root` through the persistent cache and persist the result.
///
/// Runs on the blocking pool (the caller wraps it in `spawn_blocking`): both the
/// parse and the cache write are blocking I/O. The save is best-effort — a write
/// failure is logged and swallowed, never failing the build, since the only cost is
/// that the next open reparses more.
fn build_and_cache(root: &Path) -> Result<lux_codegraph::Index, lux_codegraph::IndexError> {
    let cache = cache_path(root);
    let mut index = lux_codegraph::Index::build_cached(root, &cache)?;
    // Skip rewriting an identical cache after a fully-cached warm build — the
    // whole-cache write is the write-amplification that bites on giant workspaces.
    // (`!cache.exists()` is a defensive belt: dirty is already true when no prior
    // cache loaded, but never skip the very first write.)
    if index.is_cache_dirty() || !cache.exists() {
        match index.save_cache(&cache) {
            Ok(()) => index.mark_cache_clean(),
            Err(error) => tracing::warn!(%error, "code-graph cache save failed"),
        }
    }
    Ok(index)
}

/// Synchronous, best-effort cache flush for app exit, where we cannot `await`.
///
/// Uses non-blocking `try_lock` and plain `std::fs` I/O, so it is safe to call from
/// the Tauri run-loop callback (no tokio runtime needed for the write). A contended
/// lock or a write error just means the next open reparses this session's edits —
/// the open-time cold-build cache still covers the unchanged bulk.
pub fn flush_cache_blocking(state: &SharedState) {
    let Ok(mut guard) = state.code_graph.try_lock() else {
        return;
    };
    let Some(index) = guard.as_mut() else {
        return;
    };
    if !index.is_cache_dirty() {
        return; // on-disk cache already current
    }
    let cache = cache_path(index.root());
    match index.save_cache(&cache) {
        Ok(()) => index.mark_cache_clean(),
        Err(error) => tracing::warn!(%error, "code-graph cache flush on exit failed"),
    }
}

/// True when `generation` still matches the live workspace generation — i.e. the
/// workspace this work was started for is still the open one. Background builds
/// and incremental updates check this before committing so a stale result never
/// overwrites the current graph.
fn generation_current(state: &SharedState, generation: u64) -> bool {
    state.workspace_generation.load(Ordering::SeqCst) == generation
}

/// Per-generation single-flight guard for **full** builds.
///
/// Holds the workspace generation currently being built in
/// `state.code_graph_building_gen` (`0` = no build in flight). Two full builds for
/// the *same* workspace (e.g. the open-time background build racing the manual
/// "Rebuild" command) are deduplicated — only the first acquires. A build for a
/// *newer* generation always wins, so switching workspaces never starves the new
/// one behind a doomed older build (the older one discards itself via
/// [`generation_current`]). The slot is released on drop, but only if it still
/// belongs to this guard — a newer generation that stole it is left untouched.
struct BuildGuard {
    state: SharedState,
    generation: u64,
}

impl BuildGuard {
    /// Acquire the build slot for `generation`, or `None` if a build for this same
    /// generation is already running.
    fn acquire(state: &SharedState, generation: u64) -> Option<Self> {
        loop {
            let current = state.code_graph_building_gen.load(Ordering::Acquire);
            if current == generation {
                return None; // same workspace already building
            }
            if state
                .code_graph_building_gen
                .compare_exchange_weak(current, generation, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(Self {
                    state: state.clone(),
                    generation,
                });
            }
        }
    }
}

impl Drop for BuildGuard {
    fn drop(&mut self) {
        // Only clear the slot if it is still ours; a newer generation may have
        // taken over while we were building.
        let _ = self.state.code_graph_building_gen.compare_exchange(
            self.generation,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

/// True when a full build for exactly this workspace generation is in flight.
fn build_in_flight(state: &SharedState, generation: u64) -> bool {
    state.code_graph_building_gen.load(Ordering::Acquire) == generation
}

/// Release incremental-update ownership and discard any stashed pending paths. Used
/// on the terminal branches where a full build (which re-reads disk) supersedes the
/// incremental work, so leftover paths would be stale.
fn clear_pending_update(state: &SharedState) {
    if let Ok(mut pending) = state.code_graph_pending_paths.lock() {
        pending.clear();
    }
    state.code_graph_updating.store(false, Ordering::SeqCst);
}

// ── Progress events ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CodeGraphEvent {
    Started {
        path: String,
    },
    Progress {
        percent: u8,
        step: String,
    },
    Finished {
        success: bool,
        node_count: usize,
        edge_count: usize,
        error: Option<String>,
    },
    /// An incremental (file-watcher) update refreshed the graph in place — no
    /// full rebuild. Lets the Settings card refresh its counts after a save
    /// without flashing through the "building" state.
    Updated {
        node_count: usize,
        edge_count: usize,
    },
}

fn emit_graph(app: &AppHandle, event: &CodeGraphEvent) {
    let _ = app.emit(GRAPH_EVENT, event);
}

fn progress(app: &AppHandle, percent: u8, step: &str) {
    emit_graph(
        app,
        &CodeGraphEvent::Progress {
            percent,
            step: step.to_string(),
        },
    );
}

/// Emit a terminal failure event.
///
/// Sending this on every build error path keeps the Settings card from sticking in
/// "building" — it only leaves that state on a `Finished` event.
fn emit_finished_error(app: &AppHandle, error: &str) {
    emit_graph(
        app,
        &CodeGraphEvent::Finished {
            success: false,
            node_count: 0,
            edge_count: 0,
            error: Some(error.to_string()),
        },
    );
}

/// Shared code-graph accessor used by both commands and AI tool dispatch.
pub async fn with_index<F, T>(state: &super::SharedState, f: F) -> Result<T, String>
where
    F: FnOnce(&lux_codegraph::Index) -> Result<T, String>,
{
    let guard = state.code_graph.lock().await;
    guard.as_ref().map_or_else(
        || Err("Code graph is not built yet. Use code_graph_build first.".to_string()),
        f,
    )
}

// ── Commands ──

#[tauri::command]
pub async fn code_graph_build(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<CodeGraphSummary, String> {
    let root = super::workspace_root(&state)?;
    let generation = state.workspace_generation.load(Ordering::SeqCst);

    // Single-flight: if a build for this workspace is already running (e.g. the
    // open-time background build), don't launch a duplicate full walk — its events
    // are already driving the UI. The generation guard makes a double build safe,
    // this just avoids the wasted CPU.
    let Some(_guard) = BuildGuard::acquire(state.inner(), generation) else {
        return Err("A code graph build for this workspace is already in progress.".to_string());
    };

    emit_graph(
        &app,
        &CodeGraphEvent::Started {
            path: root.to_string_lossy().to_string(),
        },
    );
    progress(&app, 10, "Collecting and parsing source files");

    let root_clone = root.clone();
    let result = match tokio::task::spawn_blocking(move || build_and_cache(&root_clone)).await {
        Ok(Ok(index)) => index,
        // Emit a terminal failure on every error path so the Settings card never
        // gets stuck in "building" (it only leaves that state on a Finished event).
        Ok(Err(error)) => {
            let message = format!("Code graph build error: {error}");
            emit_finished_error(&app, &message);
            return Err(message);
        }
        Err(error) => {
            let message = format!("Graph build task failed: {error}");
            emit_finished_error(&app, &message);
            return Err(message);
        }
    };

    let node_count = result.graph().node_count();
    let edge_count = result.graph().edge_count();
    let file_count = result.file_count();

    // Drop the result if the workspace changed while we were building. No failure
    // event here: a newer workspace's build is already driving the card.
    if !generation_current(state.inner(), generation) {
        return Err("Workspace changed during build; discarded.".to_string());
    }
    *state.code_graph.lock().await = Some(result);

    progress(&app, 100, "Complete");
    emit_graph(
        &app,
        &CodeGraphEvent::Finished {
            success: true,
            node_count,
            edge_count,
            error: None,
        },
    );

    Ok(CodeGraphSummary {
        node_count,
        edge_count,
        file_count,
    })
}

#[tauri::command]
pub async fn code_graph_status(state: State<'_, SharedState>) -> Result<CodeGraphStatus, String> {
    let guard = state.code_graph.lock().await;
    Ok(guard
        .as_ref()
        .map_or_else(CodeGraphStatus::default, |index| CodeGraphStatus {
            ready: true,
            node_count: index.graph().node_count(),
            edge_count: index.graph().edge_count(),
            file_count: index.file_count(),
        }))
}

#[tauri::command]
pub async fn code_graph_query(
    state: State<'_, SharedState>,
    symbol: String,
) -> Result<CodeGraphQueryResult, String> {
    with_index(&state, |index| {
        let graph = index.graph();
        let Some(node_ref) = lux_codegraph::resolve_one(graph, &symbol) else {
            return Ok(CodeGraphQueryResult {
                found: false,
                ..Default::default()
            });
        };
        let node = node_ref.node;

        let neighbors = lux_codegraph::neighbors(graph, node, None)
            .into_iter()
            .map(|n| ConnectionEntry {
                name: n.node.name,
                file: n.node.file,
                line: n.node.line,
                relation: format!("{:?}", n.relation).to_lowercase(),
                direction: format!("{:?}", n.direction).to_lowercase(),
            })
            .collect();

        let explanation = lux_codegraph::explain(graph, node).map(|e| ExploreEntry {
            kind: format!("{:?}", e.kind).to_lowercase(),
            degree: e.degree,
            total_connections: e.total_connections,
            connections: e
                .connections
                .into_iter()
                .map(|n| ConnectionEntry {
                    name: n.node.name,
                    file: n.node.file,
                    line: n.node.line,
                    relation: format!("{:?}", n.relation).to_lowercase(),
                    direction: format!("{:?}", n.direction).to_lowercase(),
                })
                .collect(),
        });

        Ok(CodeGraphQueryResult {
            found: true,
            node: Some(QueryNode {
                name: node_ref.name,
                file: node_ref.file,
                line: node_ref.line,
            }),
            callers: lux_codegraph::callers(graph, node)
                .into_iter()
                .map(|r| QueryNode {
                    name: r.name,
                    file: r.file,
                    line: r.line,
                })
                .collect(),
            callees: lux_codegraph::callees(graph, node)
                .into_iter()
                .map(|r| QueryNode {
                    name: r.name,
                    file: r.file,
                    line: r.line,
                })
                .collect(),
            neighbors,
            explanation,
        })
    })
    .await
}

/// Export the current graph as a self-contained interactive `code-graph.html`
/// under `<workspace>/.lux/`, returning the absolute path written. The `.lux`
/// directory is hidden, so [`lux_codegraph`]'s own ignore policy skips it — the
/// artifact never pollutes the graph or retriggers an incremental rebuild.
#[tauri::command]
pub async fn code_graph_export_html(state: State<'_, SharedState>) -> Result<String, String> {
    let root = super::workspace_root(&state)?;
    let title = root.file_name().map_or_else(
        || "workspace".to_string(),
        |name| name.to_string_lossy().to_string(),
    );

    // Generate the HTML UNDER the lock (it needs the graph), but release the lock
    // before the filesystem write — a full visualization can be large, and holding
    // the graph mutex across `create_dir_all` + `write` blocked incremental updates,
    // status checks, and AI code-graph tools for the duration of disk I/O.
    let html = with_index(&state, |index| {
        let graph = index.graph();
        let communities = lux_codegraph::detect_communities(graph);
        Ok(lux_codegraph::to_graph_html(
            graph,
            &communities,
            &root,
            &title,
        ))
    })
    .await?;

    let dir = root.join(".lux");
    let path = dir.join("code-graph.html");
    let write_path = path.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Could not create .lux directory: {e}"))?;
        std::fs::write(&write_path, html).map_err(|e| format!("Could not write visualization: {e}"))
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(path.to_string_lossy().to_string())
}

/// Persist the current in-memory graph's parse cache, then drop it.
///
/// Called when a workspace closes or is replaced. The in-memory index carries the
/// fresh `(size, mtime)` fingerprints of every file touched incrementally during
/// the session, so saving here captures those edits on disk — the next open of that
/// workspace reuses them instead of reparsing. Persisting only at these transitions
/// (plus after a full build) keeps the cache warm without rewriting a large cache on
/// every file save. The index is taken out of the mutex first, so the save runs off
/// the async runtime holding no lock, and detached so neither close nor switch waits
/// on the write.
pub async fn persist_and_drop(state: &SharedState) {
    let Some(index) = state.code_graph.lock().await.take() else {
        return;
    };
    // Nothing changed since the last save — don't rewrite an identical cache.
    if !index.is_cache_dirty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let _ = tokio::task::spawn_blocking(move || {
            let cache = cache_path(index.root());
            if let Err(error) = index.save_cache(&cache) {
                tracing::warn!(%error, "code-graph cache save on close failed");
            }
        })
        .await;
    });
}

/// Background code-graph build triggered on workspace open. Spawns a detached
/// tokio task so the workspace loads without waiting for the index. `generation`
/// is the workspace generation at open time; the result is committed only if it
/// is still current when the build finishes (so a stale build for a closed or
/// replaced workspace is discarded).
pub fn start_build_on_workspace(
    app: AppHandle,
    state: SharedState,
    root: PathBuf,
    generation: u64,
) {
    tauri::async_runtime::spawn(async move {
        // Single-flight: a manual build (or another open-time build) for this same
        // generation may already hold the slot. If so, skip — the holder produces
        // the graph. A newer generation always wins, so switching workspaces is not
        // starved behind an older, soon-to-be-discarded build.
        let Some(_guard) = BuildGuard::acquire(&state, generation) else {
            return;
        };

        emit_graph(
            &app,
            &CodeGraphEvent::Started {
                path: root.to_string_lossy().to_string(),
            },
        );
        progress(&app, 10, "Collecting and parsing source files");

        let root_clone = root.clone();
        let result = tokio::task::spawn_blocking(move || build_and_cache(&root_clone)).await;

        match result {
            Ok(Ok(index)) => {
                let node_count = index.graph().node_count();
                let edge_count = index.graph().edge_count();
                if !generation_current(&state, generation) {
                    // Workspace changed while building — drop the result silently.
                    return;
                }
                *state.code_graph.lock().await = Some(index);
                progress(&app, 100, "Complete");
                emit_graph(
                    &app,
                    &CodeGraphEvent::Finished {
                        success: true,
                        node_count,
                        edge_count,
                        error: None,
                    },
                );
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "background code-graph build failed");
                // Suppress the failure event if the workspace changed mid-build, just
                // like the success path does — otherwise a stale build failing after a
                // switch makes the NEW workspace's graph look broken.
                if generation_current(&state, generation) {
                    emit_finished_error(&app, &format!("{e}"));
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "code-graph spawn_blocking crashed");
                if generation_current(&state, generation) {
                    emit_finished_error(&app, &format!("task panicked: {e}"));
                }
            }
        }
    });
}

/// Incremental update for a debounced **batch** of changed paths from the file
/// watcher. Re-parses all of them and rebuilds the graph **once** (no per-file
/// stampede), doing the CPU-heavy rebuild on the blocking pool so the async
/// runtime is never pinned, and holding the graph mutex only for the brief
/// take/put swap.
///
/// Concurrency: an incremental rebuild takes the index OUT of the mutex while it
/// runs. A batch arriving in that window previously saw `None` and silently
/// returned, leaving its saved files stale in the graph. Instead we coalesce — late
/// batches stash their paths in `code_graph_pending_paths` (guarded by the
/// `code_graph_updating` flag) and the in-flight rebuild drains and re-applies them
/// before finishing, so no file-watch batch is dropped.
pub fn handle_fs_batch(app: &AppHandle, state: &SharedState, paths: Vec<PathBuf>, generation: u64) {
    if paths.is_empty() {
        return;
    }
    // Fast path: a full build for this workspace is already running — it reads the
    // current disk, so it will pick these changes up. This synchronous check only
    // skips work; the real anti-clobber guarantee is the conditional restore below.
    if build_in_flight(state, generation) {
        return;
    }

    // Acquire update ownership: `swap` returns the prior value, so a `true` means an
    // incremental update is already running. In that case hand it our paths (it drains
    // `code_graph_pending_paths` before finishing) instead of racing it for the
    // currently-`None` slot and dropping the batch.
    if state.code_graph_updating.swap(true, Ordering::AcqRel) {
        if let Ok(mut pending) = state.code_graph_pending_paths.lock() {
            pending.extend(paths);
        }
        return;
    }

    let app = app.clone();
    let state = state.clone();
    tauri::async_runtime::spawn(async move {
        // We own the update flag (set by the caller above). Always clear it on the way
        // out so a future batch can start a fresh update.
        let mut batch = paths;
        loop {
            // Take the index out under a short lock so the rebuild runs off the mutex.
            let Some(mut index) = state.code_graph.lock().await.take() else {
                // Not built yet — the initial build will pick these up. Release the
                // flag and stash nothing; a full build re-reads disk anyway.
                state.code_graph_updating.store(false, Ordering::Release);
                return;
            };
            let batch_paths = std::mem::take(&mut batch);
            let updated = tokio::task::spawn_blocking(move || {
                let changed = index.update_files(&batch_paths);
                let node_count = index.graph().node_count();
                let edge_count = index.graph().edge_count();
                (index, changed, node_count, edge_count)
            })
            .await;
            let Ok((index, changed, node_count, edge_count)) = updated else {
                tracing::warn!("code-graph incremental update task panicked");
                state.code_graph_updating.store(false, Ordering::Release);
                return;
            };

            // Restore under the lock, but stand down if a full build committed a
            // fresher graph while we rebuilt off-mutex. We took the slot to `None`
            // above, so a `Some` now means a full build (or later incremental)
            // repopulated it and our copy is stale; `build_in_flight` catches a full
            // build that acquired the slot during our window but has not committed yet
            // (it will commit a disk-current graph). The authoritative full build
            // wins; a full build always re-reads disk, so dropping our result is safe.
            if generation_current(&state, generation) {
                let mut guard = state.code_graph.lock().await;
                if guard.is_none() && !build_in_flight(&state, generation) {
                    *guard = Some(index);
                    drop(guard);
                    if changed {
                        emit_graph(
                            &app,
                            &CodeGraphEvent::Updated {
                                node_count,
                                edge_count,
                            },
                        );
                    }
                } else {
                    // A full build owns the slot now — it re-reads disk, so drop our
                    // stale copy AND any stashed paths (already covered) and stop.
                    drop(guard);
                    clear_pending_update(&state);
                    return;
                }
            } else {
                // Workspace changed mid-rebuild; a new generation's build covers disk.
                clear_pending_update(&state);
                return;
            }

            // Drain any paths batches stashed while we were rebuilding. If none, clear
            // the flag and finish; if the slot was emptied between the clear and a
            // concurrent enqueue, the next batch starts a fresh update.
            let pending = state
                .code_graph_pending_paths
                .lock()
                .map(|mut p| std::mem::take(&mut *p))
                .unwrap_or_default();
            if pending.is_empty() {
                state.code_graph_updating.store(false, Ordering::Release);
                // Re-check: a batch may have enqueued after our drain but before the
                // flag cleared. If so, it stashed paths and saw the flag set, so it
                // returned expecting us to handle them — pick them up.
                let leftover = state
                    .code_graph_pending_paths
                    .lock()
                    .map(|mut p| std::mem::take(&mut *p))
                    .unwrap_or_default();
                if leftover.is_empty() {
                    return;
                }
                // Re-acquire ownership and loop again with the leftover paths.
                if state.code_graph_updating.swap(true, Ordering::AcqRel) {
                    // Someone else already took ownership; hand the paths back.
                    if let Ok(mut p) = state.code_graph_pending_paths.lock() {
                        p.extend(leftover);
                    }
                    return;
                }
                batch = leftover;
            } else {
                batch = pending;
            }
        }
    });
}

/// Full rebuild for a collapsed (overflowed) watch batch, where individual paths
/// were discarded. Rebuilds the whole index from the root, generation-guarded.
pub fn handle_fs_collapse(app: &AppHandle, state: &SharedState, root: &Path, generation: u64) {
    start_build_on_workspace(app.clone(), state.clone(), root.to_path_buf(), generation);
}

// ── Response types ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
// node_count/edge_count/file_count are the camelCase fields the frontend consumes.
#[allow(clippy::struct_field_names)]
pub struct CodeGraphSummary {
    pub node_count: usize,
    pub edge_count: usize,
    pub file_count: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodeGraphStatus {
    pub ready: bool,
    pub node_count: usize,
    pub edge_count: usize,
    pub file_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryNode {
    pub name: String,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodeGraphQueryResult {
    pub found: bool,
    pub node: Option<QueryNode>,
    pub callers: Vec<QueryNode>,
    pub callees: Vec<QueryNode>,
    pub neighbors: Vec<ConnectionEntry>,
    pub explanation: Option<ExploreEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExploreEntry {
    pub kind: String,
    pub degree: u32,
    pub total_connections: usize,
    pub connections: Vec<ConnectionEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionEntry {
    pub name: String,
    pub file: String,
    pub line: u32,
    pub relation: String,
    pub direction: String,
}
