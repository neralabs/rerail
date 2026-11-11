//! CPU time by stage, for profiling (O28): with `RERAIL_PROF` set, `time` adds each call's CPU
//! time (user + sys of the thread that runs it, `thread_cpu`) to its stage and `report` prints and
//! clears them; without it, `time` only calls. CPU, not wall time: it does not grow with the
//! machine's load. A stage called on several threads sums their CPU; work a stage hands to other
//! threads is theirs. A top-level stage also records the process's CPU (`cpu_ms.<stage>`, as its
//! calls), which counts every thread. Stages may nest, so their times overlap and need not sum. A
//! crate of its own so that the engine's crates (the Metal runtime's) count into the same stages.

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

static ON: LazyLock<bool> = LazyLock::new(|| std::env::var_os("RERAIL_PROF").is_some());
static STAGES: Mutex<BTreeMap<String, (Duration, usize)>> = Mutex::new(BTreeMap::new());

pub fn time<T>(stage: &str, f: impl FnOnce() -> T) -> T {
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
pub fn single_thread_cpu() -> Duration {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed()
}

/// The process's CPU time so far (user + sys).
#[cfg(target_family = "wasm")]
fn process_cpu() -> Duration {
    single_thread_cpu()
}

/// The calling thread's CPU time so far (user + sys).
#[cfg(unix)]
pub fn thread_cpu() -> Duration {
    let mut time = unsafe { std::mem::zeroed::<libc::timespec>() };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) };
    Duration::new(time.tv_sec as u64, time.tv_nsec as u32)
}

/// The calling thread's CPU time on wasm32: its one thread's (`single_thread_cpu`).
#[cfg(target_family = "wasm")]
pub fn thread_cpu() -> Duration {
    single_thread_cpu()
}

/// The calling thread's CPU time so far (user + kernel).
#[cfg(windows)]
pub fn thread_cpu() -> Duration {
    let (user, kernel) = windows_cpu(true);
    user + kernel
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
pub fn windows_cpu(thread: bool) -> (Duration, Duration) {
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
pub fn windows_peak_working_set() -> u64 {
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

pub fn report(title: &str) {
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
pub struct Stage(Option<(&'static str, Duration, Option<Duration>)>);

impl Stage {
    pub fn start(stage: &'static str) -> Self {
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
pub fn count(stage: &'static str, amount: usize) {
    if *ON {
        let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        stages.entry(stage.to_owned()).or_default().1 += amount;
    }
}

/// Whether stages are timed (`RERAIL_PROF`).
pub fn on() -> bool {
    *ON
}

/// `count` with a name made at run time.
pub fn count_named(stage: String, amount: usize) {
    if *ON {
        let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        stages.entry(stage).or_default().1 += amount;
    }
}

/// The stages, locked: for a reading of its own (the engine's memory peaks), kept as the
/// largest seen.
pub fn stages() -> std::sync::MutexGuard<'static, BTreeMap<String, (Duration, usize)>> {
    STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
