//! The thread's own hardware counters, and its core (`join-lab profile`): instructions and cycles of the thread
//! that replays, read before and after a run.
//!
//! - macOS: the process's (`proc_pid_rusage`, `RUSAGE_INFO_V4`'s `ri_instructions` and `ri_cycles`). A profile runs
//!   on one thread, so the process's are the thread's. A thread is not pinned: macOS has no binding of a thread to a
//!   core.
//! - Linux: the thread's (`perf_event_open` on this thread, user space only), pinned to a core
//!   (`sched_setaffinity`). A host whose kernel refuses the counters (`perf_event_paranoid`) has none: a profile
//!   says so, its times still measured.

/// Instructions and cycles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub instructions: u64,
    pub cycles: u64,
}

impl std::ops::Sub for Counts {
    type Output = Counts;
    fn sub(self, b: Counts) -> Counts {
        Counts { instructions: self.instructions.wrapping_sub(b.instructions), cycles: self.cycles.wrapping_sub(b.cycles) }
    }
}

/// The counters open for this thread.
pub struct Counters {
    #[cfg(target_os = "linux")]
    fds: [i32; 2],
}

impl Counters {
    /// This thread's counters, or none where the system gives none.
    pub fn open() -> Option<Counters> {
        #[cfg(target_vendor = "apple")]
        {
            let c = Counters {};
            return (c.read().instructions > 0).then_some(c);
        }
        #[cfg(target_os = "linux")]
        {
            // `perf_event_attr` as far as `PERF_ATTR_SIZE_VER0` (64 bytes): its type, size, config, then the
            // sampling words (none), then its flags: `exclude_kernel` (bit 5) and `exclude_hv` (bit 6).
            #[repr(C)]
            struct Attr {
                kind: u32,
                size: u32,
                config: u64,
                period: u64,
                sample_type: u64,
                read_format: u64,
                flags: u64,
                wakeup: u32,
                bp_type: u32,
                config1: u64,
            }
            let open = |config: u64| {
                let a = Attr { kind: 0, size: std::mem::size_of::<Attr>() as u32, config, period: 0, sample_type: 0, read_format: 0, flags: (1 << 5) | (1 << 6), wakeup: 0, bp_type: 0, config1: 0 };
                // SAFETY: a counter on this thread (pid 0) on any core (-1), no group, from an attr of its own.
                unsafe { libc::syscall(libc::SYS_perf_event_open, &raw const a, 0, -1, -1, 0) as i32 }
            };
            // PERF_COUNT_HW_INSTRUCTIONS, PERF_COUNT_HW_CPU_CYCLES.
            let fds = [open(1), open(0)];
            if fds.iter().any(|&f| f < 0) {
                for f in fds.into_iter().filter(|&f| f >= 0) {
                    // SAFETY: a descriptor of ours, closed once.
                    unsafe { libc::close(f) };
                }
                return None;
            }
            return Some(Counters { fds });
        }
        #[allow(unreachable_code)]
        None
    }

    /// The counts now.
    pub fn read(&self) -> Counts {
        #[cfg(target_vendor = "apple")]
        {
            // SAFETY: a zeroed `rusage_info_v4` (integers), filled by the system for this process.
            let mut r: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
            // SAFETY: the buffer is a `rusage_info_v4`, the flavor's.
            let ok = unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, (&raw mut r).cast()) } == 0;
            return if ok { Counts { instructions: r.ri_instructions, cycles: r.ri_cycles } } else { Counts::default() };
        }
        #[cfg(target_os = "linux")]
        {
            let read = |fd: i32| {
                let mut v = 0u64;
                // SAFETY: eight bytes into a u64 of ours, from a counter's descriptor.
                let n = unsafe { libc::read(fd, (&raw mut v).cast(), 8) };
                if n == 8 { v } else { 0 }
            };
            return Counts { instructions: read(self.fds[0]), cycles: read(self.fds[1]) };
        }
        #[allow(unreachable_code)]
        Counts::default()
    }
}

#[cfg(target_os = "linux")]
impl Drop for Counters {
    fn drop(&mut self) {
        for f in self.fds {
            // SAFETY: descriptors of ours, closed once.
            unsafe { libc::close(f) };
        }
    }
}

/// This thread pinned to core `core`: whether it is (never on macOS).
pub fn pin(core: usize) -> bool {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: a zeroed set with one core in it, for this thread (pid 0).
        unsafe {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            libc::CPU_SET(core, &mut set);
            return libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set) == 0;
        }
    }
    #[allow(unreachable_code)]
    {
        let _ = core;
        false
    }
}

#[cfg(test)]
mod tests {
    /// The counters, where the system gives them, count a loop's instructions: more for a longer loop.
    #[test]
    fn the_counters_count_this_threads_work() {
        let Some(c) = super::Counters::open() else {
            eprintln!("this system gives no counters");
            return;
        };
        let work = |n: u64| {
            let a = c.read();
            let mut x = 0u64;
            for i in 0..n {
                x = std::hint::black_box(x.wrapping_mul(31).wrapping_add(i));
            }
            std::hint::black_box(x);
            c.read() - a
        };
        let (small, big) = (work(10_000), work(10_000_000));
        eprintln!("counted: {small:?} {big:?}");
        assert!(big.instructions >= 10_000_000 && big.instructions > small.instructions && big.cycles > small.cycles, "{small:?} {big:?}");
    }
}
