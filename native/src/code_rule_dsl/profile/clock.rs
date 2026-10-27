//! Thread CPU clock shared by profiling hosts.
#[cfg(unix)]
pub(crate) fn thread_cpu_ns() -> Option<u128> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: the pointer is valid writable storage; the clock belongs to this thread.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) } != 0 {
        return None;
    }
    Some(u128::try_from(time.tv_sec).ok()? * 1_000_000_000 + u128::try_from(time.tv_nsec).ok()?)
}

#[cfg(not(unix))]
pub(crate) fn thread_cpu_ns() -> Option<u128> {
    None
}
