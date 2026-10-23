//! Process high-water marks and allocator residency at selected diagnostic boundaries.
#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Sample {
    peak_rss_bytes: u64,
    rss_bytes: Option<u64>,
    allocated_bytes: Option<usize>,
    in_use_bytes: Option<usize>,
}
pub(crate) fn sample(phase: &str) -> Option<Sample> {
    if !matches!(
        phase,
        "producer-prepare"
            | "input-stream"
            | "sql-input"
            | "sql-materialize"
            | "sql-index"
            | "producer-publish"
            | "anchor-working-set"
    ) {
        return None;
    }
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        // SAFETY: getrusage writes the initialized structure on success.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: the successful call initialized usage.
        let peak = unsafe { usage.assume_init() }.ru_maxrss.max(0) as u64;
        let peak_rss_bytes = if cfg!(target_os = "macos") {
            peak
        } else {
            peak.saturating_mul(1024)
        };
        let (allocated_bytes, in_use_bytes) = allocator();
        Some(Sample {
            peak_rss_bytes,
            rss_bytes: resident(),
            allocated_bytes,
            in_use_bytes,
        })
    }
    #[cfg(not(unix))]
    {
        None
    }
}
#[cfg(target_os = "macos")]
fn resident() -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::mach_task_basic_info>::zeroed();
    let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
    // SAFETY: task_info writes this C-layout buffer; count describes its capacity in words.
    #[allow(deprecated)]
    let code = unsafe {
        libc::task_info(
            libc::mach_task_self(),
            libc::MACH_TASK_BASIC_INFO,
            info.as_mut_ptr().cast(),
            &mut count,
        )
    };
    // SAFETY: a successful task_info call initializes the output structure.
    (code == libc::KERN_SUCCESS).then(|| unsafe { info.assume_init() }.resident_size)
}
#[cfg(target_os = "linux")]
fn resident() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages = status.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    // SAFETY: sysconf has no pointer arguments.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    (page_size > 0).then(|| pages.saturating_mul(page_size as u64))
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn resident() -> Option<u64> {
    None
}
#[cfg(target_os = "macos")]
fn allocator() -> (Option<usize>, Option<usize>) {
    #[repr(C)]
    #[derive(Default)]
    struct Statistics {
        blocks: libc::c_uint,
        in_use: usize,
        maximum: usize,
        allocated: usize,
    }
    unsafe extern "C" {
        fn malloc_zone_statistics(zone: *mut libc::c_void, stats: *mut Statistics);
    }
    let mut stats = Statistics::default();
    // SAFETY: the macOS API sums all zones for a null zone and writes this C-layout structure.
    unsafe {
        malloc_zone_statistics(std::ptr::null_mut(), &mut stats);
    }
    (Some(stats.allocated), Some(stats.in_use))
}
#[cfg(all(unix, not(target_os = "macos")))]
fn allocator() -> (Option<usize>, Option<usize>) {
    (None, None)
}
#[cfg(test)]
mod tests {
    #[test]
    fn memory_probes_are_limited_to_selected_boundaries() {
        assert!(super::sample("anchor-lower").is_none());
        #[cfg(unix)]
        assert!(super::sample("producer-prepare").unwrap().peak_rss_bytes > 0);
    }
}
