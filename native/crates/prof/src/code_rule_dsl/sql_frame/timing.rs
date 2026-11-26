//! The engine's timers: each measures wall time and the running thread's CPU time
//! (`CLOCK_THREAD_CPUTIME_ID`), so times summed over a pool's workers add up to CPU the process
//! spent, which their wall time under load does not.

use std::time::Instant;

/// Time spent, in nanoseconds: wall-clock and the thread's CPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timing {
    pub wall: u64,
    pub cpu: u64,
}

impl Timing {
    pub fn add(&mut self, other: Self) {
        self.wall += other.wall;
        self.cpu += other.cpu;
    }

    /// What was added from `before` to `self`.
    pub fn since(self, before: Self) -> Self {
        Self {
            wall: self.wall.saturating_sub(before.wall),
            cpu: self.cpu.saturating_sub(before.cpu),
        }
    }
}

/// When a timed span started, on this thread.
#[derive(Clone, Copy)]
pub struct Started {
    wall: Instant,
    cpu: u64,
}

impl Started {
    pub fn now() -> Self {
        Self {
            wall: Instant::now(),
            cpu: thread_cpu_nanos(),
        }
    }

    /// The time since it started; the span must not leave its thread.
    pub fn elapsed(&self) -> Timing {
        Timing {
            wall: self.wall.elapsed().as_nanos() as u64,
            cpu: thread_cpu_nanos().saturating_sub(self.cpu),
        }
    }
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
