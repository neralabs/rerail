//! CPU time by stage, for profiling (O28): with `RERAIL_PROF` set, `time` adds each call's CPU
//! time (user + sys of the thread that runs it, `thread_cpu`) to its stage and `report` prints and
//! clears them; without it, `time` only calls. CPU, not wall time: it does not grow with the
//! machine's load. A stage called on several threads sums their CPU; work a stage hands to other
//! threads is theirs. A top-level stage also records the process's CPU (`cpu_ms.<stage>`, as its
//! calls), which counts every thread. Stages may nest, so their times overlap and need not sum. A
//! crate of its own so that the engine's crates (the Metal runtime's) count into the same stages.

use std::collections::BTreeMap;
use std::time::Duration;

// The workspace profile (`RERAIL_PROFILE`) reports the stages too (its "Stages" section).
/// Whether stages are timed (`RERAIL_PROF`, `RERAIL_PROFILE`): read once a run (decision 0442).
fn stages_on() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_PROF").is_some() || std::env::var_os("RERAIL_PROFILE").is_some())
}
/// The run's stages (decision 0442: a run's measurement), by name: CPU time and calls.
#[derive(Default)]
struct Stages(std::sync::Arc<parking_lot::Mutex<BTreeMap<String, (Duration, usize)>>>);

fn held() -> parking_lot::ArcMutexGuard<parking_lot::RawMutex, BTreeMap<String, (Duration, usize)>> {
    rerail_context::local::<Stages>().0.lock_arc()
}

pub fn time<T>(stage: &str, f: impl FnOnce() -> T) -> T {
    if !stages_on() {
        return f();
    }
    let start = thread_cpu();
    // A top-level stage's process CPU (user + sys) as well: its own where nothing overlaps it.
    let top = stage.starts_with("slice.") || stage.starts_with("demand.") || stage.starts_with("labels.");
    let cpu = top.then(process_cpu);
    let out = f();
    let elapsed = thread_cpu().saturating_sub(start);
    let mut stages = held();
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
pub fn process_cpu() -> Duration {
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
pub fn process_cpu() -> Duration {
    let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let time = |t: libc::timeval| Duration::from_secs(t.tv_sec as u64) + Duration::from_micros(t.tv_usec as u64);
    time(usage.ru_utime) + time(usage.ru_stime)
}

/// The process's CPU time so far (user + kernel).
#[cfg(windows)]
pub fn process_cpu() -> Duration {
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
    if !stages_on() {
        return;
    }
    let stages = std::mem::take(&mut *held());
    eprintln!("prof {title}:");
    for (stage, (elapsed, calls)) in stages {
        eprintln!("  {stage:<40} {:>10.1} ms {calls:>7} calls", elapsed.as_secs_f64() * 1e3);
    }
}

/// A stage timed until it drops.
pub struct Stage(Option<(&'static str, Duration, Option<Duration>)>);

impl Stage {
    pub fn start(stage: &'static str) -> Self {
        Self(stages_on().then(|| (stage, thread_cpu(), (stage.starts_with("labels.") || stage.starts_with("demand.") || stage == "size.texts").then(process_cpu))))
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if let Some((stage, start, cpu)) = self.0.take() {
            let mut stages = held();
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
    if stages_on() {
        let mut stages = held();
        stages.entry(stage.to_owned()).or_default().1 += amount;
    }
}

/// Whether stages are timed (`RERAIL_PROF`, or the workspace profile's `RERAIL_PROFILE`).
pub fn on() -> bool {
    stages_on()
}

/// `count` with a name made at run time.
pub fn count_named(stage: String, amount: usize) {
    if stages_on() {
        let mut stages = held();
        stages.entry(stage).or_default().1 += amount;
    }
}

/// The stages, locked: for a reading of its own (the engine's memory peaks), kept as the
/// largest seen.
pub fn stages() -> parking_lot::ArcMutexGuard<parking_lot::RawMutex, BTreeMap<String, (Duration, usize)>> {
    held()
}

/// The copies decision 0353 still lists, counted where each is made: the measure its steps drive to 0. Each site is
/// one `static` (`copies::Site`), named by its row of 0353's table ("The copies today") and the step that removes it,
/// and marked `copy (0353 step N)` beside it. Bytes and calls are always counted (two relaxed adds); the CPU a copy
/// takes is timed only when stages are (`on`). A site joins the list at its first copy, so the profile's heading
/// `copies (decision 0353)` names every site that copied in the run, and a site absent from it copied nothing.
pub mod copies {
    use std::sync::Mutex;

    /// A site of a copy decision 0353 lists: code, its row and step fixed where it is written. What it copied is the
    /// run's (`Copies`, decision 0442): a run's profile names the copies that run made.
    pub struct Site {
        /// Its row in 0353's table.
        pub row: u8,
        /// The step of 0353's plan that removes it.
        pub step: &'static str,
        /// What is copied, and where.
        pub name: &'static str,
    }

    /// A site's totals so far.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Totals {
        pub bytes: u64,
        pub calls: u64,
        pub cpu_ns: u64,
    }

    /// The run's copies, by site (`rerail_context::local`; a site is its row and name, a `const` written once):
    /// each site that copied, in the order it first did.
    #[derive(Default)]
    struct Copies(Mutex<Vec<(&'static Site, Totals)>>);

    impl Copies {
        fn add(&self, site: &'static Site, bytes: u64, cpu_ns: u64) {
            let mut all = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let at = match all.iter().position(|(s, _)| (s.row, s.name) == (site.row, site.name)) {
                Some(at) => at,
                None => {
                    all.push((site, Totals::default()));
                    all.len() - 1
                }
            };
            let t = &mut all[at].1;
            t.bytes += bytes;
            t.calls += 1;
            t.cpu_ns += cpu_ns;
        }
    }

    impl Site {
        pub const fn new(row: u8, step: &'static str, name: &'static str) -> Self {
            Self { row, step, name }
        }

        fn count(&'static self, bytes: usize, cpu_ns: u64) {
            rerail_context::with_local::<Copies, _>(|c| c.add(self, bytes as u64, cpu_ns));
        }

        /// One copy of `bytes`, its CPU not timed (a copy inside a timed one, or too small to time).
        pub fn copied(&'static self, bytes: usize) {
            self.count(bytes, 0);
        }

        /// One copy made by `f`, which gives its bytes with its value; its CPU timed when stages are.
        pub fn time<T>(&'static self, f: impl FnOnce() -> (T, usize)) -> T {
            let start = super::on().then(super::thread_cpu);
            let (out, bytes) = f();
            let cpu = start.map_or(0, |start| super::thread_cpu().saturating_sub(start).as_nanos() as u64);
            self.count(bytes, cpu);
            out
        }

        /// A copy begun here, ended by `Copying::done` with its bytes: for a copy spread through a function.
        pub fn start(&'static self) -> Copying {
            Copying { site: self, start: super::on().then(super::thread_cpu) }
        }
    }

    /// A copy under way at a site (`Site::start`).
    #[must_use = "a copy is counted when it is done"]
    pub struct Copying {
        site: &'static Site,
        start: Option<std::time::Duration>,
    }

    impl Copying {
        pub fn done(self, bytes: usize) {
            let cpu = self.start.map_or(0, |start| super::thread_cpu().saturating_sub(start).as_nanos() as u64);
            self.site.count(bytes, cpu);
        }
    }

    /// Every site that copied in the run under way, by row, with its totals.
    pub fn sites() -> Vec<(&'static Site, Totals)> {
        let Some(copies) = rerail_context::local_if_any::<Copies>() else { return Vec::new() };
        let mut sites = copies.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        sites.sort_by_key(|(site, _)| (site.row, site.name));
        sites
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const ONE: Site = Site::new(250, "test", "a test's copy");

        #[test]
        fn a_sites_copies_are_counted_and_listed_once() {
            let _run = rerail_context::test_run();
            ONE.copied(10);
            let made = ONE.time(|| ("made", 32));
            assert_eq!(made, "made");
            let listed: Vec<_> = sites().into_iter().filter(|(site, _)| site.row == 250).collect();
            assert_eq!(listed.len(), 1, "a site is listed once, however often it copies");
            assert_eq!((listed[0].1.bytes, listed[0].1.calls), (42, 2));
        }
    }
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
        /// This thread's number for samples taken with no span open (`THREAD`), 0 until its first.
        static THREAD_IX: Cell<u32> = const { Cell::new(0) };
    }
    /// The last thread number given.
    static THREADS: AtomicU32 = AtomicU32::new(0);
    /// The threads' names by number, as each named itself (`name_thread`).
    static THREAD_NAMES: std::sync::Mutex<Vec<(u32, String)>> = std::sync::Mutex::new(Vec::new());

    /// This thread's number (1 on), given at its first ask without allocating.
    fn thread_ix() -> u32 {
        THREAD_IX
            .try_with(|c| {
                if c.get() == 0 {
                    c.set(THREADS.fetch_add(1, Relaxed) + 1);
                }
                c.get()
            })
            .unwrap_or(0)
    }

    /// Names this thread for its samples taken with no span open (the profile names each thread at its first span).
    pub fn name_thread(name: &str) {
        let ix = thread_ix();
        let mut names = THREAD_NAMES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !names.iter().any(|(i, _)| *i == ix) {
            names.push((ix, name.to_owned()));
        }
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
        // In 64 bits on every target (wasm32's pointers are 32): the same slot for the same address.
        ((ptr as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 13) as usize & (SLOTS - 1)
    }

    /// The allocators a sample can name beside Rust's (`alloc_by`): a high bit of its span id.
    pub const SQLITE: u32 = 1 << 31;
    pub const TREE_SITTER: u32 = 1 << 30;
    /// Device buffers relational work leases (shared memory, outside jemalloc's heap).
    pub const DEVICE: u32 = 1 << 29;
    /// An allocation with no span open: the low bits are its thread's number (`thread_ix`), not a span.
    pub const THREAD: u32 = 1 << 28;

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
        // With no span open, charged to the thread: "(no span open)" alone named none of 494 MB at a 300-file peak.
        let span = match SPAN.try_with(Cell::get).unwrap_or(0) {
            0 => THREAD | thread_ix(),
            s => s,
        } | by;
        let mut at = slot(ptr);
        for _ in 0..PROBES {
            let k = KEYS[at].load(Relaxed);
            if (k == EMPTY || k == GONE) && KEYS[at].compare_exchange(k, ptr, Relaxed, Relaxed).is_ok() {
                SIZES[at].store(size as u64, Relaxed);
                SPANS[at].store(span, Relaxed);
                frames::record(at);
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
        v.sort_by_key(|b| std::cmp::Reverse(b.1));
        v
    }

    /// The live samples' bytes by the span and call stack that allocated them (`RERAIL_HEAP_FRAMES`, with
    /// `RERAIL_HEAP_SAMPLE`): each stack its return addresses, innermost first. A span names the work that allocated
    /// a block, not what holds it after; a stack names the function and so the structure.
    pub fn snapshot_frames() -> Vec<((u32, Vec<usize>), u64)> {
        let mut by = std::collections::HashMap::<(u32, Vec<usize>), f64>::new();
        for at in 0..SLOTS {
            let k = KEYS[at].load(Relaxed);
            if k == EMPTY || k == GONE {
                continue;
            }
            let Some(stack) = frames::of(at) else { continue };
            let size = SIZES[at].load(Relaxed) as f64;
            let weight = if size >= BIG as f64 { size } else { size / (1.0 - (-size / INTERVAL as f64).exp()).max(1e-9) };
            *by.entry((SPANS[at].load(Relaxed), stack)).or_default() += weight;
        }
        let mut v: Vec<((u32, Vec<usize>), u64)> = by.into_iter().map(|(k, w)| (k, w as u64)).collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1));
        v
    }

    /// The main executable's load address, for `atos -l` (0 where it is not known).
    pub fn load_address() -> usize {
        #[cfg(target_os = "macos")]
        {
            unsafe extern "C" {
                fn _dyld_get_image_header(index: u32) -> *const std::ffi::c_void;
            }
            unsafe { _dyld_get_image_header(0) as usize }
        }
        #[cfg(not(target_os = "macos"))]
        {
            0
        }
    }

    /// The allocating stacks (`RERAIL_HEAP_FRAMES`): a frame-pointer walk at each sample, into a table mapped at the
    /// first (no allocation in the allocator), a slot's stack beside its sample.
    mod frames {
        use super::{Relaxed, SLOTS};
        use std::sync::atomic::{AtomicPtr, AtomicU8, AtomicUsize};

        pub(super) const DEPTH: usize = 14;
        /// The allocator's own frames, above the caller's.
        const SKIP: usize = 3;
        static STATE: AtomicU8 = AtomicU8::new(0);
        static TABLE: AtomicPtr<AtomicUsize> = AtomicPtr::new(std::ptr::null_mut());

        fn on() -> bool {
            match STATE.load(Relaxed) {
                2 => true,
                1 => false,
                _ => {
                    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                    let set = !unsafe { libc::getenv(c"RERAIL_HEAP_FRAMES".as_ptr()) }.is_null();
                    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
                    let set = false;
                    STATE.store(if set { 2 } else { 1 }, Relaxed);
                    set
                }
            }
        }

        fn table() -> Option<&'static [AtomicUsize]> {
            if !on() {
                return None;
            }
            let t = TABLE.load(Relaxed);
            let t = if t.is_null() { made()? } else { t };
            Some(unsafe { std::slice::from_raw_parts(t, SLOTS * DEPTH) })
        }

        /// The table, mapped by the first thread to ask (another's, where two race); `None` where it cannot be.
        #[cfg(unix)]
        fn made() -> Option<*mut AtomicUsize> {
            let len = SLOTS * DEPTH * std::mem::size_of::<usize>();
            let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0) };
            if p == libc::MAP_FAILED {
                STATE.store(1, Relaxed);
                return None;
            }
            let p = p.cast::<AtomicUsize>();
            Some(match TABLE.compare_exchange(std::ptr::null_mut(), p, Relaxed, Relaxed) {
                Ok(_) => p,
                Err(other) => {
                    unsafe { libc::munmap(p.cast(), len) };
                    other
                }
            })
        }
        #[cfg(not(unix))]
        fn made() -> Option<*mut AtomicUsize> {
            None
        }

        #[inline(never)]
        pub(super) fn record(at: usize) {
            let Some(t) = table() else { return };
            let row = &t[at * DEPTH..(at + 1) * DEPTH];
            let got = frames();
            for (cell, v) in row.iter().zip(got[SKIP..].iter()) {
                cell.store(*v, Relaxed);
            }
        }

        /// The calling frames' return addresses, innermost first, by the frame pointers' chain. Inlined, so the
        /// frames are `record`'s caller's (`SKIP` drops the profiler's own).
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        #[inline(always)]
        fn frames() -> [usize; DEPTH + SKIP] {
            let mut got = [0usize; DEPTH + SKIP];
            let mut fp: usize;
            unsafe { std::arch::asm!("mov {}, x29", out(reg) fp) };
            let start = fp;
            for slot in got.iter_mut() {
                // A frame record is (the caller's frame pointer, the return address); the stack grows down, so each
                // caller's record is higher, and a walk out of the stack stops.
                if fp == 0 || !fp.is_multiple_of(8) || fp < start || fp > start + (64 << 20) {
                    break;
                }
                let (next, ret) = unsafe { (*(fp as *const usize), *((fp + 8) as *const usize)) };
                *slot = ret & 0x0000_ffff_ffff_ffff;
                if next <= fp {
                    break;
                }
                fp = next;
            }
            got
        }

        /// Elsewhere no walk: no frames.
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        fn frames() -> [usize; DEPTH + SKIP] {
            [0; DEPTH + SKIP]
        }

        pub(super) fn of(at: usize) -> Option<Vec<usize>> {
            let t = table()?;
            let row = &t[at * DEPTH..(at + 1) * DEPTH];
            Some(row.iter().map(|c| c.load(Relaxed)).take_while(|v| *v != 0).collect())
        }
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
        if id & DEVICE != 0 {
            return format!("device leases, in {}", name(id & !DEVICE));
        }
        if id & TREE_SITTER != 0 {
            return format!("Tree-sitter's, in {}", name(id & !TREE_SITTER));
        }
        if id & THREAD != 0 {
            let ix = id & !THREAD;
            let names = THREAD_NAMES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            return match names.iter().find(|(i, _)| *i == ix) {
                Some((_, n)) => format!("(no span open) on {n}"),
                None => format!("(no span open) on thread #{ix}, unnamed (it opened no span)"),
            };
        }
        if id == 0 {
            return "(no span open)".to_owned();
        }
        NAMES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).1.get(id as usize - 1).cloned().unwrap_or_default()
    }
}

// The browser build has no JIT: wasm/cranelift-jit stands in under the same name (wasm/README.md).
#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;
// cratify: mounts (tools/cratify/mounts.py; generated, do not edit) begin
extern crate self as rerail_prof;
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    #[path = "logic"]
    pub mod logic {
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
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::ast::{AXIS, AggOp, DESCENDANTS, EVERY_KIND, GLOBAL, LIBRARY, LatticeDecl, NEAREST, Native, OPTION_GLOB, QUALIFIERS, RESERVED, SCOPE_NOT, SpanIn, Value, axis, globals, is_axis, is_transduced};
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
