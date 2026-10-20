//! The engine's timers: each measures wall time and the running thread's CPU time
//! (`CLOCK_THREAD_CPUTIME_ID`), so times summed over a pool's workers add up to CPU the process
//! spent, which their wall time under load does not.

use std::time::Instant;

/// Time spent, in nanoseconds: wall-clock and the thread's CPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Timing {
    pub(in crate::code_rule_dsl) wall: u64,
    pub(in crate::code_rule_dsl) cpu: u64,
}

impl Timing {
    pub(in crate::code_rule_dsl) const ZERO: Self = Self { wall: 0, cpu: 0 };

    pub(in crate::code_rule_dsl) fn add(&mut self, other: Self) {
        self.wall += other.wall;
        self.cpu += other.cpu;
    }

    pub(in crate::code_rule_dsl) fn plus(mut self, other: Self) -> Self {
        self.add(other);
        self
    }

    /// What was added from `before` to `self`.
    pub(in crate::code_rule_dsl) fn since(self, before: Self) -> Self {
        Self {
            wall: self.wall.saturating_sub(before.wall),
            cpu: self.cpu.saturating_sub(before.cpu),
        }
    }

    pub(in crate::code_rule_dsl) fn wall_milliseconds(self) -> f64 {
        self.wall as f64 / 1e6
    }

    pub(in crate::code_rule_dsl) fn cpu_milliseconds(self) -> f64 {
        self.cpu as f64 / 1e6
    }
}

/// When a timed span started, on this thread.
#[derive(Clone, Copy)]
pub(in crate::code_rule_dsl) struct Started {
    wall: Instant,
    cpu: u64,
}

impl Started {
    pub(in crate::code_rule_dsl) fn now() -> Self {
        Self {
            wall: Instant::now(),
            cpu: thread_cpu_nanos(),
        }
    }

    /// The time since it started; the span must not leave its thread.
    pub(in crate::code_rule_dsl) fn elapsed(&self) -> Timing {
        Timing {
            wall: self.wall.elapsed().as_nanos() as u64,
            cpu: thread_cpu_nanos().saturating_sub(self.cpu),
        }
    }
}

/// `body`, and the time it took on this thread.
pub(in crate::code_rule_dsl) fn timed<T>(into: &mut Timing, body: impl FnOnce() -> T) -> T {
    let started = Started::now();
    let out = body();
    into.add(started.elapsed());
    out
}

/// This thread's CPU time; 0 where the platform has no per-thread clock.
#[cfg(unix)]
fn thread_cpu_nanos() -> u64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: the pointer is valid writable storage; the clock belongs to this thread.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &raw mut time) } != 0 {
        return 0;
    }
    time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64
}

#[cfg(not(unix))]
fn thread_cpu_nanos() -> u64 {
    0
}
