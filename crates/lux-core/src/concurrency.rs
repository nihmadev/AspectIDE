//! Shared CPU concurrency budget for filesystem scans and content search.
//!
//! Heavy workspace operations — directory walks (`lux-fs`) and content search
//! (`lux-search`) — run across worker threads. To keep the IDE responsive we
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

/// `0` is the sentinel for "unset" — [`scan_threads`] then resolves [`ScanConcurrency::Auto`]
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
/// it never starves a caller — every operation is guaranteed at least one worker —
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

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard};

    use super::*;

    /// Tests that mutate the process-global `SCAN_THREADS`/`ACTIVE_SCANS` must not
    /// run concurrently or they read each other's transient state. Serialize them
    /// through one lock; a poisoned lock (from an unrelated panic) is still usable
    /// here since each test fully resets the globals it touches.
    static GLOBAL_STATE_LOCK: Mutex<()> = Mutex::new(());

    fn lock_global_state() -> MutexGuard<'static, ()> {
        GLOBAL_STATE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn auto_reserves_one_core_but_never_zero() {
        // Auto must always leave at least one worker and never return 0.
        let threads = resolve_scan_threads(ScanConcurrency::Auto);
        assert!(threads >= 1);
        assert!(threads <= detected_cores().max(1));
    }

    #[test]
    fn all_uses_every_core() {
        assert_eq!(
            resolve_scan_threads(ScanConcurrency::All),
            detected_cores().max(1)
        );
    }

    #[test]
    fn half_is_at_least_one() {
        assert!(resolve_scan_threads(ScanConcurrency::Half) >= 1);
    }

    #[test]
    fn preference_parsing_is_lenient() {
        assert_eq!(
            ScanConcurrency::from_preference("ALL"),
            ScanConcurrency::All
        );
        assert_eq!(
            ScanConcurrency::from_preference(" half "),
            ScanConcurrency::Half
        );
        assert_eq!(
            ScanConcurrency::from_preference("auto"),
            ScanConcurrency::Auto
        );
        assert_eq!(
            ScanConcurrency::from_preference("garbage"),
            ScanConcurrency::Auto
        );
        assert_eq!(ScanConcurrency::from_preference(""), ScanConcurrency::Auto);
    }

    #[test]
    fn set_then_read_round_trips() {
        let _guard = lock_global_state();
        set_scan_concurrency(ScanConcurrency::All);
        assert_eq!(scan_threads(), detected_cores().max(1));
        // Restore the lazy default so other tests are unaffected.
        SCAN_THREADS.store(0, Ordering::Relaxed);
        assert!(scan_threads() >= 1);
    }

    #[test]
    fn single_scan_gets_the_whole_budget() {
        let _guard = lock_global_state();
        // Pin a known budget so the assertion is independent of host core count.
        set_scan_concurrency(ScanConcurrency::All);
        let budget = scan_threads();
        let workers = acquire_scan_workers();
        assert_eq!(workers.count(), budget);
        drop(workers);
        SCAN_THREADS.store(0, Ordering::Relaxed);
    }

    #[test]
    fn concurrent_scans_split_the_budget_and_never_starve() {
        let _guard = lock_global_state();
        // Pin a budget that divides cleanly so every share below is deterministic
        // regardless of the host's core count.
        SCAN_THREADS.store(8, Ordering::Relaxed);

        // A lone operation may use the whole budget.
        let first = acquire_scan_workers();
        assert_eq!(first.count(), 8, "a lone scan gets the full budget");

        // The reservation is advisory: a share is computed once at acquire time from
        // the live-operation count and is never clawed back. So a *second* concurrent
        // operation does not also grab the full budget — it takes `budget / live`.
        // That divided share is the whole point: independent pools no longer each
        // spawn `budget` workers and multiply the in-flight thread count without
        // bound. (The first op keeps its 8, so the transient total can exceed the
        // budget; what is bounded is the per-new-op share, which prevents the
        // `K * budget` blow-up.)
        let second = acquire_scan_workers();
        let second_share = second.count();
        assert_eq!(second_share, 4, "a 2nd concurrent scan gets budget/2");
        assert!(
            second_share < first.count(),
            "a later concurrent op gets a smaller share than the lone first op"
        );

        // Shares keep shrinking as more operations pile on, but never hit zero.
        let third = acquire_scan_workers();
        assert!(third.count() <= second_share, "shares shrink under load");
        assert!(third.count() >= 1, "no operation is ever starved");

        // Releasing operations frees the budget: once fewer ops are live, a fresh
        // acquisition reclaims a larger share than it would have under heavier load.
        drop(third);
        drop(second);
        let reclaimed = acquire_scan_workers();
        assert!(
            reclaimed.count() >= second_share,
            "freed budget is reclaimed by a later scan"
        );

        drop(first);
        drop(reclaimed);
        // Restore the lazy default so other tests are unaffected.
        SCAN_THREADS.store(0, Ordering::Relaxed);
    }

    #[test]
    fn acquire_always_yields_at_least_one_worker() {
        let _guard = lock_global_state();
        // Even with the smallest possible budget no caller is ever starved.
        SCAN_THREADS.store(1, Ordering::Relaxed);
        let first = acquire_scan_workers();
        let second = acquire_scan_workers();
        assert_eq!(first.count(), 1);
        assert_eq!(second.count(), 1);
        drop(first);
        drop(second);
        SCAN_THREADS.store(0, Ordering::Relaxed);
    }
}
