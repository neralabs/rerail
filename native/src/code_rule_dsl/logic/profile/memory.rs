//! Memory for the profile (decision 0084): what a thread allocated and freed, from jemalloc's per-thread counters,
//! and the process's physical footprint now and at its high-water mark.
//!
//! - **Per thread.** jemalloc keeps two running byte counts per thread, `thread.allocatedp` and
//!   `thread.deallocatedp`, as pointers into the thread's own data. Reading them is two loads, so a span charges what
//!   its thread allocated and freed while it was open. Another thread may read them too (a dump, `watch`): they stay
//!   valid while the thread lives, and a torn read only makes the dump approximate.
//! - **Per process.** The footprint is what the system charges the process (macOS: `ri_phys_footprint`, and
//!   `ri_lifetime_max_phys_footprint` for its high-water mark; elsewhere the resident set's peak only). A span records
//!   how far the high-water mark rose while it was open: the spans that set the peak.
//!
//! Without jemalloc (Windows, wasm) the thread counters read 0; without a footprint counter (wasm) so does the process.

/// A thread's jemalloc counters: the addresses of its allocated and deallocated byte counts.
#[derive(Clone, Copy, Default)]
pub(super) struct Counters {
    allocated: usize,
    freed: usize,
}

impl Counters {
    /// This thread's counters (none without jemalloc).
    pub(super) fn mine() -> Self {
        #[cfg(not(any(windows, target_family = "wasm")))]
        {
            // SAFETY: both keys name a `uint64_t *` into this thread's data (jemalloc's mallctl documentation).
            let allocated = unsafe { tikv_jemalloc_ctl::raw::read::<*mut u64>(b"thread.allocatedp\0") };
            let freed = unsafe { tikv_jemalloc_ctl::raw::read::<*mut u64>(b"thread.deallocatedp\0") };
            if let (Ok(a), Ok(f)) = (allocated, freed) {
                return Counters { allocated: a as usize, freed: f as usize };
            }
        }
        Counters::default()
    }

    /// Bytes allocated and freed by the thread so far.
    pub(super) fn read(self) -> (u64, u64) {
        if self.allocated == 0 {
            return (0, 0);
        }
        // SAFETY: the addresses are the thread's jemalloc counters, valid while it lives; its registry entry (and
        // so these) is dropped with its thread-local data. A read from another thread may race a write: it is a
        // volatile load of an aligned u64, at worst a stale value.
        unsafe { (std::ptr::read_volatile(self.allocated as *const u64), std::ptr::read_volatile(self.freed as *const u64)) }
    }
}

/// The process's physical footprint now and its high-water mark, in bytes (0 where unknown).
pub(super) fn footprint() -> (u64, u64) {
    #[cfg(target_os = "macos")]
    {
        let mut info = unsafe { std::mem::zeroed::<libc::rusage_info_v4>() };
        // SAFETY: RUSAGE_INFO_V4 fills a `rusage_info_v4` for this process.
        let got = unsafe { libc::proc_pid_rusage(std::process::id() as i32, libc::RUSAGE_INFO_V4, (&raw mut info).cast()) };
        if got == 0 {
            return (info.ri_phys_footprint, info.ri_lifetime_max_phys_footprint);
        }
        (0, 0)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        // Linux reports kilobytes, and only the peak.
        let peak = usage.ru_maxrss as u64 * 1024;
        (0, peak)
    }
    #[cfg(not(unix))]
    {
        (0, 0)
    }
}

/// jemalloc's heap: bytes allocated (live) and resident, now (0 without jemalloc).
pub(super) fn heap() -> (u64, u64) {
    #[cfg(not(any(windows, target_family = "wasm")))]
    {
        let _ = tikv_jemalloc_ctl::epoch::advance();
        return (tikv_jemalloc_ctl::stats::allocated::read().unwrap_or(0) as u64, tikv_jemalloc_ctl::stats::resident::read().unwrap_or(0) as u64);
    }
    #[allow(unreachable_code)]
    (0, 0)
}

/// Bytes as MB, one decimal.
pub(super) fn mb(bytes: f64) -> String {
    format!("{:.1}", bytes / (1024.0 * 1024.0))
}
