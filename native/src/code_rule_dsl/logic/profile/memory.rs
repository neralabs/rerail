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

/// jemalloc's live bytes: the heap's part of the run's count (`rerail_accel::budget::set_heap_reader`, decision 0435).
pub fn heap_live() -> usize {
    heap().0 as usize
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
    /// What else holds memory then, beside the heap (MB): the Metal driver's allocations, the device buffer pool's
    /// reservations, relational work's leases (live, idle), SQLite's heap, the host blocks (used, and of it free
    /// on their lists; decision 0342).
    pub(super) holders: [usize; 7],
}

/// The holders a sample reads (`Sample::holders`), in MB.
fn holders() -> [usize; 7] {
    let (reserved, driver) = crate::code_rule_dsl::prof::device_memory();
    // Relational work's device leases exist only where the device does (macOS).
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    let (live, idle) = rerail_accel::gpu::shared::held_bytes();
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    let (live, idle) = (0, 0);
    let sqlite = unsafe { rusqlite::ffi::sqlite3_memory_used() }.max(0) as usize;
    let (host, host_free) = rerail_accel::blocks::host_held();
    [driver, reserved, live >> 20, idle >> 20, sqlite >> 20, host >> 20, host_free >> 20]
}

/// The relations' blocks live now by the span that made them (span, life, bytes used, blocks), as the facts crate
/// reads them (`rows::held_by_span`), which sets it at its first block: the sampler takes it at the peak.
#[derive(Default)]
pub struct BlocksBySpan(pub std::sync::OnceLock<fn() -> Vec<(u32, u8, usize, usize, String)>>);

/// The runs' relations held now by name (name, rows, columns, bytes of values, runs holding it), as the evaluator
/// reads them (`setfix::held_by_name`), which sets it at its first run: the sampler takes it at the peak.
#[derive(Default)]
pub struct RelationsByName(pub std::sync::OnceLock<fn() -> Vec<(String, usize, usize, usize, usize)>>);

#[derive(Default)]
pub(super) struct Sampled {
    pub(super) every_ms: u64,
    /// The timeline: a sample when the footprint moved 8 MB from the last one kept, or 250 ms passed.
    pub(super) rows: Vec<Sample>,
    /// The sample with the highest footprint seen.
    pub(super) top: Sample,
    /// Bytes the high-water mark rose while each `thread: path` was open, and the most footprint seen while it was.
    pub(super) rise: std::collections::BTreeMap<String, (u64, u64)>,
    /// `RERAIL_HEAP_SAMPLE`: the live heap by allocating span at the peak (`rerail_prof::heap::snapshot`), and the
    /// footprint it was taken at.
    pub(super) heap: Vec<(u32, u64)>,
    /// The same by allocating call stack (`RERAIL_HEAP_FRAMES`).
    pub(super) heap_frames: Vec<((u32, Vec<usize>), u64)>,
    /// The relations' blocks live then, by the span that made them and their life (`rows::held_by_span`): span,
    /// life, bytes used, blocks.
    pub(super) blocks: Vec<(u32, u8, usize, usize, String)>,
    /// The runs' relations held then (`RelationsByName`).
    pub(super) relations: Vec<(String, usize, usize, usize, usize)>,
    pub(super) heap_at: u64,
    /// `RERAIL_HEAP_AT=<text>`: the live heap by allocating span at the most footprint seen while a thread had a
    /// span open whose path holds the text (a phase that is not the peak's), and that footprint.
    pub(super) heap_in: Vec<(u32, u64)>,
    pub(super) heap_in_at: u64,
}

/// `RERAIL_HEAP_AT`'s text, read once a run.
fn heap_at_text() -> Option<String> {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_HEAP_AT").ok().filter(|t| !t.is_empty()))
}

pub(super) static SAMPLED: std::sync::LazyLock<std::sync::Mutex<Sampled>> = std::sync::LazyLock::new(Default::default);

/// Starts the sampler, once per process.
pub(super) fn sample() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let every_ms = std::env::var("RERAIL_PROF_MEMORY_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(5u64).max(1);
        SAMPLED.lock().unwrap().every_ms = every_ms;
        // Relational work's device leases are sampled as the heap is, under the span that leased them.
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        if rerail_prof::heap::on() {
            rerail_accel::gpu::shared::set_lease_hook(|ptr, len| {
                if len == 0 {
                    rerail_prof::heap::free(ptr);
                } else {
                    rerail_prof::heap::alloc_by(ptr, len, rerail_prof::heap::DEVICE);
                }
            });
            // Q203 (c): what holds the device's budget when a lease is refused, by the span that leased it (the first
            // few refusals of a process).
            rerail_accel::gpu::shared::set_refusal_hook(|why| {
                // Every refusal by the span that asked (one line), the holdings for the first few.
                static TOLD: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                eprintln!("rerail: refused in {}: {why}", rerail_prof::heap::name(rerail_prof::heap::span()));
                if TOLD.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 6 {
                    return;
                }
                let held: Vec<(u32, u64)> = rerail_prof::heap::snapshot().into_iter().filter(|(id, _)| id & rerail_prof::heap::DEVICE != 0).collect();
                let all: u64 = held.iter().map(|h| h.1).sum();
                let mut out = format!("rerail: refused, device leases by span ({} MB sampled): {why}", all >> 20);
                for (id, bytes) in held.iter().take(12) {
                    out.push_str(&format!("\n  {:>6.1} MB  {}", *bytes as f64 / 1048576.0, rerail_prof::heap::name(*id)));
                }
                // With `RERAIL_HEAP_FRAMES`, each big holding's leasing stack, for `atos -l` against the load address.
                let frames: Vec<((u32, Vec<usize>), u64)> = rerail_prof::heap::snapshot_frames().into_iter().filter(|((id, _), b)| id & rerail_prof::heap::DEVICE != 0 && *b >= 8 << 20).collect();
                if !frames.is_empty() {
                    out.push_str(&format!("\n  stacks, load {:#x}:", rerail_prof::heap::load_address()));
                    for ((_, stack), bytes) in frames.iter().take(16) {
                        let at: Vec<String> = stack.iter().take(14).map(|a| format!("{a:#x}")).collect();
                        out.push_str(&format!("\n  stack {:.1} MB: {}", *bytes as f64 / 1048576.0, at.join(" ")));
                    }
                }
                eprintln!("{out}");
            });
        }
        let _ = rerail_accel::exec::service("profile memory", None, move || {
            let start = std::time::Instant::now();
            let mut last_max = footprint().1;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(every_ms));
                let (now, max) = footprint();
                let (live, resident) = heap();
                let at = start.elapsed().as_secs_f64();
                let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
                let s = SAMPLED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let keep = s.rows.last().is_none_or(|l| now.abs_diff(l.footprint) >= 8 << 20 || at - l.at >= 0.25);
                let rose = max.saturating_sub(last_max);
                let top = now > s.top.footprint;
                if !(keep || rose > 0 || top) {
                    continue;
                }
                // The threads' open spans are read without this lock held: a thread closing a span holds its own lock
                // and waits for the profile's state, which a dump holds while it waits for this one (a run stopped by
                // `timeout` hung so, its profile never written).
                drop(s);
                let (mut open, threads) = super::open_paths();
                open.extend(doing());
                let mut s = SAMPLED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                for p in &open {
                    let e = s.rise.entry(p.clone()).or_default();
                    e.0 += rose;
                    e.1 = e.1.max(now);
                }
                last_max = max;
                let sample = Sample { at, unix, footprint: now, max, live, resident, open, threads, holders: holders() };
                if top {
                    s.top = sample.clone();
                    // The heap by span at the peak, again each 16 MB it rises (a scan of the sampler's table).
                    if rerail_prof::heap::on() && now >= s.heap_at + (16 << 20) {
                        s.heap_at = now;
                        drop(s);
                        let heap = rerail_prof::heap::snapshot();
                        let heap_frames = rerail_prof::heap::snapshot_frames();
                        let blocks = super::machine::<BlocksBySpan>().0.get().map_or_else(Vec::new, |f| f());
                        let relations = super::machine::<RelationsByName>().0.get().map_or_else(Vec::new, |f| f());
                        s = SAMPLED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        s.heap = heap;
                        s.heap_frames = heap_frames;
                        s.blocks = blocks;
                        s.relations = relations;
                    }
                }
                if let Some(text) = heap_at_text()
                    && rerail_prof::heap::on()
                    && now >= s.heap_in_at + (16 << 20)
                    && sample.open.iter().any(|p| p.contains(text.as_str()))
                {
                    s.heap_in_at = now;
                    drop(s);
                    let heap = rerail_prof::heap::snapshot();
                    s = SAMPLED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    s.heap_in = heap;
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
    // The footprint then, by holder: the heap's live bytes and what it keeps resident beyond them, the device's
    // buffers (the driver's count; of them the pool's reservations and relational work's leases), SQLite's heap
    // (inside jemalloc's when it allocates there), and the rest (stacks, code, maps, other allocators).
    let [driver, reserved, leased, idle, sqlite, host, host_free] = t.holders;
    let heap_mb = |b: u64| (b >> 20) as usize;
    let accounted = heap_mb(t.resident) + driver;
    let _ = writeln!(r, "  by holder then (MB): heap live {}, heap resident beyond live {}; Metal driver {} (device pool {}, relational leases {} + {} idle); SQLite heap {}; host blocks {} ({} free on their lists); the rest {}",
        heap_mb(t.live), heap_mb(t.resident.saturating_sub(t.live)), driver, reserved, leased, idle, sqlite, host, host_free, heap_mb(t.footprint).saturating_sub(accounted + host));
    // Blocks' memory handed out again and zeroed for it (decision 0342): a free list's chunk asked zeroed (a
    // descriptor), a recycled slab's allocation; a relation's rows are written before read and never zeroed.
    let _ = writeln!(r, "  blocks: {} MB zeroed as their memory was handed out again", rerail_accel::blocks::zeroed_bytes() >> 20);
    // What threads keep on the device between strata, past every admission (Q203).
    let [scratch] = rerail_accel::gpu::shared::kept();
    let _ = writeln!(r, "  device kept by threads between strata: join scratch {} MB at most ({} MB at the end)", scratch.1 >> 20, scratch.0 >> 20);
    if !s.heap.is_empty() {
        let total: u64 = s.heap.iter().map(|h| h.1).sum();
        let _ = writeln!(r, "Live heap (and device leases) by the span that allocated it, near the peak (at {} MB footprint; sampled 1 in {} KB, all of 64 KB and more; {} MB in all, {} samples dropped):",
            mb(s.heap_at as f64), rerail_prof::heap::INTERVAL >> 10, mb(total as f64), rerail_prof::heap::DROPPED.load(std::sync::atomic::Ordering::Relaxed));
        for (id, bytes) in s.heap.iter().take(30) {
            let _ = writeln!(r, "{:>10} MB  {}", mb(*bytes as f64), super::short(&rerail_prof::heap::name(*id), 150));
        }
    }
    if !s.blocks.is_empty() {
        let total: usize = s.blocks.iter().map(|b| b.2).sum();
        let _ = writeln!(r, "Relations' blocks live then, by the span that made them (at {} MB footprint; {} MB used in all, of {} blocks):",
            mb(s.heap_at as f64), total >> 20, s.blocks.iter().map(|b| b.3).sum::<usize>());
        let lives = ["a run's", "sources'", "kept", "a device run's own"];
        for (id, life, bytes, n, by) in s.blocks.iter().take(30) {
            let by = if by.is_empty() { String::new() } else { format!(", by {by}") };
            let _ = writeln!(r, "{:>10} MB  {:>5} blocks, {}, made in {}{by}", bytes >> 20, n, lives.get(*life as usize).unwrap_or(&"?"), super::short(&rerail_prof::heap::name(*id), 140));
        }
    }
    if !s.relations.is_empty() {
        let total: usize = s.relations.iter().map(|r| r.3).sum();
        let rows: usize = s.relations.iter().map(|r| r.1).sum();
        let _ = writeln!(r, "The runs' relations held then, by name (at {} MB footprint; {} relations, {} rows, {} MB of values in all; rows, columns, MB, runs):",
            mb(s.heap_at as f64), s.relations.len(), rows, total >> 20);
        // Every one: what the rows are is read off the whole list, not its head.
        for (name, rows, k, bytes, runs) in s.relations.iter() {
            let _ = writeln!(r, "{:>10} KB  {:>9} rows  {k} columns  {runs} runs  {}", bytes >> 10, rows, super::short(name, 120));
        }
    }
    if let Some(text) = heap_at_text()
        && !s.heap_in.is_empty()
    {
        let total: u64 = s.heap_in.iter().map(|h| h.1).sum();
        let _ = writeln!(r, "Live heap by the span that allocated it, at the most footprint while \"{text}\" was open (at {} MB footprint; {} MB in all):", mb(s.heap_in_at as f64), mb(total as f64));
        for (id, bytes) in s.heap_in.iter().take(30) {
            let _ = writeln!(r, "{:>10} MB  {}", mb(*bytes as f64), super::short(&rerail_prof::heap::name(*id), 150));
        }
    }
    let mut rose: Vec<(&String, &(u64, u64))> = s.rise.iter().filter(|(_, v)| v.0 > 0).collect();
    rose.sort_by_key(|(_, v)| std::cmp::Reverse(v.0));
    let _ = writeln!(r, "Open (innermost, on any thread) while the high-water mark rose, by the rise; most footprint seen while open");
    for (p, (rise, most)) in rose.iter().take(20) {
        let _ = writeln!(r, "{:>10} MB {:>10} MB  {}", mb(*rise as f64), mb(*most as f64), super::short(p, 140));
    }
}

/// The live heap by allocating span near the peak (`RERAIL_HEAP_SAMPLE`), every span's, as TSV: the report shows
/// the first 30, and a peak's tail of many small holders is where its bulk can be.
pub fn heap_tsv() -> String {
    use std::fmt::Write as _;
    let s = SAMPLED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut t = String::from("bytes\tspan\n");
    for (id, bytes) in &s.heap {
        let _ = writeln!(t, "{bytes}\t{}", rerail_prof::heap::name(*id));
    }
    t
}

/// The live heap by allocating call stack (`RERAIL_HEAP_FRAMES`), taken with the span snapshot near the peak, as
/// TSV: bytes, then the return addresses innermost first; the header gives the executable's load address for
/// `atos -o <binary> -l <load> <addresses>`. Empty when stacks were not recorded.
pub fn heap_frames_tsv() -> String {
    use std::fmt::Write as _;
    let s = SAMPLED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut t = format!("# load {:#x}\nbytes\tframes\tspan\n", rerail_prof::heap::load_address());
    for ((span, frames), bytes) in &s.heap_frames {
        let _ = writeln!(t, "{bytes}\t{}\t{}", frames.iter().map(|f| format!("{f:#x}")).collect::<Vec<_>>().join(" "), rerail_prof::heap::name(*span));
    }
    t
}

/// The sampler's timeline as TSV.
pub fn sampled_tsv() -> String {
    use std::fmt::Write as _;
    let s = SAMPLED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut t = String::from("at_s\tunix_s\tfootprint_bytes\tmax_bytes\tjemalloc_live_bytes\tjemalloc_resident_bytes\tmetal_driver_mb\tdevice_pool_mb\trelational_leased_mb\trelational_idle_mb\tsqlite_mb\thost_blocks_mb\thost_free_mb\tthreads\topen\n");
    for r in &s.rows {
        let [d, p, l, i, q, h, f] = r.holders;
        let _ = writeln!(t, "{:.3}\t{:.3}\t{}\t{}\t{}\t{}\t{d}\t{p}\t{l}\t{i}\t{q}\t{h}\t{f}\t{}\t{}", r.at, r.unix, r.footprint, r.max, r.live, r.resident, r.threads, r.open.join(" | "));
    }
    t
}

/// The memory ceiling in bytes: `RERAIL_MEMORY_MAX_MB`, else three quarters of the machine's memory; `None` with
/// `off` (or where neither the machine's memory nor the process's can be read).
pub fn memory_ceiling() -> Option<u64> {
    match std::env::var("RERAIL_MEMORY_MAX_MB").ok().as_deref().map(str::trim) {
        Some("off") | Some("0") => None,
        Some(mb) if mb.parse::<u64>().is_ok() => Some(mb.parse::<u64>().unwrap() << 20),
        _ => physical_bytes().map(|b| b / 4 * 3),
    }
    .filter(|_| resident_bytes().is_some())
}

/// The machine's memory, in bytes.
pub fn physical_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut size: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        // SAFETY: `hw.memsize` is a u64; `size` and `len` describe its buffer.
        let code = unsafe { libc::sysctlbyname(c"hw.memsize".as_ptr(), (&mut size as *mut u64).cast(), &mut len, std::ptr::null_mut(), 0) };
        (code == 0 && size > 0).then_some(size)
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_family = "wasm")))]
    {
        // SAFETY: sysconf has no pointer arguments.
        let (pages, page) = unsafe { (libc::sysconf(libc::_SC_PHYS_PAGES), libc::sysconf(libc::_SC_PAGESIZE)) };
        (pages > 0 && page > 0).then(|| pages as u64 * page as u64)
    }
    #[cfg(windows)]
    {
        rerail_prof::windows_physical_bytes()
    }
    #[cfg(not(any(all(unix, not(target_family = "wasm")), windows)))]
    {
        None
    }
}

/// The process's resident memory, in bytes (`None` where it cannot be read).
pub fn resident_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut info = std::mem::MaybeUninit::<libc::mach_task_basic_info>::zeroed();
        let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
        unsafe extern "C" {
            // The task's own port (`mach_task_self()` is a macro over it; libc's function is deprecated).
            static mach_task_self_: libc::mach_port_t;
        }
        // SAFETY: task_info writes this C-layout buffer; count describes its capacity in words.
        let code = unsafe { libc::task_info(mach_task_self_, libc::MACH_TASK_BASIC_INFO, info.as_mut_ptr().cast(), &mut count) };
        // SAFETY: a successful task_info call initializes the output structure.
        (code == libc::KERN_SUCCESS).then(|| unsafe { info.assume_init() }.resident_size)
    }
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/statm").ok()?;
        let pages = status.split_whitespace().nth(1)?.parse::<u64>().ok()?;
        // SAFETY: sysconf has no pointer arguments.
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        (page_size > 0).then(|| pages.saturating_mul(page_size as u64))
    }
    #[cfg(windows)]
    {
        rerail_prof::windows_working_set()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        None
    }
}
