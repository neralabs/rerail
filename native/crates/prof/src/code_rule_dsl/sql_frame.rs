//! The SQL profiler's counters (crate split 10c): what a virtual table's callbacks count on this thread (`frame`), an
//! index build's span (`index_build`), and whether the profiler's own `EXPLAIN`s run (`explaining`). The profiler
//! (rerail-sql-profile's `sql_profile`) starts and reads them at statement boundaries. They live here, below the
//! facts' virtual tables that count, so those need no profiler and an edit to it rebuilds only the crates that run one.
use std::cell::Cell;
pub mod clock;
pub mod timing;
pub use clock::thread_cpu_ns;
use timing::Started;

thread_local! {
    static FRAME: Cell<[u64; 9]> = const { Cell::new([0; 9]) };
    static ACTIVE: Cell<usize> = const { Cell::new(0) };
    static EXPLAINING: Cell<bool> = const { Cell::new(false) };
}

/// Whether the thread is running the profiler's own `EXPLAIN`s.
pub fn explaining() -> bool {
    EXPLAINING.get()
}

/// While it lives, the thread runs the profiler's own `EXPLAIN`s, which the admission observer leaves alone
/// (`explain`).
pub struct Explanation(bool);
impl Drop for Explanation {
    fn drop(&mut self) {
        EXPLAINING.set(self.0);
    }
}

/// Cumulative callback counts, read at statement boundaries. No clocks in row callbacks.
#[derive(Clone, Copy)]
pub enum FrameEvent {
    Scan,
    Lookup,
    Next,
    Column,
    Insert,
    IndexBuild = 5,
    IndexedRows,
}

#[inline]
pub fn frame(event: FrameEvent, count: usize) {
    if ACTIVE.get() != 0 {
        FRAME.with(|counter| {
            let mut values = counter.get();
            values[event as usize] += count as u64;
            counter.set(values);
        });
    }
}

/// Time only cache misses, including any scan-order preparation needed by the lookup.
/// These spans are contained in SQL time; they must not be added to the CPU budget.
pub fn index_build(rows: usize) -> IndexBuild {
    let started = (ACTIVE.get() != 0).then(Started::now);
    frame(FrameEvent::IndexBuild, 1);
    frame(FrameEvent::IndexedRows, rows);
    IndexBuild(started)
}

pub struct IndexBuild(Option<Started>);

impl Drop for IndexBuild {
    fn drop(&mut self) {
        if let Some(started) = self.0 {
            let elapsed = started.elapsed();
            FRAME.with(|counter| {
                let mut values = counter.get();
                values[7] += elapsed.cpu;
                values[8] += elapsed.wall;
                counter.set(values);
            });
        }
    }
}

/// The profiler's own `EXPLAIN`s run until the guard drops.
pub fn explain() -> Explanation {
    Explanation(EXPLAINING.replace(true))
}

/// The thread's counts so far: a statement's are the difference between its end and its start.
pub fn counts() -> [u64; 9] {
    FRAME.get()
}

/// A traced statement starts: the callbacks count while any runs.
pub fn statement_started() {
    ACTIVE.set(ACTIVE.get() + 1);
}

/// A traced statement ends.
pub fn statement_ended() {
    ACTIVE.set(ACTIVE.get().saturating_sub(1));
}
