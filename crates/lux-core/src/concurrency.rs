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
#[must_use]
pub fn scan_threads() -> usize {
    let stored = SCAN_THREADS.load(Ordering::Relaxed);
    if stored == 0 {
        resolve_scan_threads(ScanConcurrency::Auto)
    } else {
        stored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        set_scan_concurrency(ScanConcurrency::All);
        assert_eq!(scan_threads(), detected_cores().max(1));
        // Restore the lazy default so other tests are unaffected.
        SCAN_THREADS.store(0, Ordering::Relaxed);
        assert!(scan_threads() >= 1);
    }
}
