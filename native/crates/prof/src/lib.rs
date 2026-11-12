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

// The workspace profile (`RERAIL_PROFILE`) reports the stages too (its "Stages" section).
static ON: LazyLock<bool> = LazyLock::new(|| std::env::var_os("RERAIL_PROF").is_some() || std::env::var_os("RERAIL_PROFILE").is_some());
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

/// Whether stages are timed (`RERAIL_PROF`, or the workspace profile's `RERAIL_PROFILE`).
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

/// A sampling heap profiler (`RERAIL_HEAP_SAMPLE`): which spans allocated the heap that is live at the footprint's
/// peak. The engine's allocator calls `alloc` and `free` around jemalloc's; a thread samples an allocation per
/// `INTERVAL` bytes it allocates (every one of `BIG` bytes or more), keyed by its address with the thread's innermost
/// span, and a free of a sampled address removes it. The profile's memory sampler takes `snapshot` at its peak: each
/// live sample weighs the bytes it stands for. Nothing here allocates: the table is a static open-addressed array of
/// atomics, the thread's state const cells. Off, an allocation pays one load.
pub mod heap {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering::Relaxed};

    /// Bytes between samples, on average.
    pub const INTERVAL: u64 = 256 << 10;
    /// Allocations this large are all sampled, at their size.
    const BIG: usize = 64 << 10;
    const SLOTS: usize = 1 << 19;
    const EMPTY: usize = 0;
    const GONE: usize = 1;
    const PROBES: usize = 64;

    static STATE: AtomicU8 = AtomicU8::new(0);
    static KEYS: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(EMPTY) }; SLOTS];
    static SIZES: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
    static SPANS: [AtomicU32; SLOTS] = [const { AtomicU32::new(0) }; SLOTS];
    /// Samples the table had no place for (a probe past `PROBES`).
    pub static DROPPED: AtomicU64 = AtomicU64::new(0);

    thread_local! {
        static SPAN: Cell<u32> = const { Cell::new(0) };
        static LEFT: Cell<i64> = const { Cell::new(INTERVAL as i64) };
    }

    /// Whether heap sampling is on (`RERAIL_HEAP_SAMPLE` set): read once, without allocating.
    #[inline]
    pub fn on() -> bool {
        match STATE.load(Relaxed) {
            2 => true,
            1 => false,
            _ => {
                #[cfg(unix)]
                let set = !unsafe { libc::getenv(c"RERAIL_HEAP_SAMPLE".as_ptr()) }.is_null();
                #[cfg(not(unix))]
                let set = false;
                STATE.store(if set { 2 } else { 1 }, Relaxed);
                set
            }
        }
    }

    /// The span this thread's allocations are charged to (`intern`'s id; 0 none).
    pub fn set_span(id: u32) {
        let _ = SPAN.try_with(|s| s.set(id));
    }

    fn slot(ptr: usize) -> usize {
        (ptr.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 13) & (SLOTS - 1)
    }

    /// The allocators a sample can name beside Rust's (`alloc_by`): a high bit of its span id.
    pub const SQLITE: u32 = 1 << 31;
    pub const TREE_SITTER: u32 = 1 << 30;

    /// An allocation of `size` bytes at `ptr`: sampled when its thread's countdown runs out, or when big.
    #[inline]
    pub fn alloc(ptr: usize, size: usize) {
        alloc_by(ptr, size, 0)
    }

    /// `alloc` by another allocator than Rust's (`SQLITE`, `TREE_SITTER`).
    #[inline]
    pub fn alloc_by(ptr: usize, size: usize, by: u32) {
        if ptr == 0 {
            return;
        }
        if size < BIG {
            let Ok(sampled) = LEFT.try_with(|left| {
                let l = left.get() - size as i64;
                if l > 0 {
                    left.set(l);
                    return false;
                }
                // The next sample a varying distance on (a cheap hash of the address), so strided sizes do not alias.
                left.set(l + INTERVAL as i64 / 2 + (slot(ptr) as i64 % INTERVAL as i64));
                true
            }) else { return };
            if !sampled {
                return;
            }
        }
        let span = SPAN.try_with(Cell::get).unwrap_or(0) | by;
        let mut at = slot(ptr);
        for _ in 0..PROBES {
            let k = KEYS[at].load(Relaxed);
            if (k == EMPTY || k == GONE) && KEYS[at].compare_exchange(k, ptr, Relaxed, Relaxed).is_ok() {
                SIZES[at].store(size as u64, Relaxed);
                SPANS[at].store(span, Relaxed);
                return;
            }
            at = (at + 1) & (SLOTS - 1);
        }
        DROPPED.fetch_add(1, Relaxed);
    }

    /// A free of `ptr`: its sample, if any, removed.
    #[inline]
    pub fn free(ptr: usize) {
        let mut at = slot(ptr);
        for _ in 0..PROBES {
            let k = KEYS[at].load(Relaxed);
            if k == EMPTY {
                return;
            }
            if k == ptr {
                KEYS[at].store(GONE, Relaxed);
                return;
            }
            at = (at + 1) & (SLOTS - 1);
        }
    }

    /// The live samples' bytes by span id: a sample of `size` stands for `size` bytes when big, else for
    /// `size / (1 - e^(-size / INTERVAL))` (the expected bytes a sample of a byte-countdown stands for).
    pub fn snapshot() -> Vec<(u32, u64)> {
        let mut by = std::collections::HashMap::<u32, f64>::new();
        for at in 0..SLOTS {
            let k = KEYS[at].load(Relaxed);
            if k == EMPTY || k == GONE {
                continue;
            }
            let size = SIZES[at].load(Relaxed) as f64;
            let weight = if size >= BIG as f64 { size } else { size / (1.0 - (-size / INTERVAL as f64).exp()).max(1e-9) };
            *by.entry(SPANS[at].load(Relaxed)).or_default() += weight;
        }
        let mut v: Vec<(u32, u64)> = by.into_iter().map(|(k, w)| (k, w as u64)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v
    }

    type Names = (std::collections::HashMap<String, u32>, Vec<String>);
    static NAMES: std::sync::LazyLock<std::sync::Mutex<Names>> = std::sync::LazyLock::new(Default::default);

    /// A span path's id (1 on), made once.
    pub fn intern(path: &str) -> u32 {
        let mut names = NAMES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(&id) = names.0.get(path) {
            return id;
        }
        names.1.push(path.to_owned());
        let id = names.1.len() as u32;
        names.0.insert(path.to_owned(), id);
        id
    }

    /// The path of span id `id` (0: none open).
    pub fn name(id: u32) -> String {
        if id & SQLITE != 0 {
            return format!("SQLite's, in {}", name(id & !SQLITE));
        }
        if id & TREE_SITTER != 0 {
            return format!("Tree-sitter's, in {}", name(id & !TREE_SITTER));
        }
        if id == 0 {
            return "(no span open)".to_owned();
        }
        NAMES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).1.get(id as usize - 1).cloned().unwrap_or_default()
    }
}

// cratify: mounts (tools/cratify/mounts.py; generated, do not edit) begin
extern crate self as rerail_prof;
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    #[path = "logic"]
    pub mod logic {
        #[path = "front"]
        pub mod front {
            pub use rerail_rr_syntax::code_rule_dsl::logic::front::ast;
            pub use rerail_rr_syntax::code_rule_dsl::logic::front::name;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::fmt;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::name::Name;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::ast::{AXIS, Again, AggOp, Atom, DESCENDANTS, EVERY_KIND, GLOBAL, LIBRARY, LatticeDecl, Literal, NEAREST, Native, OPTION_GLOB, Program, QUALIFIERS, RESERVED, Rule, SCOPE_NOT, SpanIn, Term, Value, axis, cmp_binds, globals, is_axis, is_transduced, join};
        }
        pub mod profile;
    }
    pub mod phase_clock;
    pub mod prof;
    #[allow(unused_imports)]
    use napi::Result;
    #[allow(unused_imports)]
    use napi_derive::napi;
    #[allow(unused_imports)]
    use serde_json::Value;
    #[allow(unused_imports)]
    use std::sync::Arc;
}
// cratify: mounts end
