//! The most threads the engine's process uses at once: `RERAIL_THREADS`, else the machine's cores.
//!
//! ```text
//! RERAIL_THREADS=4 rerail-engine logic …    a corpus run: the rules on the main thread, chunks cut and projected on
//!                                           one worker, and the documents read, parsed and made facts on the other two
//! ```
//!
//! Rayon's global pool and the shared work pool (`WORK_POOL`) are sized to it, and a corpus run splits it between
//! its rules, its chunk worker and its document builders (`logic::plan::corpus`).

use std::sync::OnceLock;

/// The setting: `RERAIL_THREADS`, a whole number of at least 1; else the cores the machine offers.
pub fn max() -> usize {
    static MAX: OnceLock<usize> = OnceLock::new();
    *MAX.get_or_init(|| {
        std::env::var("RERAIL_THREADS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|n| *n >= 1)
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get))
    })
}

/// Sizes rayon's global pool to `max`: called before anything uses it (the CLI's start). A pool already made is left
/// as it is.
pub fn configure() {
    let _ = rayon::ThreadPoolBuilder::new().num_threads(max()).build_global();
}

/// The threads a corpus run builds documents on beside its main thread (the rules) and its chunk worker: none when
/// the setting leaves no thread for them (the worker builds them itself), and none without threads (wasm32).
pub(crate) fn document_builders() -> usize {
    if cfg!(target_family = "wasm") { 0 } else { max().saturating_sub(2) }
}

#[cfg(test)]
mod tests {
    /// The setting is at least one thread, and a run's builders leave two of them to its rules and its chunk worker.
    #[test]
    fn builders_leave_the_rules_and_the_worker_their_threads() {
        assert!(super::max() >= 1);
        assert_eq!(super::document_builders(), super::max().saturating_sub(2));
    }
}
