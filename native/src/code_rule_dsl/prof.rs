//! Wall time by stage, for profiling the wave (O28): the counters are the `rerail-prof` crate's
//! (shared with the engine's crates); here are the engine's memory readings.

#[allow(unused_imports)]
pub(crate) use rerail_prof::{Stage, count, count_named, on, report, time};
#[cfg(target_family = "wasm")]
pub(crate) use rerail_prof::single_thread_cpu;

/// Records the process's peak resident memory so far, in MB, as `stage`'s count (the largest seen).
/// Return the heap's freed pages to the system at a stage's end. jemalloc keeps freed pages a
/// second (`lib.rs`) and an audit's stages free hundreds of MB within one: S7 on Metal held
/// 0.96 GB of footprint against 283 MB live heap and 101 MB of buffers (O30).
pub(in crate::code_rule_dsl) fn release() {
    let _stage = Stage::start("heap.release");
    crate::purge_allocator_pages();
    // The system allocator's freed pages too (SQLite's and the Metal runtime's allocations).
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
    }
}

pub(in crate::code_rule_dsl) fn peak(stage: &str) {
    if on() {
        // No resident-set counter on wasm32: its peak reads 0.
        #[cfg(target_family = "wasm")]
        let mb = 0;
        #[cfg(not(target_family = "wasm"))]
        let mb = {
            let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
            unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
            // macOS reports bytes; Linux, kilobytes.
            if cfg!(target_os = "macos") { usage.ru_maxrss as usize >> 20 } else { usage.ru_maxrss as usize >> 10 }
        };
        let mut stages = rerail_prof::stages();
        let entry = stages.entry(format!("peak_mb.{stage}")).or_default();
        entry.1 = entry.1.max(mb);
        // The memory held now, not the process's high-water mark: its physical footprint.
        #[cfg(target_os = "macos")]
        {
            let mut info = unsafe { std::mem::zeroed::<libc::rusage_info_v4>() };
            let got = unsafe {
                libc::proc_pid_rusage(std::process::id() as i32, libc::RUSAGE_INFO_V4, (&raw mut info).cast())
            };
            if got == 0 {
                let entry = stages.entry(format!("now_mb.{stage}")).or_default();
                entry.1 = entry.1.max((info.ri_phys_footprint >> 20) as usize);
                // The process's footprint high-water mark so far: the stage that raised it.
                let max = (info.ri_lifetime_max_phys_footprint >> 20) as usize;
                let entry = stages.entry(format!("max_mb.{stage}")).or_default();
                entry.1 = entry.1.max(max);
                // `RERAIL_PROF_RISES=1`: each stage at which the high-water mark rose, in order.
                static RISEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                if std::env::var_os("RERAIL_PROF_RISES").is_some() && max > RISEN.fetch_max(max, std::sync::atomic::Ordering::Relaxed) + 32 {
                    eprintln!("footprint high-water {max} MB by {stage} (now {} MB)", info.ri_phys_footprint >> 20);
                }
                // `RERAIL_DIAG_PAUSE_AT=<stage>:<n>` (with `RERAIL_DIAG_PAUSE=<seconds>`): the n-th time
                // the stage is reached, wait that long (bounded), for `footprint` on the process.
                if let (Ok(at), Some(seconds)) = (std::env::var("RERAIL_DIAG_PAUSE_AT"), std::env::var("RERAIL_DIAG_PAUSE").ok().and_then(|s| s.parse::<u64>().ok())) {
                    static SEEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                    let (name, nth) = at.split_once(':').map_or((at.as_str(), 1), |(name, nth)| (name, nth.parse().unwrap_or(1)));
                    if name == stage && SEEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1 == nth {
                        eprintln!("DIAG pause at {stage}, pid {}", std::process::id());
                        std::thread::sleep(std::time::Duration::from_secs(seconds.min(30)));
                    }
                }
                // `RERAIL_PROF_STAGES=1`: every stage's footprint and live heap, in order.
                if std::env::var_os("RERAIL_PROF_STAGES").is_some() {
                    let _ = tikv_jemalloc_ctl::epoch::advance();
                    eprintln!(
                        "stage {stage}: footprint {} MB, heap live {} MB, heap resident {} MB, Metal buffers {} MB, driver {} MB",
                        info.ri_phys_footprint >> 20,
                        tikv_jemalloc_ctl::stats::allocated::read().unwrap_or(0) >> 20,
                        tikv_jemalloc_ctl::stats::resident::read().unwrap_or(0) >> 20,
                        crate::code_rule_dsl::transducer::packed::metal_backend::reserved_mb(),
                        crate::code_rule_dsl::transducer::packed::metal_backend::driver_mb()
                    );
                }
            }
        }
        // The system allocator's (SQLite's, the Metal runtime's): in use, and held.
        #[cfg(target_os = "macos")]
        {
            #[repr(C)]
            #[derive(Default)]
            struct Statistics {
                blocks_in_use: libc::c_uint,
                size_in_use: libc::size_t,
                max_size_in_use: libc::size_t,
                size_allocated: libc::size_t,
            }
            unsafe extern "C" {
                fn malloc_zone_statistics(zone: *mut std::ffi::c_void, statistics: *mut Statistics);
            }
            let mut statistics = Statistics::default();
            unsafe { malloc_zone_statistics(std::ptr::null_mut(), &mut statistics) };
            let entry = stages.entry(format!("sys_live_mb.{stage}")).or_default();
            entry.1 = entry.1.max(statistics.size_in_use >> 20);
            let entry = stages.entry(format!("sys_held_mb.{stage}")).or_default();
            entry.1 = entry.1.max(statistics.size_allocated >> 20);
        }
        // What the heap holds live (jemalloc's allocated bytes), against what it keeps mapped.
        #[cfg(not(any(windows, target_family = "wasm")))]
        {
            let _ = tikv_jemalloc_ctl::epoch::advance();
            let entry = stages.entry(format!("live_mb.{stage}")).or_default();
            entry.1 = entry.1.max(tikv_jemalloc_ctl::stats::allocated::read().unwrap_or(0) >> 20);
        }
    }
}

/// The process's physical footprint now, in MB (macOS; 0 elsewhere).
/// What the heap holds live, in MB (jemalloc's allocated bytes); 0 where the system allocator runs (Windows, wasm).
pub(in crate::code_rule_dsl) fn live_heap_mb() -> usize {
    #[cfg(not(any(windows, target_family = "wasm")))]
    {
        let _ = tikv_jemalloc_ctl::epoch::advance();
        tikv_jemalloc_ctl::stats::allocated::read().unwrap_or(0) >> 20
    }
    #[cfg(any(windows, target_family = "wasm"))]
    0
}

pub(in crate::code_rule_dsl) fn footprint_mb() -> usize {
    #[cfg(target_os = "macos")]
    {
        let mut info = unsafe { std::mem::zeroed::<libc::rusage_info_v2>() };
        let got = unsafe { libc::proc_pid_rusage(std::process::id() as i32, libc::RUSAGE_INFO_V2, (&raw mut info).cast()) };
        if got == 0 {
            return (info.ri_phys_footprint >> 20) as usize;
        }
    }
    0
}

/// The process's footprint high-water mark, in MB (0 off macOS).
pub(in crate::code_rule_dsl) fn max_footprint_mb() -> usize {
    #[cfg(target_os = "macos")]
    {
        let mut info = unsafe { std::mem::zeroed::<libc::rusage_info_v4>() };
        let got = unsafe { libc::proc_pid_rusage(std::process::id() as i32, libc::RUSAGE_INFO_V4, (&raw mut info).cast()) };
        if got == 0 {
            return (info.ri_lifetime_max_phys_footprint >> 20) as usize;
        }
    }
    0
}
