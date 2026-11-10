//! CPU time by stage, for profiling the wave (O28): with `RERAIL_PROF` set, `time` adds each
//! call's CPU time (user + sys of the thread that runs it, `thread_cpu`) to its stage and `report`
//! prints and clears them; without it, `time` only calls. CPU, not wall time: it does not grow
//! with the machine's load. A stage called on several threads sums their CPU; work a stage hands
//! to other threads is theirs (a parallel stage's own calls inside it count it). A top-level
//! stage also records the process's CPU (`cpu_ms.<stage>`, as its calls), which counts every
//! thread. Stages may nest, so their times overlap and need not sum.

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

static ON: LazyLock<bool> = LazyLock::new(|| std::env::var_os("RERAIL_PROF").is_some());
static STAGES: Mutex<BTreeMap<String, (Duration, usize)>> = Mutex::new(BTreeMap::new());

pub(in crate::code_rule_dsl) fn time<T>(stage: &str, f: impl FnOnce() -> T) -> T {
    if !*ON {
        return f();
    }
    let start = thread_cpu();
    // A top-level stage's process CPU (user + sys) as well: its own where nothing overlaps it.
    let top = stage.starts_with("slice.") || stage.starts_with("demand.") || stage.starts_with("labels.");
    let cpu = top.then(process_cpu);
    let out = f();
    let elapsed = thread_cpu().saturating_sub(start);
    let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cpu) = cpu {
        let entry = stages.entry(format!("cpu_ms.{stage}")).or_default();
        entry.1 += ((process_cpu() - cpu).as_secs_f64() * 1e3) as usize;
    }
    let entry = stages.entry(stage.to_owned()).or_default();
    entry.0 += elapsed;
    entry.1 += 1;
    out
}

/// CPU time on wasm32 (the browser build): the engine runs on one thread of one Web Worker, which holds no
/// CPU clock, so the time since the first read stands in for user + sys CPU.
#[cfg(target_family = "wasm")]
pub(crate) fn single_thread_cpu() -> Duration {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed()
}

/// The process's CPU time so far (user + sys).
#[cfg(target_family = "wasm")]
fn process_cpu() -> Duration {
    single_thread_cpu()
}

/// The calling thread's CPU time so far (user + sys).
#[cfg(not(target_family = "wasm"))]
fn thread_cpu() -> Duration {
    let mut time = unsafe { std::mem::zeroed::<libc::timespec>() };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) };
    Duration::new(time.tv_sec as u64, time.tv_nsec as u32)
}

/// The calling thread's CPU time on wasm32: its one thread's (`single_thread_cpu`).
#[cfg(target_family = "wasm")]
fn thread_cpu() -> Duration {
    single_thread_cpu()
}

/// The process's CPU time so far (user + sys).
#[cfg(unix)]
fn process_cpu() -> Duration {
    let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let time = |t: libc::timeval| Duration::from_secs(t.tv_sec as u64) + Duration::from_micros(t.tv_usec as u64);
    time(usage.ru_utime) + time(usage.ru_stime)
}

/// The process's CPU time so far (user + kernel).
#[cfg(windows)]
fn process_cpu() -> Duration {
    let (user, kernel) = windows_cpu(false);
    user + kernel
}

/// Windows: the (user, kernel) CPU time of the calling thread (`thread`) or of the process, from
/// `GetThreadTimes` / `GetProcessTimes`; zero if the call fails.
#[cfg(windows)]
pub(crate) fn windows_cpu(thread: bool) -> (Duration, Duration) {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread, GetProcessTimes, GetThreadTimes};
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the pseudo-handles name the calling thread and process, and every out-pointer is a FILETIME.
    let got = unsafe {
        if thread {
            GetThreadTimes(GetCurrentThread(), &mut created, &mut exited, &mut kernel, &mut user)
        } else {
            GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user)
        }
    };
    if got == 0 {
        return (Duration::ZERO, Duration::ZERO);
    }
    // FILETIME counts 100 ns ticks.
    let time = |t: FILETIME| Duration::from_nanos(((u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)) * 100);
    (time(user), time(kernel))
}

/// Windows: the process's peak working set (its resident high-water mark), in bytes; 0 if the call fails.
#[cfg(windows)]
pub(crate) fn windows_peak_working_set() -> u64 {
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: PROCESS_MEMORY_COUNTERS is plain integers, so all-zero is valid.
    let mut counters = unsafe { std::mem::zeroed::<PROCESS_MEMORY_COUNTERS>() };
    counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: the pseudo-handle names this process, and `cb` is the size of the struct written.
    if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) } == 0 {
        return 0;
    }
    counters.PeakWorkingSetSize as u64
}

/// Prints the stages' times under `title` and clears them.
#[cfg_attr(not(test), allow(dead_code))]
pub(in crate::code_rule_dsl) fn report(title: &str) {
    if !*ON {
        return;
    }
    let stages = std::mem::take(&mut *STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
    eprintln!("prof {title}:");
    for (stage, (elapsed, calls)) in stages {
        eprintln!("  {stage:<40} {:>10.1} ms {calls:>7} calls", elapsed.as_secs_f64() * 1e3);
    }
}

/// A stage timed until it drops.
pub(in crate::code_rule_dsl) struct Stage(Option<(&'static str, Duration, Option<Duration>)>);

impl Stage {
    pub(in crate::code_rule_dsl) fn start(stage: &'static str) -> Self {
        Self(ON.then(|| (stage, thread_cpu(), (stage.starts_with("labels.") || stage.starts_with("demand.") || stage == "size.texts").then(process_cpu))))
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if let Some((stage, start, cpu)) = self.0.take() {
            let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(cpu) = cpu {
                stages.entry(format!("cpu_ms.{stage}")).or_default().1 += ((process_cpu() - cpu).as_secs_f64() * 1e3) as usize;
            }
            let entry = stages.entry(stage.to_owned()).or_default();
            entry.0 += thread_cpu().saturating_sub(start);
            entry.1 += 1;
        }
    }
}

/// Adds `amount` (bytes, records) to `stage`'s count, shown as its calls.
pub(in crate::code_rule_dsl) fn count(stage: &'static str, amount: usize) {
    if *ON {
        let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        stages.entry(stage.to_owned()).or_default().1 += amount;
    }
}

/// Whether stages are timed (`RERAIL_PROF`).
pub(in crate::code_rule_dsl) fn on() -> bool {
    *ON
}

/// `count` with a name made at run time.
pub(in crate::code_rule_dsl) fn count_named(stage: String, amount: usize) {
    if *ON {
        let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        stages.entry(stage).or_default().1 += amount;
    }
}

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
    if *ON {
        // No resident-set counter on wasm32: its peak reads 0.
        #[cfg(target_family = "wasm")]
        let mb = 0;
        #[cfg(unix)]
        let mb = {
            let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
            unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
            // macOS reports bytes; Linux, kilobytes.
            if cfg!(target_os = "macos") { usage.ru_maxrss as usize >> 20 } else { usage.ru_maxrss as usize >> 10 }
        };
        #[cfg(windows)]
        let mb = (windows_peak_working_set() >> 20) as usize;
        let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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
