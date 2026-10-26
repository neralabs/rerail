//! Process-wide CPU time (of the calling thread, so waiting on a lock counts nothing) of the phases a performance comparison must keep apart (decision 0006):
//! construction (anchor index publication) and execution (anchor selection, relational SQL).
//! Parse and clause matching are timed per apply in `Metrics`. Read through `workspaceProfile`.

use std::sync::atomic::{AtomicU64, Ordering};

/// Construction: files published into the anchor index (`Corpus::put`).
pub(super) static INDEX_PUBLISH: AtomicU64 = AtomicU64::new(0);
/// Execution: candidate roots selected from the anchor index (`Corpus::prepare*`).
pub(super) static ANCHOR_SELECT: AtomicU64 = AtomicU64::new(0);
/// Construction, nested in `ANCHOR_SELECT` and reported apart from it: anchor plans lowered to SQL
/// and the SQL prepared.
pub(super) static ANCHOR_COMPILE: AtomicU64 = AtomicU64::new(0);
/// Execution: relational sessions, from input rows to their findings (`relational::evaluate_batch`, `relational::evaluate_prepared`).
pub(super) static RELATIONAL: AtomicU64 = AtomicU64::new(0);

/// Runs `run`, adding the CPU time this thread spent in it to `clock`.
pub(super) fn timed<T>(clock: &AtomicU64, run: impl FnOnce() -> T) -> T {
    let started = thread_cpu_nanoseconds();
    let result = run();
    clock.fetch_add(thread_cpu_nanoseconds().saturating_sub(started), Ordering::Relaxed);
    result
}

/// Adds the CPU time this thread spends until the span drops to `clock`.
pub(super) struct Span<'c> {
    clock: &'c AtomicU64,
    started: u64,
}

impl<'c> Span<'c> {
    pub(super) fn start(clock: &'c AtomicU64) -> Self {
        Self { clock, started: thread_cpu_nanoseconds() }
    }
}

impl Drop for Span<'_> {
    fn drop(&mut self) {
        self.clock
            .fetch_add(thread_cpu_nanoseconds().saturating_sub(self.started), Ordering::Relaxed);
    }
}

fn thread_cpu_nanoseconds() -> u64 {
    let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `now` is a valid timespec for the call to fill.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut now) } != 0 {
        return 0;
    }
    now.tv_sec as u64 * 1_000_000_000 + now.tv_nsec as u64
}

/// The clocks' totals so far, in nanoseconds.
pub(super) fn snapshot() -> serde_json::Value {
    serde_json::json!({
        "indexPublishNanoseconds": INDEX_PUBLISH.load(Ordering::Relaxed),
        "anchorCompileNanoseconds": ANCHOR_COMPILE.load(Ordering::Relaxed),
        "anchorSelectNanoseconds": ANCHOR_SELECT
            .load(Ordering::Relaxed)
            .saturating_sub(ANCHOR_COMPILE.load(Ordering::Relaxed)),
        "relationalNanoseconds": RELATIONAL.load(Ordering::Relaxed),
    })
}
