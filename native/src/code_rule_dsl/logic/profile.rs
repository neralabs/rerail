//! A run's profile (`RERAIL_PROFILE=<dir>`): where a rule's CPU goes, in every phase, down to the SQL statement and
//! the native join step, so a slow rule can be read and fixed.
//!
//! - **Spans.** `span("name")` opens a named span on this thread until the guard drops. Spans nest into paths
//!   (`local/stratum violation/sql #12`), summed over every call: calls, CPU (user + system of the thread, never
//!   wall) in total and in the span itself. The worker thread's spans (`worker/…`) are its own.
//! - **SQL.** Each statement SQLite runs on a traced connection is charged to the span running it, by text: runs,
//!   SQLite's time, VM steps, full-scan steps, sorts, automatic-index rows (`sql`). VM steps do not move with load.
//! - **Native joins.** Each native component's plans with rows in and out of every step, summed over the chunks
//!   (`native`).
//! - **Notes.** `note(key, text)`: one text per key (a rule's text, a statement's query plan).
//!
//! - **Plan (EXPLAIN ANALYZE).** Like Postgres's: per stratum, the engine that ran it and why; per rule plan, the
//!   planner's search (subsets and candidates priced), its estimated work and rows, and per step the access method,
//!   the source read, the estimated rows after the step and the actual rows into and out of it, summed over the loops
//!   (chunks). A plan that differs between chunks is its own entry (`explain`).
//! - **Device.** Each Metal kernel run: launches, the command buffers' GPU time (`GPUStartTime` to `GPUEndTime`),
//!   the host's wall time waiting, and the cells it solved; and where a device path fell back.
//! - **Decisions.** The run's choices and optimizations: the file-local split, chunks, text tests and their matchers,
//!   nearest-ancestor sets and where they solve, inputs read in place.
//!
//! - **Roofline.** `work(unit, n)` charges n units of one kind of work to the open span; SQL statements charge their
//!   VM steps and native joins the rows into their steps on their own. The report sets each span's work against the
//!   best time this machine gives a unit (measured as it is written, `roofline`): roof time, excess, and a join's
//!   amplification, and lists the CPU no counter explains.
//!
//! `write(dir)` writes `profile.txt` (the report: spans by CPU, statements by time with their query plans, native
//! steps by work, the roofline, and warnings) and the tables `spans.tsv`, `sql.tsv` and `native.tsv`. Off, a span costs one load.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

mod kinds;
mod memory;
mod roofline;
mod watch;
pub(in crate::code_rule_dsl) use roofline::Unit;
pub use watch::watch;

static ON: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(std::env::var_os("RERAIL_PROFILE").is_some()));

/// Whether this run is profiled.
pub(crate) fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Turns profiling on or off (a test that profiles without the environment variable).
#[allow(dead_code)]
pub(in crate::code_rule_dsl) fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

static DIR: LazyLock<Mutex<Option<std::path::PathBuf>>> = LazyLock::new(|| Mutex::new(std::env::var_os("RERAIL_PROFILE").map(Into::into)));

/// The directory the next report is written to: `RERAIL_PROFILE`'s, unless set (one per rule in a measurement).
pub(in crate::code_rule_dsl) fn directory() -> Option<std::path::PathBuf> {
    DIR.lock().unwrap().clone()
}

/// Sets the directory the next report is written to.
#[allow(dead_code)]
pub(in crate::code_rule_dsl) fn set_directory(dir: &Path) {
    *DIR.lock().unwrap() = Some(dir.to_path_buf());
}

fn cpu() -> f64 {
    // macOS: the thread's CPU by `clock_gettime`, a quarter of `thread_info`'s cost (every span reads it twice).
    #[cfg(target_os = "macos")]
    {
        let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: fills `t` with the calling thread's CPU time.
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
        t.tv_sec as f64 + t.tv_nsec as f64 / 1e9
    }
    #[cfg(not(target_os = "macos"))]
    {
        let (u, s) = super::plan::corpus::thread_usage();
        u + s
    }
}

#[derive(Default, Clone)]
pub(in crate::code_rule_dsl) struct SpanTotal {
    pub(in crate::code_rule_dsl) calls: u64,
    pub(in crate::code_rule_dsl) total: f64,
    pub(in crate::code_rule_dsl) own: f64,
    /// Bytes its thread allocated while it was open, in all and less the spans inside it, and bytes freed
    /// (`memory`): allocated less freed is what it kept.
    pub(in crate::code_rule_dsl) alloc: u64,
    pub(in crate::code_rule_dsl) own_alloc: u64,
    pub(in crate::code_rule_dsl) freed: u64,
    /// How far the process's footprint high-water mark rose while it was open, in all and less the spans inside it.
    pub(in crate::code_rule_dsl) rise: u64,
    pub(in crate::code_rule_dsl) own_rise: u64,
}

#[derive(Default, Clone)]
pub(in crate::code_rule_dsl) struct SqlTotal {
    pub(in crate::code_rule_dsl) runs: u64,
    pub(in crate::code_rule_dsl) ns: u64,
    pub(in crate::code_rule_dsl) vm: u64,
    pub(in crate::code_rule_dsl) scan: u64,
    pub(in crate::code_rule_dsl) sort: u64,
    pub(in crate::code_rule_dsl) autoindex: u64,
}

#[derive(Default)]
struct State {
    spans: BTreeMap<String, SpanTotal>,
    /// By (span path, statement text).
    sql: BTreeMap<(String, String), SqlTotal>,
    /// By (span path, plan text): per step, rows in and out.
    native: BTreeMap<(String, String), Vec<(u64, u64)>>,
    /// By (span path, plan text): the CPU seconds its probes ran, as the native engine timed them.
    native_cpu: BTreeMap<(String, String), f64>,
    notes: BTreeMap<String, String>,
    /// Run-level decisions by (topic, text): how many times made.
    decisions: BTreeMap<(String, String), u64>,
    /// Strata in the order first run: key, engine, why, loops.
    strata: Vec<Stratum>,
    /// The strata that ran in SQLite (the SQL wave kernel), by key: the kind of literal the native engine declined.
    fallbacks: BTreeMap<String, String>,
    /// Plans by (stratum, kind, plan text).
    plans: BTreeMap<(String, String, String), PlanTotal>,
    /// Device kernels by (span path, kernel).
    devices: BTreeMap<(String, String), DeviceTotal>,
    /// Work charged by (span path, unit) (`work`).
    work: BTreeMap<(String, Unit), u64>,
    /// Waits by (the waiter's span path, what it waited on, what that was doing when the wait began) (`wait`).
    waits: BTreeMap<(String, String, String), WaitTotal>,
    /// Where each native stratum ran, by stratum (`placed`, decision 0095).
    placements: BTreeMap<String, PlacedTotal>,
    /// SQL inside the waves (`sql_in_wave`, `wait_in_wave`), by (hazard class, span path, connection and statement):
    /// runs and nanoseconds.
    hazards: BTreeMap<(&'static str, String, String), (u64, u64)>,
}

/// A stratum's placements, summed over its runs (chunks, guests, the cross-file part).
#[derive(Default)]
struct PlacedTotal {
    runs: u64,
    on_gpu: u64,
    gpu_cheaper: u64,
    device: bool,
    forms: Vec<&'static str>,
    missing: Vec<&'static str>,
    planned_work: f64,
    work: u64,
    cpu_estimate: f64,
    gpu_estimate: f64,
    cpu: f64,
    /// Its join steps by the form placement chose (CPU index, GPU sort and probe, GPU tile): how many, their
    /// estimate on the CPU's index, and on the form chosen (of the steps estimated under `JOIN_ESTIMATE_CAP`), and
    /// how many were estimated over it.
    joins: [(u64, f64, f64, u64); 3],
    /// Its runs on the device (decision 0095, step 4), summed: rules, stages, submissions, dispatches, the GPU's
    /// time, this thread's CPU encoding, waiting and reading back, uploads, residency hits; and why the device
    /// refused it, when it was placed there and did not run there.
    gpu: GpuTotal,
    refused: Option<String>,
}

#[derive(Default, Clone)]
struct GpuTotal {
    runs: u64,
    rules: u64,
    stages: u64,
    submissions: u64,
    dispatches: u64,
    reruns: u64,
    gpu: f64,
    encode: f64,
    upload: f64,
    wait: f64,
    wait_wall: f64,
    decode: f64,
    uploads: u64,
    upload_bytes: u64,
    resident: u64,
    sorts: u64,
    sort_hits: u64,
    rows: u64,
    distinct: u64,
    bindings: u64,
}

impl GpuTotal {
    fn add(&mut self, o: &GpuTotal) {
        self.runs += o.runs;
        self.rules += o.rules;
        self.stages += o.stages;
        self.submissions += o.submissions;
        self.dispatches += o.dispatches;
        self.reruns += o.reruns;
        self.gpu += o.gpu;
        self.encode += o.encode;
        self.upload += o.upload;
        self.wait += o.wait;
        self.wait_wall += o.wait_wall;
        self.decode += o.decode;
        self.uploads += o.uploads;
        self.upload_bytes += o.upload_bytes;
        self.resident += o.resident;
        self.sorts += o.sorts;
        self.sort_hits += o.sort_hits;
        self.rows += o.rows;
        self.distinct += o.distinct;
        self.bindings += o.bindings;
    }

    /// The host's CPU on the device's work: encoding, uploads, waiting, reading back.
    fn host(&self) -> f64 {
        self.encode + self.upload + self.wait + self.decode
    }
}

/// What the device reported for a stratum (`setfix::device`).
#[cfg(target_os = "macos")]
pub(in crate::code_rule_dsl) type DeviceReport = crate::code_rule_dsl::logic::native::component::DeviceReport;
#[cfg(not(target_os = "macos"))]
pub(in crate::code_rule_dsl) type DeviceReport = ();

#[derive(Default)]
struct WaitTotal {
    count: u64,
    /// Wall time blocked, and the waiter's CPU meanwhile (spinning, waking).
    wall: f64,
    cpu: f64,
}

/// What a thread is doing, for the threads that wait on it (`wait`): one shared marker, set as it moves between
/// phases. Cheap to set: a store of a static name.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Activity(parking_lot::Mutex<&'static str>);

impl Activity {
    pub(in crate::code_rule_dsl) fn set(&self, doing: &'static str) {
        *self.0.lock() = doing;
    }
    pub(in crate::code_rule_dsl) fn get(&self) -> &'static str {
        *self.0.lock()
    }
}

/// Runs `blocking` (a receive, a send on a full channel, a join), recorded as a wait of this thread's span on
/// `awaited`, and on what it was doing when it began (`doing`, an `Activity` of the awaited thread or a fixed name). The wall
/// time is the wait; the CPU spent meanwhile is reported with it. Not profiling, it only runs `blocking`.
pub(in crate::code_rule_dsl) fn wait<T>(awaited: &str, doing: &Activity, blocking: impl FnOnce() -> T) -> T {
    if !on() {
        return blocking();
    }
    let at = doing.get();
    let (wall0, cpu0) = (std::time::Instant::now(), cpu());
    let out = blocking();
    let (wall, spent) = (wall0.elapsed().as_secs_f64(), cpu() - cpu0);
    let path = current();
    let mut state = STATE.lock().unwrap();
    let w = state.waits.entry((path, awaited.to_owned(), if at.is_empty() { "-".to_owned() } else { at.to_owned() })).or_default();
    w.count += 1;
    w.wall += wall;
    w.cpu += spent;
    out
}

struct Stratum {
    key: String,
    engine: String,
    why: String,
    loops: u64,
}

/// One planned step: the literal, how it is read, from what, and the rows the planner expects after it.
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct StepPlan {
    pub(in crate::code_rule_dsl) literal: String,
    pub(in crate::code_rule_dsl) access: String,
    pub(in crate::code_rule_dsl) source: String,
    pub(in crate::code_rule_dsl) rows: f64,
}

/// A rule's plan as the planner chose it: estimated work and rows, and its search (subsets kept, candidates priced).
#[derive(Clone, Debug, Default)]
pub(in crate::code_rule_dsl) struct RulePlan {
    pub(in crate::code_rule_dsl) work: f64,
    pub(in crate::code_rule_dsl) rows: f64,
    pub(in crate::code_rule_dsl) subsets: u64,
    pub(in crate::code_rule_dsl) candidates: u64,
    pub(in crate::code_rule_dsl) steps: Vec<StepPlan>,
}

#[derive(Default)]
struct PlanTotal {
    rule: Option<usize>,
    plan: RulePlan,
    loops: u64,
    planning: f64,
    replans: u64,
    actual: Vec<(u64, u64)>,
    measured: bool,
    produced: Option<u64>,
}

#[derive(Default)]
struct DeviceTotal {
    runs: u64,
    launches: u64,
    gpu: f64,
    wall: f64,
    cells: u64,
    /// Where the host's time went, summed (`device_phases`): seconds per `PHASES` entry.
    phases: [f64; PHASES.len()],
    /// Bytes of buffers made for the runs, and of buffers reused.
    made: u64,
    reused: u64,
}

/// A device run's host-side phases, in order (`device_phases`).
pub(in crate::code_rule_dsl) const PHASES: [&str; 7] = [
    "buffers made (map, wire, copy)",
    "command queue made",
    "encode",
    "submit (commit to kernelStartTime)",
    "schedule (kernelStartTime to GPUStartTime: residency)",
    "GPU",
    "wake (GPUEndTime to wait returning)",
];

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

struct Open {
    path: String,
    start: f64,
    children: f64,
    /// The thread's allocated and freed bytes, and the footprint's high-water mark, when it opened; what the spans
    /// inside it allocated and raised.
    alloc: (u64, u64),
    peak: u64,
    children_alloc: u64,
    children_rise: u64,
}

/// A thread's open spans and the SQL statement it runs, where a dump on another thread can read them (`watch`).
struct Thread {
    name: String,
    /// Its Mach port (macOS), for its CPU read from another thread; 0 elsewhere.
    port: u32,
    counters: memory::Counters,
    opens: Vec<Open>,
    /// The statement SQLite is running for it, and since when (`statement`).
    statement: Option<(String, std::time::Instant)>,
}

/// Every thread that opened a span, while it lives.
static THREADS: LazyLock<Mutex<Vec<std::sync::Weak<parking_lot::Mutex<Thread>>>>> = LazyLock::new(Default::default);

fn register() -> std::sync::Arc<parking_lot::Mutex<Thread>> {
    #[cfg(target_os = "macos")]
    // SAFETY: the calling thread's own port; no right is taken.
    let port = unsafe { libc::pthread_mach_thread_np(libc::pthread_self()) };
    #[cfg(not(target_os = "macos"))]
    let port = 0;
    let me = std::sync::Arc::new(parking_lot::Mutex::new(Thread {
        name: std::thread::current().name().unwrap_or("unnamed").to_owned(),
        port,
        counters: memory::Counters::mine(),
        opens: Vec::new(),
        statement: None,
    }));
    let mut all = THREADS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    all.retain(|t| t.strong_count() > 0);
    all.push(std::sync::Arc::downgrade(&me));
    me
}

thread_local! {
    static STACK: std::sync::Arc<parking_lot::Mutex<Thread>> = register();
}

/// Records the SQL statement this thread starts (`Some`), or that it ended (`None`), for a dump. Only when profiling.
pub(in crate::code_rule_dsl) fn statement(text: Option<String>) {
    if on() {
        STACK.with(|s| s.lock().statement = text.map(|t| (t, std::time::Instant::now())));
    }
}

/// An open span; it closes when dropped.
pub(in crate::code_rule_dsl) struct Span(bool);

/// Opens span `name` (made only when profiling) under this thread's open span.
pub(in crate::code_rule_dsl) fn span(name: impl FnOnce() -> String) -> Span {
    if !on() {
        return Span(false);
    }
    let name = name();
    memory::sample();
    STACK.with(|s| {
        let mut s = s.lock();
        let path = match s.opens.last() {
            Some(top) => format!("{}/{name}", top.path),
            None => name,
        };
        let alloc = s.counters.read();
        s.opens.push(Open { path, start: cpu(), children: 0.0, alloc, peak: memory::peak_sampled(), children_alloc: 0, children_rise: 0 });
    });
    Span(true)
}

impl Drop for Span {
    fn drop(&mut self) {
        if !self.0 {
            return;
        }
        let now = cpu();
        let peak = memory::peak_sampled();
        STACK.with(|s| {
            let mut s = s.lock();
            let (allocated, freed) = s.counters.read();
            let Some(open) = s.opens.pop() else { return };
            let total = now - open.start;
            let alloc = allocated.saturating_sub(open.alloc.0);
            let rise = peak.saturating_sub(open.peak);
            if let Some(parent) = s.opens.last_mut() {
                parent.children += total;
                parent.children_alloc += alloc;
                parent.children_rise += rise;
            }
            let mut state = STATE.lock().unwrap();
            let t = state.spans.entry(open.path).or_default();
            t.calls += 1;
            t.total += total;
            t.own += total - open.children;
            t.alloc += alloc;
            t.own_alloc += alloc.saturating_sub(open.children_alloc);
            t.freed += freed.saturating_sub(open.alloc.1);
            t.rise += rise;
            t.own_rise += rise.saturating_sub(open.children_rise);
        });
    }
}

/// Each live thread's innermost open span, as `thread: path` (for the memory sampler).
fn open_paths() -> (Vec<String>, usize) {
    let all: Vec<_> = THREADS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().filter_map(std::sync::Weak::upgrade).collect();
    (all.iter().filter_map(|t| { let t = t.lock(); t.opens.last().map(|o| format!("{}: {}", t.name, o.path)) }).collect(), all.len())
}

/// The process's peaks so far, for a caller to print after a run (the run's report is written before its caller's
/// own work): the high-water mark of the footprint and of the resident set, and the sampler's peak with what was
/// open then. Also rewrites `memory.tsv` in the profile directory, so the timeline covers the caller's work too.
pub(in crate::code_rule_dsl) fn peaks() -> String {
    let (now, max) = memory::footprint();
    let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
    // SAFETY: fills a `rusage` for this process.
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    // macOS reports bytes, Linux kilobytes.
    let rss = if cfg!(target_os = "macos") { usage.ru_maxrss as f64 } else { usage.ru_maxrss as f64 * 1024.0 };
    let mut r = format!("process: footprint {} MB now, {} MB at most; resident set {} MB at most\n", memory::mb(now as f64), memory::mb(max as f64), memory::mb(rss));
    memory::sampled_report(&mut r);
    if let Some(dir) = directory() {
        let _ = std::fs::write(dir.join("memory.tsv"), memory::sampled_tsv());
    }
    r
}

/// This thread's open span's path, `-` outside any.
pub(in crate::code_rule_dsl) fn current() -> String {
    STACK.with(|s| s.lock().opens.last().map_or_else(|| "-".to_owned(), |o| o.path.clone()))
}

/// Charges one run of statement `text` to the current span.
pub(in crate::code_rule_dsl) fn sql(text: String, ns: u64, vm: u64, scan: u64, sort: u64, autoindex: u64) {
    let path = current();
    let mut state = STATE.lock().unwrap();
    let t = state.sql.entry((path, text)).or_default();
    t.runs += 1;
    t.ns += ns;
    t.vm += vm;
    t.scan += scan;
    t.sort += sort;
    t.autoindex += autoindex;
}

/// Waves under way in this process (`waves`): SQL from a wave's thread meanwhile is a hazard.
static WAVES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

thread_local! {
    /// This thread works behind the run (the store's writer): what it sends to SQLite no wave waits for.
    static BEHIND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Waves are under way until dropped: from a run's first wave (its first chunk) to its last (the cross-file part's
/// last stratum). SQL before and after is the run's beginning and end, where it may be; inside, it is a hazard.
pub(in crate::code_rule_dsl) struct Waves(bool);

pub(in crate::code_rule_dsl) fn waves() -> Waves {
    let on = on();
    if on {
        WAVES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    Waves(on)
}

impl Drop for Waves {
    fn drop(&mut self) {
        if self.0 {
            WAVES.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// Marks this thread as working behind the run (the store's writer).
pub(crate) fn behind_the_run() {
    BEHIND.with(|b| b.set(true));
}

fn in_waves() -> bool {
    on() && WAVES.load(std::sync::atomic::Ordering::Relaxed) > 0 && !BEHIND.with(std::cell::Cell::get)
}

/// Why a statement inside the waves forces a wait on SQLite, by where it ran and what it does.
fn hazard_class(conn: &str, path: &str, text: &str) -> &'static str {
    let t = text.trim_start().to_ascii_uppercase();
    if conn == "store" {
        return if t.starts_with("SELECT") || t.starts_with("WITH") { "a read of the store (memo, tapes, statistics, kept state)" } else { "a write to the store on a wave's thread" };
    }
    if t.contains("FROM SET_DERIVED") {
        "a native relation copied into a SQL table"
    } else if t.starts_with("CREATE") || t.starts_with("DROP") || t.starts_with("ALTER") {
        "a SQL table made or dropped for a relation"
    } else if t.starts_with("INSERT") && t.contains(" VALUES") {
        "facts written into a SQL table"
    } else if t.starts_with("INSERT") || t.starts_with("REPLACE") || t.starts_with("DELETE") || t.starts_with("UPDATE") {
        "a rule or a delta run in SQLite"
    } else if (t.starts_with("SELECT") || t.starts_with("WITH")) && path.contains("input load") {
        "an input read from a SQL table"
    } else if t.starts_with("SELECT") || t.starts_with("WITH") {
        "a relation read from SQL"
    } else {
        "other SQL (attach, pragma, transaction)"
    }
}

/// A statement that connection `conn` (`run`: the run's database; `store`: the content store) ran in `ns`
/// nanoseconds: inside the waves, on a wave's thread, it is a hazard (decision 0104: no wave waits for SQL).
pub(crate) fn sql_in_wave(conn: &'static str, text: &str, ns: u64) {
    if !in_waves() {
        return;
    }
    let path = current();
    let class = hazard_class(conn, &path, text);
    let mut state = STATE.lock().unwrap();
    let e = state.hazards.entry((class, path, format!("{conn}: {text}"))).or_default();
    e.0 += 1;
    e.1 += ns;
}

/// A wave's thread waited `wall` seconds for SQLite's side of the store (`what`: a queued write it reads, the queue's
/// back-pressure): a hazard like a statement.
pub(crate) fn wait_in_wave(what: &str, wall: f64) {
    if !in_waves() {
        return;
    }
    let path = current();
    let mut state = STATE.lock().unwrap();
    let e = state.hazards.entry(("a wait for the store's writer", path, format!("store: {what}"))).or_default();
    e.0 += 1;
    e.1 += (wall * 1e9) as u64;
}

/// The hazards: SQL a wave's thread ran or waited on between the run's first wave and its last.
fn hazards(r: &mut String, state: &State) {
    let (runs, ns): (u64, u64) = state.hazards.values().fold((0, 0), |a, v| (a.0 + v.0, a.1 + v.1));
    let _ = writeln!(
        r,
        "\n== Forces SQL waits (hazards): {} statements, {runs} runs, {:.1} ms inside the waves",
        state.hazards.len(),
        ns as f64 / 1e6
    );
    let _ = writeln!(r, "SQL is in a run's data path only before its first wave and after its last. A wave that runs, reads or waits on");
    let _ = writeln!(r, "SQL either waits for SQLite or reads rows it was not given (stale). Each is a hazard to remove.");
    if state.hazards.is_empty() {
        return;
    }
    let mut by: BTreeMap<&str, (usize, u64, u64)> = BTreeMap::new();
    for ((class, _, _), (n, t)) in &state.hazards {
        let e = by.entry(class).or_default();
        e.0 += 1;
        e.1 += n;
        e.2 += t;
    }
    let mut by: Vec<_> = by.into_iter().collect();
    by.sort_by_key(|(_, v)| std::cmp::Reverse(v.2));
    let _ = writeln!(r, "By kind (ms: SQLite's wall time, waits for its locks included):\n       ms     runs  stmts  kind");
    for (class, (k, n, t)) in &by {
        let _ = writeln!(r, "  {:>7.1} {n:>8} {k:>6}  {class}", *t as f64 / 1e6);
    }
    let mut all: Vec<_> = state.hazards.iter().collect();
    all.sort_by_key(|(_, (_, t))| std::cmp::Reverse(*t));
    let _ = writeln!(r, "Costliest (of {}):\n       ms     runs  kind / span / statement", all.len());
    for ((class, path, text), (n, t)) in all.iter().take(40) {
        let _ = writeln!(r, "  {:>7.1} {n:>8}  {class}\n                    {path}\n                    {}", *t as f64 / 1e6, short(text, 160));
    }    // The run's own connection's, every one: few, and each a relation the engine should hold instead.
    let own: Vec<_> = all.iter().filter(|((_, _, text), _)| text.starts_with("run: ")).collect();
    let _ = writeln!(r, "On the run's connection (all {}):\n       ms     runs  kind / span / statement", own.len());
    for ((class, path, text), (n, t)) in own {
        let _ = writeln!(r, "  {:>7.1} {n:>8}  {class}\n                    {path}\n                    {}", *t as f64 / 1e6, short(text, 160));
    }
}

/// Charges `n` units of `unit` work to the current span (the roofline). Off, it costs one load.
pub(in crate::code_rule_dsl) fn work(unit: Unit, n: u64) {
    if !on() || n == 0 {
        return;
    }
    let path = current();
    *STATE.lock().unwrap().work.entry((path, unit)).or_default() += n;
}

/// Adds a native plan's rows in and out per step, and its probes' CPU seconds, to the current span.
pub(in crate::code_rule_dsl) fn native(plan: String, steps: &[(u64, u64)], cpu_s: f64) {
    let path = current();
    let mut state = STATE.lock().unwrap();
    *state.native_cpu.entry((path.clone(), plan.clone())).or_default() += cpu_s;
    let t = state.native.entry((path, plan)).or_default();
    if t.len() < steps.len() {
        t.resize(steps.len(), (0, 0));
    }
    for (a, b) in t.iter_mut().zip(steps) {
        a.0 += b.0;
        a.1 += b.1;
    }
}

/// Keeps `text` under `key` (the first one given).
pub(in crate::code_rule_dsl) fn note(key: String, text: impl FnOnce() -> String) {
    let mut state = STATE.lock().unwrap();
    state.notes.entry(key).or_insert_with(text);
}

/// A statement's text as the SQL trace keys it: its words, single-spaced.
pub(in crate::code_rule_dsl) fn normal(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Notes `statement`'s query plan (`EXPLAIN QUERY PLAN`, once per text) when profiling.
pub(in crate::code_rule_dsl) fn plan(db: &rusqlite::Connection, statement: &str) {
    if !on() {
        return;
    }
    let key = format!("plan:{}", normal(statement));
    if STATE.lock().unwrap().notes.contains_key(&key) {
        return;
    }
    let text = (|| -> rusqlite::Result<String> {
        let mut q = db.prepare(&format!("EXPLAIN QUERY PLAN {statement}"))?;
        let rows: Vec<(i64, i64, String)> = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
        // Each step indented under its parent.
        let mut depth: BTreeMap<i64, usize> = BTreeMap::new();
        let mut out = String::new();
        for (id, parent, detail) in rows {
            let d = depth.get(&parent).map_or(0, |d| d + 1);
            depth.insert(id, d);
            let _ = writeln!(out, "{}{detail}", "  ".repeat(d));
        }
        Ok(out)
    })()
    .unwrap_or_else(|e| format!("(no plan: {e})"));
    note(key, || text);
}

/// This thread's CPU now (to time a part a span cannot hold).
pub(in crate::code_rule_dsl) fn now() -> f64 {
    cpu()
}

/// A run-level decision or optimization under `topic` (counted when made again, as per chunk).
pub(in crate::code_rule_dsl) fn decision(topic: &str, text: impl FnOnce() -> String) {
    if !on() {
        return;
    }
    *STATE.lock().unwrap().decisions.entry((topic.to_owned(), text())).or_default() += 1;
}

/// The decisions made so far (none when profiling is off): `decisions_since` takes the ones made after.
pub(in crate::code_rule_dsl) fn decisions() -> BTreeMap<(String, String), u64> {
    if !on() {
        return BTreeMap::new();
    }
    STATE.lock().unwrap().decisions.clone()
}

/// The decisions made since `before` (`decisions`), with how often: what a step decided, for a run that reuses the
/// step's result to state again (`add_decisions`).
pub(in crate::code_rule_dsl) fn decisions_since(before: &BTreeMap<(String, String), u64>) -> Vec<((String, String), u64)> {
    decisions().into_iter().filter_map(|(k, n)| n.checked_sub(before.get(&k).copied().unwrap_or(0)).filter(|d| *d > 0).map(|d| (k, d))).collect()
}

/// Decisions a reused step made when it ran (`decisions_since`), stated for this run.
pub(in crate::code_rule_dsl) fn add_decisions(made: &[((String, String), u64)]) {
    if !on() {
        return;
    }
    let mut state = STATE.lock().unwrap();
    for (k, n) in made {
        *state.decisions.entry(k.clone()).or_default() += n;
    }
}

/// The engine stratum `key` ran on, and why (the first reason given; one loop per call).
/// Records where native stratum `key` ran (`plan::placement`), its shape, the work it did and the CPU it took.
pub(in crate::code_rule_dsl) fn placed(key: &str, p: &crate::code_rule_dsl::logic::plan::placement::Placement, shape: &crate::code_rule_dsl::logic::plan::placement::Shape, work: u64, cpu: f64, device: Option<&DeviceReport>) {
    if !on() {
        return;
    }
    let mut state = STATE.lock().unwrap();
    let t = state.placements.entry(key.to_owned()).or_default();
    t.runs += 1;
    #[cfg(target_os = "macos")]
    let ran = match device {
        Some(d) if d.refused.is_none() => {
            t.gpu.add(&GpuTotal {
                runs: 1,
                rules: u64::from(d.rules),
                stages: u64::from(d.stages),
                submissions: u64::from(d.submissions),
                dispatches: u64::from(d.dispatches),
                reruns: u64::from(d.reruns),
                gpu: d.gpu_seconds,
                encode: d.encode_cpu,
                upload: d.upload_cpu,
                wait: d.wait_cpu,
                wait_wall: d.wait_wall,
                decode: d.decode_cpu,
                uploads: u64::from(d.uploads),
                upload_bytes: d.upload_bytes,
                resident: u64::from(d.resident_hits),
                sorts: u64::from(d.sorts),
                sort_hits: u64::from(d.sort_hits),
                rows: d.rows,
                distinct: d.distinct,
                bindings: d.bindings,
            });
            true
        }
        Some(d) => {
            t.refused = d.refused.clone();
            false
        }
        None => false,
    };
    #[cfg(not(target_os = "macos"))]
    let ran = device.is_some() && false;
    t.on_gpu += u64::from(ran);
    t.gpu_cheaper += u64::from(p.gpu_cheaper);
    t.device |= p.device;
    t.forms = shape.forms.iter().map(|f| f.name()).collect();
    t.missing = p.missing.iter().map(|f| f.name()).collect();
    t.planned_work += shape.work;
    t.work += work;
    t.cpu_estimate += p.cpu_s;
    t.gpu_estimate += p.gpu_s;
    t.cpu += cpu;
    for c in &p.joins {
        use crate::code_rule_dsl::logic::plan::placement::JoinForm;
        let chosen = match c.form {
            JoinForm::Index => c.index_s,
            JoinForm::Probe => c.probe_s.unwrap_or(c.index_s),
            JoinForm::Tile => c.tile_s.unwrap_or(c.index_s),
        };
        let e = &mut t.joins[c.form as usize];
        e.0 += 1;
        if c.index_s < JOIN_ESTIMATE_CAP {
            e.1 += c.index_s;
            e.2 += chosen;
        } else {
            e.3 += 1;
        }
    }
}

/// The strata the device ran (decision 0095, step 4): in all, then the costliest; and the strata placed there that it
/// refused, by reason (no stratum is moved from the device to the CPU without a line here).
fn device_strata(r: &mut String, all: &BTreeMap<String, PlacedTotal>) {
    let ran: Vec<(&String, &PlacedTotal)> = all.iter().filter(|(_, t)| t.gpu.runs > 0).collect();
    let refused: Vec<(&String, &PlacedTotal)> = all.iter().filter(|(_, t)| t.refused.is_some()).collect();
    if ran.is_empty() && refused.is_empty() {
        return;
    }
    let mut sum = GpuTotal::default();
    for (_, t) in &ran {
        sum.add(&t.gpu);
    }
    let cpu: f64 = ran.iter().map(|(_, t)| t.cpu).sum();
    let ms = |s: f64| s * 1e3;
    let _ = writeln!(r, "Strata run on the GPU: {} strata, {} runs, {} rules in {} stages; their CPU {:.2} s (the whole stratum: planning its replans, the store, the device's host part)",
        ran.len(), sum.runs, sum.rules, sum.stages, cpu);
    let _ = writeln!(r, "  GPU {:.1} ms (command buffers' timestamps) in {} submissions, {} dispatches ({} stages run again after an overflow)",
        ms(sum.gpu), sum.submissions, sum.dispatches, sum.reruns);
    let _ = writeln!(r, "  host CPU {:.1} ms: encode {:.1}, uploads {:.1}, wait {:.1} ({:.1} ms wall), rows back into the store {:.1}",
        ms(sum.host()), ms(sum.encode), ms(sum.upload), ms(sum.wait), ms(sum.wait_wall), ms(sum.decode));
    let _ = writeln!(r, "  relations uploaded {} ({:.1} MB), found resident {}; sorts made {}, found resident {}; bindings {}, head rows {} ({} distinct, made so on the device)",
        sum.uploads, sum.upload_bytes as f64 / 1e6, sum.resident, sum.sorts, sum.sort_hits, sum.bindings, sum.rows, sum.distinct);
    let mut top = ran.clone();
    top.sort_by(|a, b| b.1.cpu.total_cmp(&a.1.cpu));
    let _ = writeln!(r, "{:>8} {:>8} {:>8} {:>8} {:>5} {:>6} {:>6} {:>10}  stratum (costliest 25 on the GPU)", "cpu ms", "host ms", "gpu ms", "est ms", "runs", "rules", "subm", "rows");
    for (key, t) in top.iter().take(25) {
        let g = &t.gpu;
        let _ = writeln!(r, "{:>8.2} {:>8.2} {:>8.3} {:>8.3} {:>5} {:>6} {:>6} {:>10}  {}", ms(t.cpu), ms(g.host()), ms(g.gpu), ms(t.gpu_estimate), g.runs, g.rules, g.submissions, g.rows, short(key, 60));
    }
    if !refused.is_empty() {
        let mut by: BTreeMap<&str, (u64, f64, Vec<(f64, &str)>)> = BTreeMap::new();
        for (key, t) in &refused {
            let e = by.entry(t.refused.as_deref().unwrap_or("")).or_default();
            e.0 += 1;
            e.1 += t.cpu;
            e.2.push((t.cpu, key.as_str()));
        }
        let mut by: Vec<_> = by.into_iter().collect();
        by.sort_by(|a, b| b.1.1.total_cmp(&a.1.1));
        let _ = writeln!(r, "Placed on the GPU and refused by it (run on the CPU), by reason (its costliest strata):");
        let _ = writeln!(r, "{:>9} {:>7}  reason", "cpu s", "strata");
        for (why, (n, c, mut strata)) in by {
            strata.sort_by(|a, b| b.0.total_cmp(&a.0));
            let names: Vec<String> = strata.iter().take(3).map(|(c, k)| format!("{} {:.3} s", short(k, 40), c)).collect();
            let _ = writeln!(r, "{c:>9.3} {n:>7}  {why} ({})", names.join(", "));
        }
    }
}

/// A join step estimated over this many seconds is a planned estimate gone wrong (a cross product priced at 10^20
/// rows): counted, not summed.
const JOIN_ESTIMATE_CAP: f64 = 1e3;

/// Where the native strata ran (decision 0095): how many on each backend, the strata the GPU would run cheaper and
/// the forms it lacks for them, by their CPU. What the device must take on next, and what it would save.
fn placements(r: &mut String, state: &State) {
    use crate::code_rule_dsl::logic::plan::placement::gpu_price;
    let all = &state.placements;
    if all.is_empty() {
        return;
    }
    let runs: u64 = all.values().map(|t| t.runs).sum();
    let gpu: u64 = all.values().map(|t| t.on_gpu).sum();
    let cpu: f64 = all.values().map(|t| t.cpu).sum();
    let device = all.values().any(|t| t.device);
    let _ = writeln!(
        r,
        "\n== Placement (decision 0095): {} strata, {runs} runs; {gpu} runs on the GPU, {} on the CPU ({cpu:.2} s CPU); binding {}",
        all.len(),
        runs - gpu,
        if device { "Metal" } else { "CPU" }
    );
    let _ = writeln!(r, "Prices: CPU measured (100 ns a unit of planned work, 0.13 ms a stratum); GPU: each join step on its device form (the relational lab's measures), each stage and submission at the host's measured cost.");
    device_strata(r, all);
    // The strata the GPU would run cheaper in at least one run, and did not.
    let cheaper: Vec<(&String, &PlacedTotal)> = all.iter().filter(|(_, t)| t.gpu_cheaper > t.on_gpu).collect();
    let cheaper_cpu: f64 = cheaper.iter().map(|(_, t)| t.cpu).sum();
    // The GPU priced at the work each stratum did, not the work planned: a planned estimate can be far off (a cross
    // product priced at 10^20 rows that met none).
    let at_work = |t: &PlacedTotal| gpu_price(t.runs, t.work as f64);
    let cheaper_gpu: f64 = cheaper.iter().map(|(_, t)| at_work(t)).sum();
    let _ = writeln!(r, "The GPU would be cheaper for {} strata: {cheaper_cpu:.2} s CPU taken, against {cheaper_gpu:.2} s on the GPU at the work they did.", cheaper.len());
    // Strata whose planned work is off by 10x or more, either way, by their CPU: the placement read a wrong price, and
    // the join order likely came from the same estimate.
    let off = |t: &PlacedTotal| {
        let (p, w) = (t.planned_work.max(1.0), (t.work as f64).max(1.0));
        w / p >= 10.0 || p / w >= 10.0
    };
    let (under, over) = all.values().filter(|t| off(t)).fold(((0u64, 0.0), (0u64, 0.0)), |(u, o), t| {
        if (t.work as f64) > t.planned_work { ((u.0 + 1, u.1 + t.cpu), o) } else { (u, (o.0 + 1, o.1 + t.cpu)) }
    });
    let _ = writeln!(r, "Planned work off by 10x or more: {} strata under (did more, {:.2} s CPU), {} over (did less, {:.2} s CPU); marked ! below.", under.0, under.1, over.0, over.1);
    // By the form the GPU lacks: a stratum counts under each form it lacks.
    let mut by: BTreeMap<&str, (u64, f64)> = BTreeMap::new();
    for (_, t) in &cheaper {
        for f in &t.missing {
            let e = by.entry(f).or_default();
            e.0 += 1;
            e.1 += t.cpu;
        }
    }
    let mut by: Vec<_> = by.into_iter().collect();
    by.sort_by(|a, b| b.1.1.total_cmp(&a.1.1));
    let _ = writeln!(r, "GPU support missing, by form (a stratum counts under each form it lacks):");
    let _ = writeln!(r, "{:>9} {:>7}  form", "cpu s", "strata");
    for (f, (n, c)) in &by {
        let _ = writeln!(r, "{c:>9.3} {n:>7}  {f}");
    }
    // The strata themselves, costliest first.
    let mut top = cheaper.clone();
    top.sort_by(|a, b| b.1.cpu.total_cmp(&a.1.cpu));
    // Each join step on the form its prices chose (the lab's measured device prices): what the device's joins would
    // take on, in strata the device runs once it has their other forms.
    let joins: Vec<(u64, f64, f64, u64)> = (0..3).map(|f| all.values().fold((0, 0.0, 0.0, 0), |a, t| (a.0 + t.joins[f].0, a.1 + t.joins[f].1, a.2 + t.joins[f].2, a.3 + t.joins[f].3))).collect();
    if joins.iter().any(|j| j.0 > 0) {
        use crate::code_rule_dsl::logic::plan::placement::JoinForm;
        let _ = writeln!(r, "Join steps by the form their prices choose (estimates; the device's prices measured by the relational lab; steps estimated over {JOIN_ESTIMATE_CAP:.0} s counted, not summed):");
        let _ = writeln!(r, "{:>9} {:>10} {:>10} {:>9}  form", "steps", "index s", "chosen s", "over cap");
        for (f, form) in [JoinForm::Index, JoinForm::Probe, JoinForm::Tile].into_iter().enumerate() {
            let _ = writeln!(r, "{:>9} {:>10.3} {:>10.3} {:>9}  {}", joins[f].0, joins[f].1, joins[f].2, joins[f].3, form.name());
        }
    }
    let _ = writeln!(r, "Costliest strata the GPU would run cheaper:");
    let _ = writeln!(r, "{:>8} {:>8} {:>12} {:>12} {:>5}  stratum: forms (missing on the GPU)", "cpu s", "gpu s", "planned", "work", "runs");
    for (key, t) in top.iter().take(20) {
        let mark = if off(t) { "!" } else { " " };
        let _ = writeln!(r, "{:>8.3} {:>8.4} {:>12.0} {:>11}{mark} {:>5}  {}: {} (missing {})", t.cpu, at_work(t), t.planned_work, t.work, t.runs, short(key, 60), t.forms.join(", "), t.missing.join(", "));
    }
}

/// Records that stratum `key` fell back to SQLite, and the kind of literal the native engine declined (`class`).
pub(in crate::code_rule_dsl) fn fallback(key: &str, class: String) {
    if !on() {
        return;
    }
    STATE.lock().unwrap().fallbacks.insert(key.to_owned(), class);
}

/// The strata that fell back to SQLite, costliest first, and the same by the kind of literal declined: each one's
/// CPU (its `stratum …` spans, wherever they ran) and SQLite's VM steps. What the native engine lacks, by its cost.
fn fallbacks(r: &mut String, state: &State) {
    let roots: f64 = state.spans.iter().filter(|(p, _)| !p.contains('/')).map(|(_, t)| t.total).sum();
    let mut rows: Vec<(f64, u64, u64, &String, &String)> = state
        .fallbacks
        .iter()
        .map(|(key, class)| {
            let name = format!("stratum {key}");
            let at = |p: &String| p == &name || p.ends_with(&format!("/{name}"));
            let cpu: f64 = state.spans.iter().filter(|(p, _)| at(p)).map(|(_, t)| t.total).sum();
            let calls: u64 = state.spans.iter().filter(|(p, _)| at(p)).map(|(_, t)| t.calls).sum();
            let vm: u64 = state.sql.iter().filter(|((p, _), _)| at(p) || p.contains(&format!("{name}/"))).map(|(_, s)| s.vm).sum();
            (cpu, vm, calls, key, class)
        })
        .collect();
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    let total: f64 = rows.iter().map(|r| r.0).sum();
    let _ = writeln!(r, "\n== Fell back to SQLite: {} strata the native engine declined, {total:.2} s CPU of {roots:.2} s", rows.len());
    if rows.is_empty() {
        return;
    }
    let mut by: BTreeMap<&String, (usize, f64, u64)> = BTreeMap::new();
    for (cpu, vm, _, _, class) in &rows {
        let e = by.entry(class).or_default();
        e.0 += 1;
        e.1 += cpu;
        e.2 += vm;
    }
    let mut by: Vec<_> = by.into_iter().collect();
    by.sort_by(|a, b| b.1.1.total_cmp(&a.1.1));
    let _ = writeln!(r, "By what the native engine has no step for:");
    let _ = writeln!(r, "{:>9} {:>7} {:>14}  declined", "cpu s", "strata", "SQLite VM");
    for (class, (n, cpu, vm)) in by.iter().take(15) {
        let _ = writeln!(r, "{cpu:>9.3} {n:>7} {vm:>14}  {class}");
    }
    let _ = writeln!(r, "Costliest strata (their plans and reasons are under == Plan):");
    let _ = writeln!(r, "{:>9} {:>6} {:>14}  stratum: declined", "cpu s", "runs", "SQLite VM");
    for (cpu, vm, calls, key, class) in rows.iter().take(20) {
        let _ = writeln!(r, "{cpu:>9.3} {calls:>6} {vm:>14}  {}: {class}", short(key, 90));
    }
}

pub(in crate::code_rule_dsl) fn stratum(key: &str, engine: &str, why: impl FnOnce() -> String) {
    if !on() {
        return;
    }
    let mut state = STATE.lock().unwrap();
    match state.strata.iter_mut().find(|s| s.key == key) {
        Some(s) if s.engine == engine => s.loops += 1,
        Some(_) => {
            // The engine changed between chunks: both are kept.
            let (k, e, w) = (format!("{key} (chunk with {engine})"), engine.to_owned(), why());
            match state.strata.iter_mut().find(|s| s.key == k) {
                Some(s) => s.loops += 1,
                None => state.strata.push(Stratum { key: k, engine: e, why: w, loops: 1 }),
            }
        }
        None => state.strata.push(Stratum { key: key.to_owned(), engine: engine.to_owned(), why: why(), loops: 1 }),
    }
}

/// A rule plan run in stratum `key`: `kind` (seed, join with its delta, SQL insert …), the rule's number, the CPU
/// its planning took, how often it was replanned, the actual rows into and out of each step where measured, and the
/// rows it produced where known. One loop per call; equal plans add up.
#[allow(clippy::too_many_arguments)]
pub(in crate::code_rule_dsl) fn planned(
    key: &str,
    kind: &str,
    rule: Option<usize>,
    plan: &RulePlan,
    planning: f64,
    replans: u64,
    actual: Option<&[(u64, u64)]>,
    produced: Option<u64>,
) {
    if !on() {
        return;
    }
    let text = plan.steps.iter().map(|s| s.literal.as_str()).collect::<Vec<_>>().join(" , ");
    let mut state = STATE.lock().unwrap();
    let t = state.plans.entry((key.to_owned(), kind.to_owned(), text)).or_default();
    t.rule = rule;
    t.plan = plan.clone();
    t.loops += 1;
    t.planning += planning;
    t.replans += replans;
    if let Some(actual) = actual {
        t.measured = true;
        if t.actual.len() < actual.len() {
            t.actual.resize(actual.len(), (0, 0));
        }
        for (a, b) in t.actual.iter_mut().zip(actual) {
            a.0 += b.0;
            a.1 += b.1;
        }
    }
    if let Some(n) = produced {
        *t.produced.get_or_insert(0) += n;
    }
}

/// A device kernel's run in the current span: its launches, GPU seconds, the host's wall seconds waiting, its cells.
pub(in crate::code_rule_dsl) fn device(kernel: &str, launches: u64, gpu: f64, wall: f64, cells: u64) {
    if !on() {
        return;
    }
    let path = current();
    let mut state = STATE.lock().unwrap();
    let t = state.devices.entry((path, kernel.to_owned())).or_default();
    t.runs += 1;
    t.launches += launches;
    t.gpu += gpu;
    t.wall += wall;
    t.cells += cells;
}

/// A device run's host-side phases (seconds, in `PHASES` order) and the buffer bytes it made and reused, added to
/// the current span's entry for `kernel`.
pub(in crate::code_rule_dsl) fn device_phases(kernel: &str, phases: [f64; PHASES.len()], made: u64, reused: u64) {
    if !on() {
        return;
    }
    let path = current();
    let mut state = STATE.lock().unwrap();
    let t = state.devices.entry((path, kernel.to_owned())).or_default();
    for (a, b) in t.phases.iter_mut().zip(phases) {
        *a += b;
    }
    t.made += made;
    t.reused += reused;
}

/// The profile is process-wide, so one profiled run (or profile test) at a time: a run that resets it holds this,
/// and so does a test that turns it on, so a run in another thread cannot reset the records the test reads.
static ONE_RUN: Mutex<()> = Mutex::new(());

/// Holds the profile for one run (`ONE_RUN`).
pub(in crate::code_rule_dsl) fn one_run() -> std::sync::MutexGuard<'static, ()> {
    ONE_RUN.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Holds the profile for one run if no run holds it: a run nested in another (phase 0's, inside a run's preparing)
/// is then the outer run's, and neither resets nor writes the profile.
pub(in crate::code_rule_dsl) fn one_run_if_free() -> Option<std::sync::MutexGuard<'static, ()>> {
    match ONE_RUN.try_lock() {
        Ok(guard) => Some(guard),
        Err(std::sync::TryLockError::Poisoned(e)) => Some(e.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => None,
    }
}

/// Forgets everything recorded (a run's start).
pub(in crate::code_rule_dsl) fn reset() {
    *STATE.lock().unwrap() = State::default();
}

/// The statements recorded, by SQLite's time, most first: (span, text, totals).
pub(in crate::code_rule_dsl) fn statements() -> Vec<(String, String, SqlTotal)> {
    let state = STATE.lock().unwrap();
    let mut all: Vec<_> = state.sql.iter().map(|((p, t), s)| (p.clone(), t.clone(), s.clone())).collect();
    all.sort_by_key(|(_, _, s)| std::cmp::Reverse(s.ns));
    all
}

/// The spans recorded: path and totals.
pub(in crate::code_rule_dsl) fn spans() -> BTreeMap<String, SpanTotal> {
    STATE.lock().unwrap().spans.clone()
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_owned() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

/// An estimate of rows: two decimals below 10, else whole.
fn rows(x: f64) -> String {
    if x < 10.0 { format!("{x:.2}") } else { format!("{x:.0}") }
}

fn rule_name(rule: Option<usize>, kind: &str) -> String {
    match rule {
        Some(k) => format!("rule #{k:04} ({kind})"),
        None => kind.to_owned(),
    }
}

/// The EXPLAIN ANALYZE section: decisions, then each stratum's engine and its plans, steps with estimates and actuals.
fn explain(r: &mut String, state: &State) {
    let _ = writeln!(r, "\n== Decisions and optimizations (times made)");
    if state.decisions.is_empty() {
        r.push_str("(none recorded)\n");
    }
    let mut topic = "";
    for ((t, text), n) in &state.decisions {
        if t != topic {
            let _ = writeln!(r, "{t}:");
            topic = t;
        }
        let _ = writeln!(r, "  - {text}{}", if *n > 1 { format!("  (x{n})") } else { String::new() });
    }
    let _ = writeln!(
        r,
        "\n== Plan (EXPLAIN ANALYZE): strata in run order; per plan: est = the planner's rows after each step (per loop), actual = rows in -> out summed over loops"
    );
    // The costliest plans first: by planning CPU and by actual rows into their steps.
    let rows_in = |t: &PlanTotal| t.actual.iter().map(|(a, _)| a).sum::<u64>();
    let mut top: Vec<(&(String, String, String), &PlanTotal)> = state.plans.iter().collect();
    top.sort_by(|a, b| b.1.planning.total_cmp(&a.1.planning));
    let _ = writeln!(r, "Costliest plans by planning CPU:");
    for ((key, kind, _), t) in top.iter().take(8) {
        let _ = writeln!(r, "  {:>8.3} s  {} in {key}  ({} loops, {} steps)", t.planning, rule_name(t.rule, kind), t.loops, t.plan.steps.len());
    }
    top.sort_by_key(|(_, t)| std::cmp::Reverse(rows_in(t)));
    let _ = writeln!(r, "Costliest plans by actual rows into their steps:");
    for ((key, kind, _), t) in top.iter().take(8).filter(|(_, t)| rows_in(t) > 0) {
        let _ = writeln!(r, "  {:>12}  {} in {key}  ({} loops)", rows_in(t), rule_name(t.rule, kind), t.loops);
    }
    r.push('\n');
    for s in &state.strata {
        let plans: Vec<(&(String, String, String), &PlanTotal)> = state.plans.iter().filter(|((k, _, _), _)| *k == s.key).collect();
        let planning: f64 = plans.iter().map(|(_, t)| t.planning).sum();
        let _ = writeln!(r, "Stratum {}  [engine: {}]  (loops={}, planning {planning:.3} s CPU)", short(&s.key, 120), s.engine, s.loops);
        if !s.why.is_empty() {
            let _ = writeln!(r, "  why: {}", s.why);
        }
        for ((_, kind, _), t) in plans {
            let produced = t.produced.map_or(String::new(), |n| format!(" produced={n}"));
            let _ = writeln!(
                r,
                "  -> {}  (est work={:.0} rows={:.1}) (loops={} replans={}{produced})  search: {} subsets, {} candidates; planning {:.3} s",
                rule_name(t.rule, kind),
                t.plan.work,
                t.plan.rows,
                t.loops,
                t.replans,
                t.plan.subsets,
                t.plan.candidates,
                t.planning
            );
            for (i, st) in t.plan.steps.iter().enumerate() {
                let actual = if t.measured {
                    t.actual.get(i).map_or("actual -".to_owned(), |(a, b)| format!("actual {a} -> {b}"))
                } else {
                    String::new()
                };
                let _ = writeln!(r, "       {:>2}. {:<44} {:<30} {:<20} est {:>11}  {actual}", i + 1, short(&st.literal, 44), short(&st.access, 30), short(&st.source, 20), rows(st.rows));
            }
        }
    }
    if state.strata.is_empty() {
        r.push_str("(no strata recorded)\n");
    }
}

/// The memory section: the process now, the spans by bytes they allocated themselves, and those that raised the
/// footprint's high-water mark (`memory`).
fn memory_report(r: &mut String, spans: &BTreeMap<String, SpanTotal>) {
    let (now, max) = memory::footprint();
    let (live, resident) = memory::heap();
    let _ = writeln!(
        r,
        "\n== Memory: footprint {} MB now, {} MB at most; jemalloc heap {} MB live, {} MB resident",
        memory::mb(now as f64),
        memory::mb(max as f64),
        memory::mb(live as f64),
        memory::mb(resident as f64)
    );
    let _ = writeln!(r, "Spans by bytes their thread allocated while open, less the spans inside them (own); kept = allocated less freed, in all");
    let _ = writeln!(r, "{:>10} {:>10} {:>10} {:>9}  span", "own MB", "all MB", "kept MB", "calls");
    let mut by: Vec<(&String, &SpanTotal)> = spans.iter().filter(|(_, t)| t.own_alloc > 0).collect();
    by.sort_by_key(|(_, t)| std::cmp::Reverse(t.own_alloc));
    for (p, t) in by.iter().take(25) {
        let kept = t.alloc as f64 - t.freed as f64;
        let _ = writeln!(r, "{:>10} {:>10} {:>10} {:>9}  {}", memory::mb(t.own_alloc as f64), memory::mb(t.alloc as f64), memory::mb(kept), t.calls, short(p, 140));
    }
    if by.is_empty() {
        r.push_str(if memory::Counters::mine().read() == (0, 0) { "(no allocation recorded: no jemalloc counters on this build)\n" } else { "(no span has closed)\n" });
    }
    let mut rose: Vec<(&String, &SpanTotal)> = spans.iter().filter(|(_, t)| t.own_rise > 0).collect();
    rose.sort_by_key(|(_, t)| std::cmp::Reverse(t.own_rise));
    let _ = writeln!(r, "Spans that raised the footprint's high-water mark (own: less the spans inside them; the peak is the process's, so another thread may have raised it meanwhile)");
    for (p, t) in rose.iter().take(15) {
        let _ = writeln!(r, "{:>10} MB  {}", memory::mb(t.own_rise as f64), short(p, 140));
    }
    memory::sampled_report(r);
}

/// CPU (user + system) of the thread with Mach port `port` (macOS), read from another thread; `None` elsewhere.
fn thread_cpu(port: u32) -> Option<f64> {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::thread_basic_info = unsafe { std::mem::zeroed() };
        let mut count = (std::mem::size_of::<libc::thread_basic_info>() / std::mem::size_of::<libc::integer_t>()) as libc::mach_msg_type_number_t;
        // SAFETY: THREAD_BASIC_INFO fills `count` integers; a port of a thread that has exited returns an error.
        let got = unsafe { libc::thread_info(port, libc::THREAD_BASIC_INFO as libc::thread_flavor_t, (&raw mut info).cast(), &mut count) };
        if got != 0 {
            return None;
        }
        let t = |v: libc::time_value_t| f64::from(v.seconds) + f64::from(v.microseconds) / 1e6;
        Some(t(info.user_time) + t(info.system_time))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = port;
        None
    }
}

/// Every thread's open spans now, innermost last, with the CPU and bytes each has taken so far, and the SQL
/// statement running: what a run was doing when it was dumped (`watch`). Open spans are not yet in the totals.
pub(in crate::code_rule_dsl) fn open_spans() -> String {
    let threads: Vec<_> = THREADS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().filter_map(std::sync::Weak::upgrade).collect();
    let mut r = String::new();
    let _ = writeln!(r, "== Open at the dump: each thread's open spans, outermost first, with CPU and bytes allocated since each opened");
    let mut shown = 0;
    for t in threads {
        // A thread inside `span` holds its own lock only briefly; one that does not let go is reported as such.
        let Some(t) = t.try_lock_for(std::time::Duration::from_millis(200)) else {
            let _ = writeln!(r, "thread (busy: its span list stayed locked)");
            continue;
        };
        if t.opens.is_empty() && t.statement.is_none() {
            continue;
        }
        shown += 1;
        let cpu = thread_cpu(t.port);
        let (allocated, freed) = t.counters.read();
        let _ = writeln!(
            r,
            "thread {:?}: cpu {} s, allocated {} MB, kept {} MB (its lifetime)",
            t.name,
            cpu.map_or_else(|| "?".to_owned(), |c| format!("{c:.2}")),
            memory::mb(allocated as f64),
            memory::mb(allocated as f64 - freed as f64)
        );
        for (depth, o) in t.opens.iter().enumerate() {
            let name = o.path.rsplit('/').next().unwrap_or(&o.path);
            let spent = cpu.map_or_else(|| "?".to_owned(), |c| format!("{:.3}", c - o.start));
            let _ = writeln!(r, "  {:>9} s {:>9} MB  {}{}", spent, memory::mb(allocated.saturating_sub(o.alloc.0) as f64), "  ".repeat(depth), short(name, 160));
        }
        if let Some((text, since)) = &t.statement {
            let _ = writeln!(r, "  SQL running for {:.2} s (wall): {}", since.elapsed().as_secs_f64(), short(text, 600));
            let plan = STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).notes.get(&format!("plan:{}", normal(text))).cloned();
            for line in plan.iter().flat_map(|p| p.lines()) {
                let _ = writeln!(r, "      {line}");
            }
        }
    }
    if shown == 0 {
        r.push_str("(no thread has an open span: profiling is off, or the run had not started)\n");
    }
    r
}

/// The report, and the tables, into `dir`. `header`: what was run (lines put first).
pub(in crate::code_rule_dsl) fn write(dir: &Path, header: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let state = STATE.lock().unwrap();
    let mut spans_tsv = String::from("path\tcalls\tcpu_s\town_s\talloc_bytes\town_alloc_bytes\tfreed_bytes\tpeak_rise_bytes\town_peak_rise_bytes\n");
    for (p, t) in &state.spans {
        let _ = writeln!(spans_tsv, "{p}\t{}\t{:.4}\t{:.4}\t{}\t{}\t{}\t{}\t{}", t.calls, t.total, t.own, t.alloc, t.own_alloc, t.freed, t.rise, t.own_rise);
    }
    std::fs::write(dir.join("spans.tsv"), spans_tsv)?;
    std::fs::write(dir.join("memory.tsv"), memory::sampled_tsv())?;
    let mut sql: Vec<_> = state.sql.iter().collect();
    sql.sort_by_key(|(_, s)| std::cmp::Reverse(s.ns));
    let mut sql_tsv = String::from("span\truns\tms\tvm_steps\tfullscan_steps\tsorts\tautoindex_rows\tsql\n");
    for ((p, text), s) in &sql {
        let _ = writeln!(sql_tsv, "{p}\t{}\t{:.3}\t{}\t{}\t{}\t{}\t{text}", s.runs, s.ns as f64 / 1e6, s.vm, s.scan, s.sort, s.autoindex);
    }
    std::fs::write(dir.join("sql.tsv"), sql_tsv)?;
    let work = |steps: &Vec<(u64, u64)>| steps.iter().map(|(a, _)| a).sum::<u64>();
    let mut native: Vec<_> = state.native.iter().collect();
    native.sort_by_key(|(_, s)| std::cmp::Reverse(work(s)));
    let mut native_tsv = String::from("span\twork\tsteps_in_out\tplan\tcpu_s\n");
    for ((p, plan), steps) in &native {
        let s: Vec<String> = steps.iter().map(|(a, b)| format!("{a}:{b}")).collect();
        let cpu = state.native_cpu.get(&(p.clone(), plan.clone())).copied().unwrap_or(0.0);
        let _ = writeln!(native_tsv, "{p}\t{}\t{}\t{plan}\t{cpu:.4}", work(steps), s.join(","));
    }
    std::fs::write(dir.join("native.tsv"), native_tsv)?;
    let mut placement_tsv = String::from("stratum\truns\ton_gpu\tgpu_cheaper\tplanned_work\twork\tcpu_s\tcpu_estimate_s\tgpu_estimate_s\tforms\tmissing\n");
    for (key, t) in &state.placements {
        let _ = writeln!(
            placement_tsv,
            "{key}\t{}\t{}\t{}\t{:.0}\t{}\t{:.4}\t{:.4}\t{:.4}\t{}\t{}",
            t.runs, t.on_gpu, t.gpu_cheaper, t.planned_work, t.work, t.cpu, t.cpu_estimate, t.gpu_estimate, t.forms.join(","), t.missing.join(",")
        );
    }
    std::fs::write(dir.join("placement.tsv"), placement_tsv)?;

    // The report.
    let mut r = String::new();
    r.push_str(header);
    fallbacks(&mut r, &state);
    hazards(&mut r, &state);
    placements(&mut r, &state);
    explain(&mut r, &state);
    // A root is a span with no parent recorded: a top span, or one whose parent was still open (a dump).
    let root = |p: &str| p.rsplit_once('/').is_none_or(|(parent, _)| !state.spans.contains_key(parent));
    let roots: f64 = state.spans.iter().filter(|(p, _)| root(p)).map(|(_, t)| t.total).sum();
    let _ = writeln!(r, "\n== Spans: CPU (user + system), summed over calls; own = less the spans inside it ({roots:.2} s in all)");
    let _ = writeln!(r, "{:>9} {:>9} {:>6} {:>9}  span", "cpu s", "own s", "%", "calls");
    // A tree: each span under its parent, children by CPU.
    fn walk(r: &mut String, spans: &BTreeMap<String, SpanTotal>, parent: Option<&str>, depth: usize, all: f64) {
        let mut kids: Vec<(&String, &SpanTotal)> = spans
            .iter()
            .filter(|(p, _)| match parent {
                None => p.rsplit_once('/').is_none_or(|(parent, _)| !spans.contains_key(parent)),
                Some(par) => p.strip_prefix(par).and_then(|rest| rest.strip_prefix('/')).is_some_and(|rest| !rest.contains('/')),
            })
            .collect();
        kids.sort_by(|a, b| b.1.total.total_cmp(&a.1.total));
        for (p, t) in kids {
            // Spans under 0.1% of the run are left out of the tree (the tables keep them).
            if t.total < all * 0.001 && depth > 0 {
                continue;
            }
            let name = p.rsplit('/').next().unwrap_or(p);
            let _ = writeln!(r, "{:>9.3} {:>9.3} {:>5.1}% {:>9}  {}{}", t.total, t.own, 100.0 * t.total / all.max(1e-9), t.calls, "  ".repeat(depth), short(name, 100));
            walk(r, spans, Some(p), depth + 1, all);
        }
    }
    walk(&mut r, &state.spans, None, 0, roots);
    let kinds_tsv = kinds::by_kind(&mut r, &state.spans);
    std::fs::write(dir.join("kinds.tsv"), kinds_tsv)?;
    kinds::unexplained(&mut r, &state.spans);
    kinds::searches(&mut r, &state.spans);
    let heads_tsv = kinds::by_head(&mut r, &native, &state.native_cpu);
    std::fs::write(dir.join("heads.tsv"), heads_tsv)?;
    memory_report(&mut r, &state.spans);

    let sql_ns: u64 = sql.iter().map(|(_, s)| s.ns).sum();
    let sql_vm: u64 = sql.iter().map(|(_, s)| s.vm).sum();
    let _ = writeln!(
        r,
        "\n== SQL statements by SQLite's time ({} distinct, {:.2} s, {sql_vm} VM steps in all); plans for the first 15",
        sql.len(),
        sql_ns as f64 / 1e9
    );
    let _ = writeln!(r, "{:>9} {:>7} {:>12} {:>11} {:>6} {:>9}  span / statement", "ms", "runs", "vm steps", "full scan", "sorts", "autoidx");
    for (i, ((p, text), s)) in sql.iter().take(40).enumerate() {
        let _ = writeln!(r, "{:>9.1} {:>7} {:>12} {:>11} {:>6} {:>9}  {p}", s.ns as f64 / 1e6, s.runs, s.vm, s.scan, s.sort, s.autoindex);
        let _ = writeln!(r, "{:>58}{}", "", short(text, 400));
        if i < 15
            && let Some(plan) = state.notes.get(&format!("plan:{text}"))
        {
            for line in plan.lines() {
                let _ = writeln!(r, "{:>60}{line}", "");
            }
        }
    }

    let _ = writeln!(r, "\n== Native joins by work (rows into every step, summed over chunks), with their probes' CPU; steps in:out");
    for ((p, plan), steps) in native.iter().take(30) {
        let s: Vec<String> = steps.iter().map(|(a, b)| format!("{a}:{b}")).collect();
        let cpu = state.native_cpu.get(&((*p).clone(), (*plan).clone())).copied().unwrap_or(0.0);
        let _ = writeln!(r, "{:>12}  {cpu:.2} s  {p}\n{:>14}steps {}\n{:>14}{}", work(steps), "", s.join(" "), "", short(plan, 600));
    }

    // The roofline: SQL's VM steps and the native joins' rows are charged here, the rest by `work`. A native
    // component's step counts are read after its run, so its rows go to the run's span (`…/engine run (joins)`).
    {
        let own: BTreeMap<String, f64> = state.spans.iter().map(|(p, t)| (p.clone(), t.own)).collect();
        let mut charged = state.work.clone();
        for ((p, _), s) in &sql {
            *charged.entry((p.clone(), Unit::VmStep)).or_default() += s.vm;
        }
        let mut joins: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for ((p, _), steps) in &native {
            let run = format!("{p}/engine run (joins)");
            let at = if own.contains_key(&run) { run } else { p.clone() };
            // A join's work: the rows into each step, and the rows its last step makes.
            let made = steps.last().map_or(0, |s| s.1);
            *charged.entry((at.clone(), Unit::JoinRow)).or_default() += work(steps) + made;
            let j = joins.entry(at).or_default();
            j.0 += work(steps) + made;
            j.1 += made;
        }
        roofline::report(&mut r, &own, &charged, &joins);
    }

    let _ = writeln!(r, "\n== Waits: wall time each thread spent blocked, on what, and what that was doing when the wait began");
    if state.waits.is_empty() {
        r.push_str("(no waits)\n");
    } else {
        let _ = writeln!(r, "{:>9} {:>9} {:>7}  waiter: waited on (which was doing)", "wall s", "cpu s", "waits");
        let mut waits: Vec<_> = state.waits.iter().collect();
        waits.sort_by(|a, b| b.1.wall.total_cmp(&a.1.wall));
        for ((path, on, doing), w) in waits {
            let _ = writeln!(r, "{:>9.3} {:>9.3} {:>7}  {path}: {on} ({doing})", w.wall, w.cpu, w.count);
        }
    }

    r.push_str(&super::lanes::prof::report());
    let _ = writeln!(r, "\n== Device (Metal): per span and kernel; GPU = command buffers' GPU time, wait = host wall time");
    if state.devices.is_empty() {
        r.push_str("(no device work)\n");
    }
    for ((p, k), d) in &state.devices {
        let _ = writeln!(
            r,
            "{:>9.3} ms GPU {:>9.3} ms wait {:>5} runs {:>5} launches {:>11} cells  {k}  ({p})",
            d.gpu * 1e3,
            d.wall * 1e3,
            d.runs,
            d.launches,
            d.cells
        );
        if d.phases.iter().any(|p| *p > 0.0) {
            for (name, s) in PHASES.iter().zip(d.phases) {
                let _ = writeln!(r, "{:>14.3} ms  {name}", s * 1e3);
            }
            let _ = writeln!(r, "{:>14} MB made, {} MB reused", d.made >> 20, d.reused >> 20);
        }
    }

    // The executor's scoreboard (rerail-accel): every job on every lane, its thread CPU and its phases. On a device
    // lane, `cpu s` is the host CPU the device cost; `wall` phases with no CPU are the device's own timeline.
    let _ = writeln!(r, "\n== Executor (rerail-accel): jobs per lane, host CPU per phase");
    let scores = super::lanes::scores();
    if scores.is_empty() {
        r.push_str("(no executor)\n");
    }
    for (binding, score) in scores {
        let _ = writeln!(r, "-- {binding}");
        let _ = writeln!(r, "{score}");
    }

    // The cost statistics (decision 0100 §14): the store, the samples read, and the rates learned in place of priors.
    let _ = writeln!(r, "\n== Cost statistics (RERAIL_COSTS)");
    r.push_str(&super::costs::summary());

    // Waits that ran others' jobs (decision 0100 §12): a thread waiting on the pool runs queued jobs meanwhile, and a
    // long one makes its own wait end late. Per site: its waits, the jobs they ran, and how late those made them.
    let (waits, late_by) = rerail_accel::exec::waits();
    let _ = writeln!(r, "\n== Waits that ran others' jobs (rerail-accel)");
    if waits.is_empty() {
        r.push_str("(no waits)\n");
    } else {
        let _ = writeln!(
            r,
            "{:<40} {:>7} {:>7} {:>7} {:>10} {:>10} {:>10} {:>6} {:>10} {:>10} {:>10}  longest job run",
            "wait", "waits", "helped", "jobs", "wait ms", "jobs ms", "jobs cpu", "late", "late ms<=", "late max", "late exact"
        );
        for (site, w) in &waits {
            let _ = writeln!(
                r,
                "{:<40} {:>7} {:>7} {:>7} {:>10.1} {:>10.1} {:>10.1} {:>6} {:>10.2} {:>10.2} {:>10.2}  {:.2} ms {}",
                short(site, 40),
                w.waits,
                w.helping,
                w.jobs,
                w.wall * 1e3,
                w.jobs_wall * 1e3,
                w.jobs_cpu * 1e3,
                w.late,
                w.late_wall * 1e3,
                w.late_max * 1e3,
                w.late_exact * 1e3,
                w.longest * 1e3,
                w.longest_job
            );
        }
        // `late ms<=`: a wait that found its work ready right after a job it ran was late by at most that job's
        // wall; `late exact` where the waited work said when it ended (a batch's launches, a host job).
        let mut late_by = late_by;
        late_by.sort_by(|a, b| b.1.late_wall.total_cmp(&a.1.late_wall));
        let _ = writeln!(r, "jobs that made waits late: {:<34} {:>7} {:>10} {:>10}", "job", "waits", "late ms<=", "late max");
        for (job, l) in late_by.iter().take(12) {
            let _ = writeln!(r, "  {:<58} {:>7} {:>10.2} {:>10.2}", short(job, 58), l.waits, l.late_wall * 1e3, l.late_max * 1e3);
        }
    }

    // Warnings: the shapes that usually explain a slow rule.
    let _ = writeln!(r, "\n== Warnings");
    let mut warned = 0;
    if !state.hazards.is_empty() {
        let runs: u64 = state.hazards.values().map(|v| v.0).sum();
        let _ = writeln!(r, "- {} SQL statements ({runs} runs) force waits inside the waves: see == Forces SQL waits.", state.hazards.len());
        warned += 1;
    }
    for ((p, text), s) in &sql {
        if s.autoindex > 0 {
            let _ = writeln!(r, "- automatic index ({} rows) built for a statement in {p}: an index is missing.\n    {}", s.autoindex, short(text, 300));
            warned += 1;
        }
        if s.vm > 0 && s.scan * 2 > s.vm && s.vm > 1_000_000 {
            let _ = writeln!(r, "- full scans are {:.0}% of {} VM steps in {p}: a join reads a whole table.\n    {}", 100.0 * s.scan as f64 / s.vm as f64, s.vm, short(text, 300));
            warned += 1;
        }
        if s.runs > 1000 && s.ns / s.runs.max(1) < 20_000 {
            let _ = writeln!(r, "- {} runs of one statement in {p}: per-row or per-chunk statement overhead.\n    {}", s.runs, short(text, 200));
            warned += 1;
        }
    }
    for ((p, plan), steps) in &native {
        // Step 0 is the seed (one binding a chunk), so its step is the first atom's scan, not a fan-out.
        for (i, w) in steps.windows(2).enumerate().skip(1) {
            if w[1].0 > 0 && w[0].0 > 0 && w[1].0 > 20 * w[0].0 && w[1].0 > 1_000_000 {
                let _ = writeln!(r, "- native step {} fans out {}x ({} -> {} rows) in {p}:\n    {}", i + 1, w[1].0 / w[0].0, w[0].0, w[1].0, short(plan, 300));
                warned += 1;
            }
        }
    }
    // Planning that is a visible share of the run: one warning, its costliest plans named.
    let mut slow: Vec<(&(String, String, String), &PlanTotal)> = state.plans.iter().filter(|(_, t)| t.planning > 0.01 * roots).collect();
    slow.sort_by(|a, b| b.1.planning.total_cmp(&a.1.planning));
    if !slow.is_empty() {
        let total: f64 = slow.iter().map(|(_, t)| t.planning).sum();
        let _ = writeln!(
            r,
            "- planning takes {total:.2} s CPU ({:.0}% of the run) in {} plans, each over 1% of the run; each is planned again per loop (chunk). A plan reused across chunks or a bounded search would remove it. The costliest:",
            100.0 * total / roots.max(1e-9),
            slow.len()
        );
        for ((key, kind, _), t) in slow.iter().take(5) {
            let _ = writeln!(
                r,
                "    {} in {key}: {:.2} s over {} loops, {} literals, {} subsets and {} candidates a plan",
                rule_name(t.rule, kind),
                t.planning,
                t.loops,
                t.plan.steps.len(),
                t.plan.subsets,
                t.plan.candidates
            );
        }
        warned += 1;
    }
    for ((key, kind, _), t) in &state.plans {
        for (i, (st, a)) in t.plan.steps.iter().zip(&t.actual).enumerate() {
            let est = st.rows * t.loops as f64;
            let got = a.1 as f64;
            if t.measured && (got > 100.0 * est.max(1.0) && got > 100_000.0) {
                let _ = writeln!(r, "- {} in {key}: step {} ({}) estimated {est:.0} rows, produced {got:.0}: the estimate is off by over 100x.", rule_name(t.rule, kind), i + 1, st.literal);
                warned += 1;
            }
        }
    }
    for ((p, k), d) in &state.devices {
        // Over a second waiting on the device.
        if d.wall > 1.0 {
            let _ = writeln!(
                r,
                "- {k} in {p}: {:.2} s of GPU time for {} cells ({:.0} ns a cell); on a shared GPU the time includes other work queued ahead.",
                d.gpu,
                d.cells,
                1e9 * d.gpu / (d.cells.max(1) as f64)
            );
            warned += 1;
        }
    }
    for ((topic, text), _) in &state.decisions {
        if topic == "fallback" {
            let _ = writeln!(r, "- fallback: {text}");
            warned += 1;
        }
    }
    if warned == 0 {
        r.push_str("(none)\n");
    }
    let _ = writeln!(r, "\n== Notes");
    for (k, v) in state.notes.iter().filter(|(k, _)| !k.starts_with("plan:")) {
        let _ = writeln!(r, "{k}: {}", short(v, 600));
    }
    std::fs::write(dir.join("profile.txt"), r)
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn spans_nest_and_charge_their_own_cpu() {
        let _serial = one_run();
        set(true);
        reset();
        {
            let _a = span(|| "a".into());
            {
                let _b = span(|| "b".into());
                let mut x = 0u64;
                for i in 0..20_000_000u64 {
                    x = x.wrapping_mul(31).wrapping_add(i);
                }
                std::hint::black_box(x);
                sql("SELECT 1".into(), 10, 5, 0, 0, 0);
            }
        }
        let s = spans();
        set(false);
        assert!(s["a/b"].total > 0.0 && s["a"].total >= s["a/b"].total);
        assert!(s["a"].own < s["a"].total);
        // Other tests' statements may land while profiling is on: this one's is there.
        assert!(statements().iter().any(|(p, t, _)| p == "a/b" && t == "SELECT 1"));
    }

    /// Decision 0084: a span charges the bytes its thread allocated, its own apart from its children's, and what it
    /// freed; a dump lists the spans still open on every thread, with the SQL statement running.
    #[test]
    fn the_memory_sampler_charges_a_rise_to_the_span_open_on_its_thread() {
        let _serial = one_run();
        set(true);
        {
            let _s = span(|| "sampler probe: a big allocation".into());
            // Touched pages, held for several sampling periods, well above anything before it in this test process.
            let big = std::hint::black_box(vec![7u8; 256 << 20]);
            std::thread::sleep(std::time::Duration::from_millis(120));
            drop(big);
        }
        let s = memory::SAMPLED.lock().unwrap();
        assert!(s.every_ms > 0);
        let charged: Vec<&String> = s.rise.iter().filter(|(p, (r, _))| p.ends_with("sampler probe: a big allocation") && *r > 0).map(|(p, _)| p).collect();
        assert!(!charged.is_empty(), "the rise is charged to the open span: {:?}", s.rise.keys().collect::<Vec<_>>());
        drop(s);
        set(false);
    }

    #[test]
    fn spans_charge_memory_and_a_dump_lists_what_is_open() {
        let _serial = one_run();
        set(true);
        reset();
        let mb = 1 << 20;
        {
            let _outer = span(|| "outer".into());
            let kept = std::hint::black_box(vec![1u8; 8 * mb]);
            {
                let _inner = span(|| "inner".into());
                drop(std::hint::black_box(vec![2u8; 16 * mb]));
            }
            drop(kept);
        }
        // Another thread, stopped inside a span with a statement running, while this one dumps.
        let (opened, release) = (std::sync::mpsc::channel(), std::sync::mpsc::channel::<()>());
        let holder = std::thread::Builder::new()
            .name("holder".into())
            .spawn(move || {
                let _held = span(|| "held open".into());
                statement(Some("SELECT 42".into()));
                opened.0.send(()).unwrap();
                release.1.recv().unwrap();
                statement(None);
            })
            .unwrap();
        opened.1.recv().unwrap();
        let open = open_spans();
        release.0.send(()).unwrap();
        holder.join().unwrap();
        let s = spans();
        set(false);
        let jemalloc = memory::Counters::mine().read() != (0, 0);
        if jemalloc {
            assert!(s["outer/inner"].alloc >= 16 * mb as u64, "{}", s["outer/inner"].alloc);
            assert!(s["outer"].alloc >= 24 * mb as u64 && s["outer"].own_alloc >= 8 * mb as u64 && s["outer"].own_alloc < 16 * mb as u64);
            assert!(s["outer"].freed >= 24 * mb as u64);
        }
        assert!(open.contains("thread \"holder\"") && open.contains("held open") && open.contains("SQL running") && open.contains("SELECT 42"), "{open}");
    }

    #[test]
    fn explain_shows_engines_plans_devices_and_decisions() {
        let _serial = one_run();
        set(true);
        reset();
        decision("xtopic", || "2 chunks".into());
        stratum("xstratum", "native set engine", || "every rule has a native step".into());
        stratum("xstratum", "native set engine", String::new);
        let plan = RulePlan {
            work: 10.0,
            rows: 2.0,
            subsets: 4,
            candidates: 6,
            steps: vec![
                StepPlan { literal: "a($x)".into(), access: "scan".into(), source: "input".into(), rows: 5.0 },
                StepPlan { literal: "b($x)".into(), access: "lookup on (c0)".into(), source: "derived".into(), rows: 2.0 },
            ],
        };
        planned("xstratum", "seed xrule", Some(3), &plan, 0.01, 0, Some(&[(1, 5), (5, 2)]), None);
        planned("xstratum", "seed xrule", Some(3), &plan, 0.01, 1, Some(&[(1, 4), (4, 1)]), None);
        {
            let _s = span(|| "xspan".into());
            device("xkernel", 1, 0.002, 0.003, 100);
        }
        // A stratum that fell back to SQLite, and its span.
        stratum("xsql", "SQL wave kernel", || "the native engine declined".into());
        fallback("xsql", "aggregate join (string concatenation)".into());
        drop(span(|| "stratum xsql".into()));
        // SQL before the waves is the run's beginning; inside them, from a wave's thread, a hazard; from the store's
        // writer (behind the run), none.
        sql_in_wave("run", "SELECT 1 FROM xbefore", 1_000);
        {
            let _w = waves();
            let _s = span(|| "xwave".into());
            sql_in_wave("store", "BEGIN IMMEDIATE", 2_000_000);
            std::thread::spawn(|| {
                behind_the_run();
                sql_in_wave("store", "INSERT INTO xbehind VALUES (1)", 1_000);
            })
            .join()
            .unwrap();
        }
        sql_in_wave("run", "SELECT 1 FROM xafter", 1_000);
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("explain");
        write(&dir, "header\n").unwrap();
        let report = std::fs::read_to_string(dir.join("profile.txt")).unwrap();
        set(false);
        assert!(report.contains("Stratum xstratum  [engine: native set engine]  (loops=2"), "{report}");
        assert!(report.contains("why: every rule has a native step"));
        assert!(report.contains("== Fell back to SQLite: 1 strata"), "{report}");
        assert!(report.contains("== Forces SQL waits (hazards): 1 statements, 1 runs, 2.0 ms inside the waves"), "{report}");
        assert!(report.contains("a write to the store on a wave's thread") && report.contains("xwave"), "{report}");
        assert!(!report.contains("xbefore") && !report.contains("xafter") && !report.contains("xbehind"), "{report}");
        assert!(report.contains("xsql: aggregate join (string concatenation)"), "{report}");
        assert!(report.contains("rule #0003 (seed xrule)") && report.contains("loops=2 replans=1"));
        // Actual rows add over the loops.
        assert!(report.contains("actual 9 -> 3"), "{report}");
        assert!(report.contains("xkernel  (xspan)"));
        assert!(report.contains("xtopic:\n  - 2 chunks"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
