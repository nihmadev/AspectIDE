//! Shared CPU concurrency budget for filesystem scans and content search.
//!
//! Heavy workspace operations вЂ” directory walks (`aspect-fs`) and content search
//! (`aspect-search`) вЂ” run across worker threads. To keep the IDE responsive we
//! reserve headroom for the UI/IPC/WebView by default instead of saturating every
//! logical core: a full-core scan on a large repo would otherwise contend with the
//! main thread and cause visible stutter.
//!
//! The budget is process-global and set once from the frontend preference; it is
//! re-applied whenever the user changes it. `Auto` resolves per-machine via
//! [`std::thread::available_parallelism`], so a sensible default holds on any host
//! without hardcoding a core count.

use std::{
    num::NonZeroUsize,
    sync::atomic::{AtomicUsize, Ordering},
    thread::available_parallelism,
};

/// Number of scan/search operations currently holding a worker reservation.
///
/// This is the process-global coordination point that makes [`scan_threads`] a
/// *shared* budget rather than a per-operation one: every concurrent walk divides
/// the budget by the live operation count, so K simultaneous scans spawn ~`budget`
/// workers in total instead of `K * budget`. See [`acquire_scan_workers`].
static ACTIVE_SCANS: AtomicUsize = AtomicUsize::new(0);

/// Concurrency policy for the scan/search worker pools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanConcurrency {
    /// Reserve one logical core for the UI: `max(1, cores - 1)`. Default.
    Auto,
    /// Use every logical core. Fastest scans, may briefly stutter the UI on
    /// low-core machines during large operations.
    All,
    /// Use half the logical cores: `max(1, cores / 2)`. Gentlest on responsiveness.
    Half,
}

impl ScanConcurrency {
    /// Parses a frontend preference value. Unknown/empty input falls back to
    /// [`ScanConcurrency::Auto`], matching the default policy.
    #[must_use]
    pub fn from_preference(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "all" => Self::All,
            "half" => Self::Half,
            _ => Self::Auto,
        }
    }
}

/// `0` is the sentinel for "unset" вЂ” [`scan_threads`] then resolves [`ScanConcurrency::Auto`]
/// lazily. Any non-zero value is the explicit, already-resolved worker count.
static SCAN_THREADS: AtomicUsize = AtomicUsize::new(0);

/// Detected logical core count, clamped to at least 1.
fn detected_cores() -> usize {
    available_parallelism().map_or(1, NonZeroUsize::get)
}

/// Resolves a policy to a concrete worker-thread count for the current machine.
#[must_use]
pub fn resolve_scan_threads(mode: ScanConcurrency) -> usize {
    let cores = detected_cores();
    match mode {
        ScanConcurrency::Auto => cores.saturating_sub(1).max(1),
        ScanConcurrency::All => cores.max(1),
        ScanConcurrency::Half => (cores / 2).max(1),
    }
}

/// Sets the global scan/search concurrency budget. Called from the frontend
/// preference at startup and whenever the user changes the setting.
pub fn set_scan_concurrency(mode: ScanConcurrency) {
    SCAN_THREADS.store(resolve_scan_threads(mode), Ordering::Relaxed);
}

/// Current scan/search worker-thread budget (always `>= 1`). Defaults to the
/// resolved [`ScanConcurrency::Auto`] value until [`set_scan_concurrency`] runs.
///
/// This is the *total* process budget, not a per-operation allowance. Callers that
/// spawn worker pools (directory walks, content search) should reserve their slice
/// of it through [`acquire_scan_workers`] so concurrent operations share the budget
/// instead of each claiming all of it.
#[must_use]
pub fn scan_threads() -> usize {
    let stored = SCAN_THREADS.load(Ordering::Relaxed);
    if stored == 0 {
        resolve_scan_threads(ScanConcurrency::Auto)
    } else {
        stored
    }
}

/// A reservation of worker threads carved out of the process-global scan budget.
///
/// Hold this for the lifetime of a walk/search and size the worker pool with
/// [`ScanWorkers::count`]. Dropping it releases the reservation so later operations
/// reclaim the budget. The reservation is intentionally advisory (no hard blocking):
/// it never starves a caller вЂ” every operation is guaranteed at least one worker вЂ”
/// while still preventing the `K * budget` thread blow-up of independent pools.
#[derive(Debug)]
pub struct ScanWorkers {
    count: usize,
}

impl ScanWorkers {
    /// Worker-thread count this operation may use (always `>= 1`).
    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
    }
}

impl Drop for ScanWorkers {
    fn drop(&mut self) {
        // Releasing our slot lets the *next* operation to call `acquire_scan_workers`
        // see a smaller live count and claim a larger fair share.
        ACTIVE_SCANS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Reserve a fair slice of the global scan/search worker budget for one operation.
///
/// The slice is `max(1, budget / live_operations)`: the first concurrent scan can
/// use the whole budget, a second simultaneous scan takes half each, and so on, so
/// the *total* in-flight worker threads stay near [`scan_threads`] no matter how
/// many AI tools, file pickers, and background indexers run at once. The returned
/// guard must outlive the worker pool it sizes.
#[must_use]
pub fn acquire_scan_workers() -> ScanWorkers {
    let budget = scan_threads();
    // `fetch_add` returns the prior value; `+ 1` is our own slot, so `live` counts
    // this operation too and the division never yields a zero share.
    let live = ACTIVE_SCANS.fetch_add(1, Ordering::AcqRel) + 1;
    let count = (budget / live).max(1).min(budget);
    ScanWorkers { count }
}

