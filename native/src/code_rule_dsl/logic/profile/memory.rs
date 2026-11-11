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

/// The footprint's high-water mark as a span reads it at its open and close: read again only when a millisecond
/// has passed since the last read, else the last value. A read is a system call (about a microsecond, more on a
/// loaded machine) and a run opens and closes millions of tiny spans; between two reads the mark can rise only by
/// what a millisecond allocates, which the next read charges to the span then open.
pub(super) fn peak_sampled() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::LazyLock;
    static START: LazyLock<std::time::Instant> = LazyLock::new(std::time::Instant::now);
    static LAST: AtomicU64 = AtomicU64::new(u64::MAX);
    static PEAK: AtomicU64 = AtomicU64::new(0);
    let now = START.elapsed().as_micros() as u64;
    let last = LAST.load(Ordering::Relaxed);
    if last != u64::MAX && now.saturating_sub(last) < 1000 {
        return PEAK.load(Ordering::Relaxed);
    }
    LAST.store(now, Ordering::Relaxed);
    let peak = footprint().1;
    PEAK.fetch_max(peak, Ordering::Relaxed);
    PEAK.load(Ordering::Relaxed)
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

/// The sampler: a thread that reads the footprint, its high-water mark and jemalloc's heap every few milliseconds
/// (`RERAIL_PROF_MEMORY_MS`, 5 by default) while profiling, for as long as the process lives. A span's own rise
/// (above) misses a peak no span is open around (a run's end, a caller's work after it) and cannot say which thread
/// raised it; the sampler charges each rise of the high-water mark to the spans open on every thread when it saw
/// the rise, and keeps a timeline (`memory.tsv`). A peak between two samples is still counted (the high-water mark
/// is the system's), only its charge is shared by what was open in that interval's end.
#[derive(Clone, Default)]
pub(super) struct Sample {
    /// Seconds since the sampler started, and the wall clock then (Unix seconds), to line samples up with another
    /// tool's (`sample`, a watcher's).
    pub(super) at: f64,
    pub(super) unix: f64,
    pub(super) footprint: u64,
    pub(super) max: u64,
    pub(super) live: u64,
    pub(super) resident: u64,
    /// Each thread's innermost open span, as `thread: path`, and how many threads had opened a span (live).
    pub(super) open: Vec<String>,
    pub(super) threads: usize,
}

#[derive(Default)]
pub(super) struct Sampled {
    pub(super) every_ms: u64,
    /// The timeline: a sample when the footprint moved 8 MB from the last one kept, or 250 ms passed.
    pub(super) rows: Vec<Sample>,
    /// The sample with the highest footprint seen.
    pub(super) top: Sample,
    /// Bytes the high-water mark rose while each `thread: path` was open, and the most footprint seen while it was.
    pub(super) rise: std::collections::BTreeMap<String, (u64, u64)>,
}

pub(super) static SAMPLED: std::sync::LazyLock<std::sync::Mutex<Sampled>> = std::sync::LazyLock::new(Default::default);

/// Starts the sampler, once per process.
pub(super) fn sample() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let every_ms = std::env::var("RERAIL_PROF_MEMORY_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(5u64).max(1);
        SAMPLED.lock().unwrap().every_ms = every_ms;
        let _ = std::thread::Builder::new().name("profile memory".into()).spawn(move || {
            let start = std::time::Instant::now();
            let mut last_max = footprint().1;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(every_ms));
                let (now, max) = footprint();
                let (live, resident) = heap();
                let at = start.elapsed().as_secs_f64();
                let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
                let mut s = SAMPLED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let keep = s.rows.last().is_none_or(|l| now.abs_diff(l.footprint) >= 8 << 20 || at - l.at >= 0.25);
                let rose = max.saturating_sub(last_max);
                let top = now > s.top.footprint;
                if !(keep || rose > 0 || top) {
                    continue;
                }
                let (mut open, threads) = super::open_paths();
                open.extend(doing());
                for p in &open {
                    let e = s.rise.entry(p.clone()).or_default();
                    e.0 += rose;
                    e.1 = e.1.max(now);
                }
                last_max = max;
                let sample = Sample { at, unix, footprint: now, max, live, resident, open, threads };
                if top {
                    s.top = sample.clone();
                }
                if keep {
                    s.rows.push(sample);
                }
            }
        });
    });
}

/// Work the sampler names that runs in no span (the roofline's loops, run while `write` holds the profile's
/// records): set while the guard lives.
pub(super) struct Doing(&'static str);

static DOING: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());

impl Doing {
    pub(super) fn new(what: &'static str) -> Self {
        DOING.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(what);
        Doing(what)
    }
}

impl Drop for Doing {
    fn drop(&mut self) {
        let mut d = DOING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(i) = d.iter().position(|w| *w == self.0) {
            d.remove(i);
        }
    }
}

/// What `Doing` guards name now.
fn doing() -> Vec<String> {
    DOING.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().map(|w| (*w).to_owned()).collect()
}

/// The sampler's findings, for a report: its peak and what was open then, and the spans open while the high-water
/// mark rose. Empty when it never ran.
pub(super) fn sampled_report(r: &mut String) {
    use std::fmt::Write as _;
    let s = SAMPLED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if s.every_ms == 0 {
        return;
    }
    let t = &s.top;
    let _ = writeln!(
        r,
        "Sampled every {} ms: the most footprint seen {} MB at {:.2} s (jemalloc {} MB live, {} MB resident); the high-water mark then {} MB. Open then:",
        s.every_ms,
        mb(t.footprint as f64),
        t.at,
        mb(t.live as f64),
        mb(t.resident as f64),
        mb(t.max as f64)
    );
    for p in &t.open {
        let _ = writeln!(r, "    {}", super::short(p, 160));
    }
    let mut rose: Vec<(&String, &(u64, u64))> = s.rise.iter().filter(|(_, v)| v.0 > 0).collect();
    rose.sort_by_key(|(_, v)| std::cmp::Reverse(v.0));
    let _ = writeln!(r, "Open (innermost, on any thread) while the high-water mark rose, by the rise; most footprint seen while open");
    for (p, (rise, most)) in rose.iter().take(20) {
        let _ = writeln!(r, "{:>10} MB {:>10} MB  {}", mb(*rise as f64), mb(*most as f64), super::short(p, 140));
    }
}

/// The sampler's timeline as TSV.
pub(super) fn sampled_tsv() -> String {
    use std::fmt::Write as _;
    let s = SAMPLED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut t = String::from("at_s\tunix_s\tfootprint_bytes\tmax_bytes\tjemalloc_live_bytes\tjemalloc_resident_bytes\tthreads\topen\n");
    for r in &s.rows {
        let _ = writeln!(t, "{:.3}\t{:.3}\t{}\t{}\t{}\t{}\t{}\t{}", r.at, r.unix, r.footprint, r.max, r.live, r.resident, r.threads, r.open.join(" | "));
    }
    t
}
