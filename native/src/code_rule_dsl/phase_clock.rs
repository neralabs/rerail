//! Process-wide CPU time (of the calling thread, so waiting on a lock counts nothing) of the phases a performance comparison must keep apart (decision 0006):
//! construction (anchor index publication) and execution (anchor selection, relational SQL).
//! Parse and clause matching are timed per apply in `Metrics`. Read through `workspaceProfile`.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

thread_local! {
    /// Set while this thread runs phase 0 (`logic::plan::corpus::phase0`): `bootstrap.rerail` itself is read by the
    /// builtin reader, and the profiler's roofline measures nothing in it.
    pub static IN_PHASE_0: Cell<bool> = const { Cell::new(false) };
}

/// Whether this thread is running phase 0 (`bootstrap.rerail` and `core.rerail` are then read by the builtin reader).
pub fn active() -> bool {
    IN_PHASE_0.with(Cell::get)
}

/// A run's phase clocks, in CPU nanoseconds (decision 0442: the run's).
#[derive(Default)]
pub struct Clocks {
    /// Construction: files published into the anchor index (`Corpus::put`).
    pub index_publish: AtomicU64,
    /// Execution: candidate roots selected from the anchor index (`Corpus::prepare*`).
    pub anchor_select: AtomicU64,
    /// Construction, nested in `anchor_select` and reported apart from it: anchor plans lowered to SQL and the SQL
    /// prepared.
    pub anchor_compile: AtomicU64,
    /// Execution: relational sessions, from input rows to their findings (`relational::evaluate_batch`,
    /// `relational::evaluate_prepared`).
    pub relational: AtomicU64,
}

/// The run's clocks.
pub fn clocks() -> std::sync::Arc<Clocks> {
    crate::code_rule_dsl::logic::profile::local::<Clocks>()
}

/// Which of a run's clocks (`|c| &c.relational`).
pub type Clock = fn(&Clocks) -> &AtomicU64;

/// Runs `run`, adding the CPU time this thread spent in it to the run's `clock`.
pub fn timed<T>(clock: Clock, run: impl FnOnce() -> T) -> T {
    let _span = Span::start(clock);
    run()
}

/// Adds the CPU time this thread spends until the span drops to the run's `clock` (the run it started in).
pub struct Span {
    clocks: std::sync::Arc<Clocks>,
    clock: Clock,
    started: u64,
}

impl Span {
    pub fn start(clock: Clock) -> Self {
        Self { clocks: clocks(), clock, started: thread_cpu_nanoseconds() }
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        (self.clock)(&self.clocks).fetch_add(thread_cpu_nanoseconds().saturating_sub(self.started), Ordering::Relaxed);
    }
}

#[cfg(target_family = "wasm")]
fn thread_cpu_nanoseconds() -> u64 {
    crate::code_rule_dsl::prof::single_thread_cpu().as_nanos() as u64
}

#[cfg(unix)]
fn thread_cpu_nanoseconds() -> u64 {
    let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `now` is a valid timespec for the call to fill.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut now) } != 0 {
        return 0;
    }
    now.tv_sec as u64 * 1_000_000_000 + now.tv_nsec as u64
}

#[cfg(windows)]
fn thread_cpu_nanoseconds() -> u64 {
    let (user, kernel) = crate::code_rule_dsl::prof::windows_cpu(true);
    (user + kernel).as_nanos() as u64
}

/// The run's clocks' totals so far, in nanoseconds.
pub fn snapshot() -> serde_json::Value {
    let c = clocks();
    serde_json::json!({
        "indexPublishNanoseconds": c.index_publish.load(Ordering::Relaxed),
        "anchorCompileNanoseconds": c.anchor_compile.load(Ordering::Relaxed),
        "anchorSelectNanoseconds": c.anchor_select
            .load(Ordering::Relaxed)
            .saturating_sub(c.anchor_compile.load(Ordering::Relaxed)),
        "relationalNanoseconds": c.relational.load(Ordering::Relaxed),
    })
}
