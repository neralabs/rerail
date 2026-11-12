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

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

pub mod analysis;
pub mod analysis_report;
pub mod kinds;
pub mod memory;
pub mod roofline;
// cratify: `watch` is rerail-logic's (it mounts it)
pub use roofline::Unit;
// cratify: `report` is rerail-logic's (it mounts it)

static ON: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(std::env::var_os("RERAIL_PROFILE").is_some()));

/// Whether this run is profiled.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Turns profiling on or off (a test that profiles without the environment variable).
#[allow(dead_code)]
pub fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

static DIR: LazyLock<Mutex<Option<std::path::PathBuf>>> = LazyLock::new(|| Mutex::new(std::env::var_os("RERAIL_PROFILE").map(Into::into)));

/// The directory the next report is written to: `RERAIL_PROFILE`'s, unless set (one per rule in a measurement).
pub fn directory() -> Option<std::path::PathBuf> {
    DIR.lock().unwrap().clone()
}

/// Sets the directory the next report is written to.
#[allow(dead_code)]
pub fn set_directory(dir: &Path) {
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
        let (u, s) = crate::code_rule_dsl::prof::thread_usage();
        u + s
    }
}

#[derive(Default, Clone)]
pub struct SpanTotal {
    pub calls: u64,
    pub total: f64,
    pub own: f64,
    /// Bytes its thread allocated while it was open, in all and less the spans inside it, and bytes freed
    /// (`memory`): allocated less freed is what it kept.
    pub alloc: u64,
    pub own_alloc: u64,
    pub freed: u64,
    /// How far the process's footprint high-water mark rose while it was open, in all and less the spans inside it.
    pub rise: u64,
    pub own_rise: u64,
}

#[derive(Default, Clone)]
pub struct SqlTotal {
    pub runs: u64,
    pub ns: u64,
    pub vm: u64,
    pub scan: u64,
    pub sort: u64,
    pub autoindex: u64,
}

#[derive(Default)]
pub struct State {
    pub spans: BTreeMap<String, SpanTotal>,
    /// By (span path, statement text).
    pub sql: BTreeMap<(String, String), SqlTotal>,
    /// By (span path, plan text): per step, rows in and out.
    pub native: BTreeMap<(String, String), Vec<(u64, u64)>>,
    /// By (span path, plan text): the CPU seconds its probes ran, as the native engine timed them.
    pub native_cpu: BTreeMap<(String, String), f64>,
    pub notes: BTreeMap<String, String>,
    /// Run-level decisions by (topic, text): how many times made.
    pub decisions: BTreeMap<(String, String), u64>,
    /// Strata in the order first run: key, engine, why, loops.
    strata: Vec<Stratum>,
    /// The strata that ran in SQLite (the SQL wave kernel), by key: the kind of literal the native engine declined.
    fallbacks: BTreeMap<String, String>,
    /// Plans by (stratum, kind, plan text).
    pub plans: BTreeMap<(String, String, String), PlanTotal>,
    /// Device kernels by (span path, kernel).
    pub devices: BTreeMap<(String, String), DeviceTotal>,
    /// Work charged by (span path, unit) (`work`).
    pub work: BTreeMap<(String, Unit), u64>,
    /// Waits by (the waiter's span path, what it waited on, what that was doing when the wait began) (`wait`).
    pub waits: BTreeMap<(String, String, String), WaitTotal>,
    /// Where each native stratum ran, by stratum (`placed`, decision 0095).
    pub placements: BTreeMap<String, PlacedTotal>,
    /// SQL inside the waves (`sql_in_wave`, `wait_in_wave`), by (hazard class, span path, connection and statement):
    /// runs and nanoseconds.
    pub hazards: BTreeMap<(&'static str, String, String), (u64, u64)>,
    /// What kept each stratum off the device and off the native engine (`executor`), by stratum.
    pub blocked: BTreeMap<String, Blocked>,
}

/// A stratum's executors: where it ran, the native engine's reason to decline it, what kept it off the device.
#[derive(Default)]
pub struct Blocked {
    pub ran: BTreeMap<&'static str, u64>,
    pub native: BTreeSet<String>,
    pub device: BTreeSet<String>,
}

/// A stratum's placements, summed over its runs (chunks, guests, the cross-file part).
#[derive(Default)]
pub struct PlacedTotal {
    pub runs: u64,
    pub on_gpu: u64,
    pub gpu_cheaper: u64,
    pub device: bool,
    pub forms: Vec<&'static str>,
    pub missing: Vec<&'static str>,
    pub planned_work: f64,
    pub work: u64,
    pub cpu_estimate: f64,
    pub gpu_estimate: f64,
    pub cpu: f64,
    /// Its join steps by the form placement chose (CPU index, GPU sort and probe, GPU tile): how many, their
    /// estimate on the CPU's index, and on the form chosen (of the steps estimated under `JOIN_ESTIMATE_CAP`), and
    /// how many were estimated over it.
    pub joins: [(u64, f64, f64, u64); 3],
    /// Its runs on the device (decision 0095, step 4), summed: rules, stages, submissions, dispatches, the GPU's
    /// time, this thread's CPU encoding, waiting and reading back, uploads, residency hits; and why the device
    /// refused it, when it was placed there and did not run there.
    pub gpu: GpuTotal,
    pub refused: Option<String>,
}

#[derive(Default, Clone)]
pub struct GpuTotal {
    pub runs: u64,
    pub rules: u64,
    pub stages: u64,
    pub submissions: u64,
    pub dispatches: u64,
    pub reruns: u64,
    pub gpu: f64,
    pub encode: f64,
    pub upload: f64,
    pub wait: f64,
    pub wait_wall: f64,
    pub decode: f64,
    pub uploads: u64,
    pub upload_bytes: u64,
    pub resident: u64,
    pub sorts: u64,
    pub sort_hits: u64,
    pub rows: u64,
    pub distinct: u64,
    pub bindings: u64,
    /// Texts made on the device into heads, stored as symbols.
    pub made: u64,
    /// Runs whose joins went out in their level's wave (one commit and wait for the level's strata).
    pub waved: u64,
    /// Batches run again because their text arena filled.
    pub arena_reruns: u64,
    /// Stages whose grids the device sized.
    pub sized: u64,
}

impl GpuTotal {
    pub fn add(&mut self, o: &GpuTotal) {
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
        self.made += o.made;
        self.waved += o.waved;
        self.arena_reruns += o.arena_reruns;
        self.sized += o.sized;
    }

    /// The host's CPU on the device's work: encoding, uploads, waiting, reading back.
    fn host(&self) -> f64 {
        self.encode + self.upload + self.wait + self.decode
    }
}

/// What the device reported for a stratum (`logic::device`).
pub type DeviceReport = Report;

#[derive(Default)]
pub struct WaitTotal {
    pub count: u64,
    /// Wall time blocked, and the waiter's CPU meanwhile (spinning, waking).
    pub wall: f64,
    pub cpu: f64,
}

/// What a thread is doing, for the threads that wait on it (`wait`): one shared marker, set as it moves between
/// phases. Cheap to set: a store of a static name.
#[derive(Default)]
pub struct Activity(parking_lot::Mutex<&'static str>);

impl Activity {
    pub fn set(&self, doing: &'static str) {
        *self.0.lock() = doing;
    }
    pub(in crate::code_rule_dsl) fn get(&self) -> &'static str {
        *self.0.lock()
    }
}

/// Runs `blocking` (a receive, a send on a full channel, a join), recorded as a wait of this thread's span on
/// `awaited`, and on what it was doing when it began (`doing`, an `Activity` of the awaited thread or a fixed name). The wall
/// time is the wait; the CPU spent meanwhile is reported with it. Not profiling, it only runs `blocking`.
pub fn wait<T>(awaited: &str, doing: &Activity, blocking: impl FnOnce() -> T) -> T {
    if !on() {
        return blocking();
    }
    let at = doing.get();
    let (wall0, cpu0) = (std::time::Instant::now(), cpu());
    let out = blocking();
    let (wall, spent) = (wall0.elapsed().as_secs_f64(), cpu() - cpu0);
    let path = current();
    let mut state = state();
    let w = state.waits.entry((path, awaited.to_owned(), if at.is_empty() { "-".to_owned() } else { at.to_owned() })).or_default();
    w.count += 1;
    w.wall += wall;
    w.cpu += spent;
    out
}

pub struct Stratum {
    key: String,
    engine: String,
    why: String,
    loops: u64,
}

/// One planned step: the literal, how it is read, from what, and the rows the planner expects after it; for a join,
/// the joins it passed over there, each with the rows a binding it was priced at (why this one was taken).
#[derive(Clone, Debug)]
pub struct StepPlan {
    pub literal: String,
    pub access: String,
    pub source: String,
    pub rows: f64,
    pub passed: Vec<(String, f64)>,
}

/// A rule's plan as the planner chose it: estimated work and rows, and its search (subsets kept, candidates priced).
#[derive(Clone, Debug, Default)]
pub struct RulePlan {
    pub work: f64,
    pub rows: f64,
    pub subsets: u64,
    pub candidates: u64,
    pub steps: Vec<StepPlan>,
}

#[derive(Default)]
pub struct PlanTotal {
    pub rule: Option<usize>,
    pub plan: RulePlan,
    pub loops: u64,
    pub planning: f64,
    replans: u64,
    pub actual: Vec<(u64, u64)>,
    pub measured: bool,
    produced: Option<u64>,
}

#[derive(Default)]
pub struct DeviceTotal {
    pub runs: u64,
    pub launches: u64,
    pub gpu: f64,
    pub wall: f64,
    pub cells: u64,
    /// Where the host's time went, summed (`device_phases`): seconds per `PHASES` entry.
    pub phases: [f64; PHASES.len()],
    /// Bytes of buffers made for the runs, and of buffers reused.
    pub made: u64,
    pub reused: u64,
}

/// A device run's host-side phases, in order (`device_phases`).
pub const PHASES: [&str; 7] = [
    "buffers made (map, wire, copy)",
    "command queue made",
    "encode",
    "submit (commit to kernelStartTime)",
    "schedule (kernelStartTime to GPUStartTime: residency)",
    "GPU",
    "wake (GPUEndTime to wait returning)",
];

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

thread_local! {
    /// Whether this thread holds the profile's state (`state`).
    static HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The profile's state, held: the only way to it. Its lock is not reentrant, so a profile call made while this
/// thread holds it (from a closure run under the lock: a decision's text that splits a program, which opens a span)
/// would wait on itself forever. Every closure a profile function takes is run before the lock, and a call that would
/// wait on itself panics here instead, naming the cause.
pub struct Held(std::sync::MutexGuard<'static, State>);

impl std::ops::Deref for Held {
    type Target = State;
    fn deref(&self) -> &State {
        &self.0
    }
}

impl std::ops::DerefMut for Held {
    fn deref_mut(&mut self) -> &mut State {
        &mut self.0
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        HELD.with(|h| h.set(false));
    }
}

pub fn state() -> Held {
    assert!(!HELD.with(|h| h.replace(true)), "the profile's state was taken again on a thread holding it: a closure ran under its lock");
    Held(STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
}

pub struct Open {
    path: String,
    start: f64,
    children: f64,
    /// The thread's allocated and freed bytes, and the footprint's high-water mark, when it opened; what the spans
    /// inside it allocated and raised.
    alloc: (u64, u64),
    peak: u64,
    children_alloc: u64,
    children_rise: u64,
    /// Its id in the heap sampler (`rerail_prof::heap::intern`; 0 off).
    heap: u32,
}

/// A thread's open spans and the SQL statement it runs, where a dump on another thread can read them (`watch`).
pub struct Thread {
    name: String,
    /// Its Mach port (macOS), for its CPU read from another thread; 0 elsewhere.
    port: u32,
    counters: memory::Counters,
    opens: Vec<Open>,
    /// The statement SQLite is running for it, and since when (`strict_statement`).
    statement: Option<(String, std::time::Instant)>,
    /// The CPU its outermost spans took: what of its CPU the spans account for (`threads`).
    covered: f64,
    /// The outermost span that closed last, and the thread's CPU then.
    between: Option<(String, f64)>,
    /// Its CPU outside every span, by the outermost spans around it: (closed before, opened after) (`gaps`).
    gaps: HashMap<(String, String), f64>,
}

/// Where a thread was outside every span before its first span opened, and after its last closed.
const THREAD_START: &str = "(the thread's start)";
const THREAD_END: &str = "(the thread's end, or now)";

/// A thread that opened a span leaves its account when it ends (`threads`): its CPU and its spans' then.
impl Drop for Thread {
    fn drop(&mut self) {
        if let Some(cpu) = thread_cpu(self.port) {
            EXITED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((self.name.clone(), cpu, self.covered));
            let mut gaps = std::mem::take(&mut self.gaps);
            if let Some((after, at)) = self.between.take() {
                *gaps.entry((after, THREAD_END.to_owned())).or_default() += (cpu - at).max(0.0);
            }
            let mut ended = EXITED_GAPS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            for (k, v) in gaps {
                *ended.entry(k).or_default() += v;
            }
        }
    }
}

/// The gaps of threads that have ended (`gaps`).
static EXITED_GAPS: LazyLock<Mutex<HashMap<(String, String), f64>>> = LazyLock::new(Default::default);

/// CPU a thread took outside every span, between the outermost span that closed before it (`after`) and the one that
/// opened after it (`before`), summed over the threads.
#[derive(Clone, Debug)]
pub struct Gap {
    pub after: String,
    pub before: String,
    pub cpu: f64,
}

/// Every thread's CPU outside its spans, by the spans around it, largest first: ended threads' and live ones' (their
/// CPU since their last span closed counted as before their end).
pub fn gaps() -> Vec<Gap> {
    let mut all = EXITED_GAPS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    let live: Vec<_> = THREADS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().filter_map(std::sync::Weak::upgrade).collect();
    for t in live {
        let t = t.lock();
        for (k, v) in &t.gaps {
            *all.entry(k.clone()).or_default() += v;
        }
        if t.opens.is_empty()
            && let (Some((after, at)), Some(now)) = (&t.between, thread_cpu(t.port))
        {
            *all.entry((after.clone(), THREAD_END.to_owned())).or_default() += (now - at).max(0.0);
        }
    }
    let mut out: Vec<Gap> = all.into_iter().map(|((after, before), cpu)| Gap { after, before, cpu }).collect();
    out.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
    out
}

/// The accounts of threads that opened spans and have ended: name, CPU, CPU under spans.
static EXITED: LazyLock<Mutex<Vec<(String, f64, f64)>>> = LazyLock::new(Default::default);

/// One thread's CPU, and what of it its spans took.
#[derive(Clone, Debug)]
pub struct ThreadCpu {
    pub name: String,
    pub cpu: f64,
    pub covered: f64,
}

/// The process's CPU by thread (`threads`): every live thread, the ended ones that opened spans, and a row for the
/// threads that ended without one (the process's CPU less all the others'), so the rows add up to `process`.
#[derive(Clone, Debug, Default)]
pub struct Threads {
    pub process: f64,
    pub rows: Vec<ThreadCpu>,
}

/// The process's CPU (user + system) so far.
pub fn process_cpu() -> f64 {
    #[cfg(unix)]
    {
        let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
        // SAFETY: fills a `rusage` for this process.
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1e6;
        t(usage.ru_utime) + t(usage.ru_stime)
    }
    #[cfg(not(unix))]
    {
        0.0
    }
}

/// The live threads' Mach ports and names (macOS; none elsewhere).
fn live_threads() -> Vec<(u32, String)> {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn mach_port_deallocate(task: libc::mach_port_t, name: libc::mach_port_t) -> libc::kern_return_t;
        }
        let mut list: libc::thread_act_array_t = std::ptr::null_mut();
        let mut count: libc::mach_msg_type_number_t = 0;
        // SAFETY: the task's own threads; the list and each port's right are given back below.
        let task = unsafe { libc::mach_task_self() };
        if unsafe { libc::task_threads(task, &mut list, &mut count) } != 0 {
            return Vec::new();
        }
        let ports: Vec<u32> = (0..count as usize).map(|i| unsafe { *list.add(i) }).collect();
        let named = ports
            .iter()
            .map(|&port| {
                let mut name = [0 as libc::c_char; 64];
                // SAFETY: a thread of this task; a thread that ended meanwhile gives a null pthread.
                let thread = unsafe { libc::pthread_from_mach_thread_np(port) };
                let name = if thread == 0 || unsafe { libc::pthread_getname_np(thread, name.as_mut_ptr(), name.len()) } != 0 {
                    String::new()
                } else {
                    unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }.to_string_lossy().into_owned()
                };
                (port, if name.is_empty() { "unnamed".to_owned() } else { name })
            })
            .collect();
        for &port in &ports {
            unsafe { mach_port_deallocate(task, port) };
        }
        unsafe { libc::vm_deallocate(task, list as libc::vm_address_t, (count as usize * std::mem::size_of::<libc::thread_act_t>()) as libc::vm_size_t) };
        named
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// The process's CPU now, by thread: each live thread's, with what its outermost spans took, and the ended ones'.
/// A span's CPU is its own thread's, so work it hands to another thread (rayon's, a helper's) is under no span unless
/// that thread opens one (`span_at`): this says where such CPU went.
pub fn threads() -> Threads {
    let process = process_cpu();
    let registered: Vec<(u32, f64)> = THREADS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .filter_map(std::sync::Weak::upgrade)
        .map(|t| {
            let t = t.lock();
            // Spans open now count what they have taken so far.
            let open = t.opens.first().map_or(0.0, |o| thread_cpu(t.port).map_or(0.0, |now| now - o.start));
            (t.port, t.covered + open)
        })
        .collect();
    let mut rows: Vec<ThreadCpu> = live_threads()
        .into_iter()
        .filter_map(|(port, name)| {
            let cpu = thread_cpu(port)?;
            let covered = registered.iter().find(|(p, _)| *p == port).map_or(0.0, |(_, c)| *c);
            Some(ThreadCpu { name, cpu, covered })
        })
        .collect();
    let live: f64 = rows.iter().map(|t| t.cpu).sum();
    let ended = EXITED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    let named: f64 = ended.iter().map(|(_, cpu, _)| cpu).sum();
    rows.extend(ended.into_iter().map(|(name, cpu, covered)| ThreadCpu { name: format!("{name} (ended)"), cpu, covered }));
    let unnamed = process - live - named;
    if unnamed > 0.005 {
        rows.push(ThreadCpu { name: "ended threads that opened no span".to_owned(), cpu: unnamed, covered: 0.0 });
    }
    Threads { process, rows }
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
        covered: 0.0,
        between: None,
        gaps: HashMap::new(),
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
pub fn statement(text: Option<String>) {
    if on() {
        STACK.with(|s| s.lock().statement = text.map(|t| (t, std::time::Instant::now())));
    }
}

/// An open span; it closes when dropped.
pub struct Span(bool);

/// This thread's innermost open span's path (only when profiling): a worker the thread starts opens its spans under
/// it (`span_at`), so the worker's work is charged to what started it (a phase's run, say).
pub fn here() -> Option<String> {
    if !on() {
        return None;
    }
    STACK.with(|s| s.lock().opens.last().map(|o| o.path.clone()))
}

/// Opens span `name` under `under`, a path another thread's `here` gave: a pool job's span (`crate::threads`), at
/// that path whichever thread runs it. Without `under`, under this thread's open span, as `span` opens one.
/// `f` for a parallel iterator's items, each a sizeable piece of work (a file, a tape): on a thread with no span
/// open (a pool thread that took the item), under `name` in the span open where `carried` was called; on a thread
/// with one open (the caller's), as it is. Rayon's work taken by another thread is under no span of that thread's.
pub fn carried<A, R>(name: &'static str, f: impl Fn(A) -> R + Sync + Send) -> impl Fn(A) -> R + Sync + Send {
    let under = here();
    move |a| {
        let _span = (under.is_some() && STACK.with(|s| s.lock().opens.is_empty())).then(|| span_at(under.as_deref(), || name.to_owned()));
        f(a)
    }
}

pub fn span_at(under: Option<&str>, name: impl FnOnce() -> String) -> Span {
    match under {
        Some(p) => open(format!("{p}/{}", name())),
        None => span(name),
    }
}

/// Opens span `name` (made only when profiling) under this thread's open span.
pub fn span(name: impl FnOnce() -> String) -> Span {
    if !on() {
        return Span(false);
    }
    let name = name();
    let path = STACK.with(|s| match s.lock().opens.last() {
        Some(top) => format!("{}/{name}", top.path),
        None => name,
    });
    open(path)
}

/// Opens the span at `path`.
fn open(path: String) -> Span {
    if !on() {
        return Span(false);
    }
    memory::sample();
    STACK.with(|s| {
        let mut s = s.lock();
        let alloc = s.counters.read();
        let start = cpu();
        if s.opens.is_empty() {
            let (after, at) = s.between.take().unwrap_or_else(|| (THREAD_START.to_owned(), 0.0));
            if start > at {
                *s.gaps.entry((after, path.clone())).or_default() += start - at;
            }
        }
        // The heap sampler charges this thread's allocations to the span now (`rerail_prof::heap`).
        let heap = if rerail_prof::heap::on() { rerail_prof::heap::intern(&path) } else { 0 };
        rerail_prof::heap::set_span(heap);
        s.opens.push(Open { path, start, children: 0.0, alloc, peak: memory::peak_sampled(), children_alloc: 0, children_rise: 0, heap });
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
            rerail_prof::heap::set_span(s.opens.last().map_or(0, |o| o.heap));
            let total = now - open.start;
            if s.opens.is_empty() {
                s.covered += total;
                s.between = Some((open.path.clone(), now));
            }
            let alloc = allocated.saturating_sub(open.alloc.0);
            let rise = peak.saturating_sub(open.peak);
            if let Some(parent) = s.opens.last_mut() {
                parent.children += total;
                parent.children_alloc += alloc;
                parent.children_rise += rise;
            }
            let mut state = state();
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
pub fn peaks() -> String {
    let (now, max) = memory::footprint();
    // macOS reports bytes, Linux kilobytes; Windows has its peak working set, the browser build no counter.
    #[cfg(unix)]
    let rss = {
        let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
        // SAFETY: fills a `rusage` for this process.
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        if cfg!(target_os = "macos") { usage.ru_maxrss as f64 } else { usage.ru_maxrss as f64 * 1024.0 }
    };
    #[cfg(windows)]
    let rss = crate::code_rule_dsl::prof::windows_peak_working_set() as f64;
    #[cfg(target_family = "wasm")]
    let rss = 0.0;
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
pub fn sql(text: String, ns: u64, vm: u64, scan: u64, sort: u64, autoindex: u64) {
    let path = current();
    let mut state = state();
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
pub struct Waves(bool);

pub fn waves() -> Waves {
    WAVES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Waves(true)
}

/// Whether a run's waves are under way (`waves`): what is not read before the first wave is not read from SQLite.
pub fn inside_waves() -> bool {
    WAVES.load(std::sync::atomic::Ordering::Relaxed) > 0
}


/// `RERAIL_SQL_STRICT=1`, the benchmark's mode: SQL only where decision 0104 puts it. A run reads what it needs
/// before its first wave (each run of the process, outside every wave), every wave is given what it reads in memory, and what the run keeps is written behind it by
/// the store's writer. Any other statement, on any connection of the process, ends it at once (`strict_statement`), naming
/// the statement, its thread and span, and where it was sent from; so does a wave's wait for the writer.
pub fn strict() -> bool {
    static STRICT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *STRICT.get_or_init(|| std::env::var("RERAIL_SQL_STRICT").is_ok_and(|v| !matches!(v.as_str(), "" | "0" | "off")))
}

/// Whether the store's connections trace their statements: for the profile, or the strict mode.
pub fn tracing() -> bool {
    on() || strict()
}

/// Whether `text` only reads: a query, a pragma, or a read transaction's bounds.
fn reads_only(text: &str) -> bool {
    let t = text.trim_start().to_ascii_uppercase();
    let word = t.split(|c: char| !c.is_ascii_alphabetic()).next().unwrap_or("");
    match word {
        "SELECT" | "WITH" | "PRAGMA" | "EXPLAIN" | "VALUES" | "COMMIT" | "END" | "ROLLBACK" | "RELEASE" | "SAVEPOINT" => true,
        "BEGIN" => !t.contains("IMMEDIATE") && !t.contains("EXCLUSIVE"),
        _ => false,
    }
}

/// A statement about to run on connection `conn` (`run`, `store`, or `sqlite`: any other). In the strict mode
/// (`strict`), the process ends unless it is the store writer's, or a read before the process's first wave.
pub fn strict_statement(conn: &'static str, text: &str) {
    if !strict() || BEHIND.with(std::cell::Cell::get) {
        return;
    }
    // A run reads before its first wave; a process's runs (phase 0's include graphs, then the run) each do.
    let inside = WAVES.load(std::sync::atomic::Ordering::Relaxed) > 0;
    if !inside && reads_only(text) {
        return;
    }
    let when = if inside { "inside the waves" } else { "a write outside the waves, not the store writer's" };
    violation(when, &format!("{conn}: {}", text.split_whitespace().collect::<Vec<_>>().join(" ")));
}

/// Ends the process for the strict mode: what ran where, and the stack that sent it. An abort, not a panic: it is
/// raised inside SQLite's callback, which cannot unwind.
fn violation(when: &str, what: &str) -> ! {
    let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
    let mut what = what.to_owned();
    if what.len() > 600 {
        what.truncate(what.floor_char_boundary(600));
        what.push('…');
    }
    eprintln!(
        "rerail: RERAIL_SQL_STRICT: SQL {when} (decision 0104: one read before the first wave, writes behind the run)\n  {what}\n  thread {thread}, span {}\n{}",
        current(),
        std::backtrace::Backtrace::force_capture()
    );
    std::process::abort();
}

/// In the strict mode, every SQLite connection the process opens checks its statements (`strict_statement`): one that
/// sets a trace of its own (the store's, the run's) checks them there.
pub fn strict_on_every_connection() {
    use rusqlite::ffi;
    unsafe extern "C" fn trace(event: u32, _: *mut std::ffi::c_void, p: *mut std::ffi::c_void, _: *mut std::ffi::c_void) -> std::ffi::c_int {
        if event == ffi::SQLITE_TRACE_STMT as u32 {
            // SAFETY: for SQLITE_TRACE_STMT, P is the statement.
            let text = unsafe { std::ffi::CStr::from_ptr(ffi::sqlite3_sql(p.cast::<ffi::sqlite3_stmt>())) }.to_string_lossy();
            strict_statement("sqlite", &text);
        }
        0
    }
    unsafe extern "C" fn opened(db: *mut ffi::sqlite3, _: *mut *mut std::ffi::c_char, _: *const ffi::sqlite3_api_routines) -> std::ffi::c_int {
        // SAFETY: SQLite calls this with a connection it has just opened.
        unsafe { ffi::sqlite3_trace_v2(db, ffi::SQLITE_TRACE_STMT as u32, Some(trace), std::ptr::null_mut()) };
        ffi::SQLITE_OK
    }
    if strict() {
        // SAFETY: registers a function with the signature SQLite calls; it outlives every connection.
        unsafe { ffi::sqlite3_auto_extension(Some(opened)) };
    }
}

impl Drop for Waves {
    fn drop(&mut self) {
        if self.0 {
            WAVES.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// Marks this thread as working behind the run (the store's writer).
pub fn behind_the_run() {
    BEHIND.with(|b| b.set(true));
}

/// Runs `f` on this thread as work behind the run: a write after the run's last wave, as a command's end flushes its
/// cost samples (decision 0104).
pub fn behind<T>(f: impl FnOnce() -> T) -> T {
    let was = BEHIND.with(|b| b.replace(true));
    let out = f();
    BEHIND.with(|b| b.set(was));
    out
}

fn in_waves() -> bool {
    on() && WAVES.load(std::sync::atomic::Ordering::Relaxed) > 0 && !BEHIND.with(std::cell::Cell::get)
}

/// In the strict mode a wave's wait for the writer ends the process, as its SQL would (`strict_statement`); the run's last
/// wait, for every queued write, is the end of the run.
fn strict_wait(what: &str) {
    if strict() && !BEHIND.with(std::cell::Cell::get) && WAVES.load(std::sync::atomic::Ordering::Relaxed) > 0 {
        violation("inside the waves", &format!("a wait for the store's writer: {what}"));
    }
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
pub fn sql_in_wave(conn: &'static str, text: &str, ns: u64) {
    if !in_waves() {
        return;
    }
    let path = current();
    let class = hazard_class(conn, &path, text);
    let mut state = state();
    let e = state.hazards.entry((class, path, format!("{conn}: {text}"))).or_default();
    e.0 += 1;
    e.1 += ns;
}

/// SQL waits attributed to their connection and span, including the content store's separate connection.
/// Each row is (span, connection and SQL text, executions, nanoseconds).
pub fn sql_waits() -> Vec<(String, String, u64, u64)> {
    state().hazards.iter().map(|((_, path, text), (runs, ns))|
        (path.clone(), text.clone(), *runs, *ns)).collect()
}

/// A wave's thread waited `wall` seconds for SQLite's side of the store (`what`: a queued write it reads, the queue's
/// back-pressure): a hazard like a statement.
pub fn wait_in_wave(what: &str, wall: f64) {
    strict_wait(what);
    if !in_waves() {
        return;
    }
    let path = current();
    let mut state = state();
    let e = state.hazards.entry(("a wait for the store's writer", path, format!("store: {what}"))).or_default();
    e.0 += 1;
    e.1 += (wall * 1e9) as u64;
}

/// The hazards: SQL a wave's thread ran or waited on between the run's first wave and its last.
pub fn hazards(r: &mut String, state: &State) {
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
pub fn work(unit: Unit, n: u64) {
    if !on() || n == 0 {
        return;
    }
    let path = current();
    *state().work.entry((path, unit)).or_default() += n;
}

/// Adds a native plan's rows in and out per step, and its probes' CPU seconds, to the current span.
pub fn native(plan: String, steps: &[(u64, u64)], cpu_s: f64) {
    let path = current();
    let mut state = state();
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
pub fn note(key: String, text: impl FnOnce() -> String) {
    if state().notes.contains_key(&key) {
        return;
    }
    // Made without the state's lock: making it may profile too.
    let text = text();
    state().notes.entry(key).or_insert(text);
}

/// A statement's text as the SQL trace keys it: its words, single-spaced.
pub fn normal(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Notes `statement`'s query plan (`EXPLAIN QUERY PLAN`, once per text) when profiling.
pub fn plan(db: &rusqlite::Connection, statement: &str) {
    if !on() {
        return;
    }
    let key = format!("plan:{}", normal(statement));
    if state().notes.contains_key(&key) {
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
pub fn now() -> f64 {
    cpu()
}

/// A run-level decision or optimization under `topic` (counted when made again, as per chunk).
pub fn decision(topic: &str, text: impl FnOnce() -> String) {
    if !on() {
        return;
    }
    // The text first: making it may profile too (a split's span), and the state's lock is not reentrant.
    let text = text();
    *state().decisions.entry((topic.to_owned(), text)).or_default() += 1;
}

/// The decisions made so far (none when profiling is off): `decisions_since` takes the ones made after.
pub fn decisions() -> BTreeMap<(String, String), u64> {
    if !on() {
        return BTreeMap::new();
    }
    state().decisions.clone()
}

/// The decisions made since `before` (`decisions`), with how often: what a step decided, for a run that reuses the
/// step's result to state again (`add_decisions`).
pub fn decisions_since(before: &BTreeMap<(String, String), u64>) -> Vec<((String, String), u64)> {
    decisions().into_iter().filter_map(|(k, n)| n.checked_sub(before.get(&k).copied().unwrap_or(0)).filter(|d| *d > 0).map(|d| (k, d))).collect()
}

/// Decisions a reused step made when it ran (`decisions_since`), stated for this run.
pub(in crate::code_rule_dsl) fn add_decisions(made: &[((String, String), u64)]) {
    if !on() {
        return;
    }
    let mut state = state();
    for (k, n) in made {
        *state.decisions.entry(k.clone()).or_default() += n;
    }
}

/// The strata the device ran (decision 0095, step 4): in all, then the costliest; and the strata placed there that it
/// refused, by reason (no stratum is moved from the device to the CPU without a line here).
pub fn device_strata(r: &mut String, all: &BTreeMap<String, PlacedTotal>) {
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
    let _ = writeln!(r, "  GPU {:.1} ms (command buffers' timestamps) in {} submissions, {} dispatches ({} stages run again after an overflow, {} after the text arena filled); {} runs sent in their level's wave; {} stages' grids sized on the device",
        ms(sum.gpu), sum.submissions, sum.dispatches, sum.reruns, sum.arena_reruns, sum.waved, sum.sized);
    let _ = writeln!(r, "  host CPU {:.1} ms: encode {:.1}, uploads {:.1}, wait {:.1} ({:.1} ms wall), rows back into the store {:.1}",
        ms(sum.host()), ms(sum.encode), ms(sum.upload), ms(sum.wait), ms(sum.wait_wall), ms(sum.decode));
    let _ = writeln!(r, "  relations uploaded {} ({:.1} MB), found resident {}; sorts made {}, found resident {}; bindings {}, head rows {} ({} distinct, made so on the device); texts made {}",
        sum.uploads, sum.upload_bytes as f64 / 1e6, sum.resident, sum.sorts, sum.sort_hits, sum.bindings, sum.rows, sum.distinct, sum.made);
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
pub const JOIN_ESTIMATE_CAP: f64 = 1e3;

/// Records that stratum `key` fell back to SQLite, and the kind of literal the native engine declined (`class`).
/// A stratum SQLite is about to run inside the waves: under the strict mode, the end of the process, naming it and
/// what the native engine declined.
pub fn strict_declined(key: &str, why: &str, class: Option<&str>) {
    if strict() && inside_waves() {
        violation("inside the waves: a stratum the native engine declined", &format!("stratum {key}: {why}; {}", class.unwrap_or("no class")));
    }
}

pub fn fallback(key: &str, class: String) {
    if !on() {
        return;
    }
    state().fallbacks.insert(key.to_owned(), class);
}

/// The strata that fell back to SQLite, costliest first, and the same by the kind of literal declined: each one's
/// CPU (its `stratum …` spans, wherever they ran) and SQLite's VM steps. What the native engine lacks, by its cost.
pub fn fallbacks(r: &mut String, state: &State) {
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

/// Where stratum `key` ran (`"Metal"`, `"native CPU"`, `"SQLite"`), the native engine's reason to decline it
/// (SQLite's strata), and every hazard that kept it off the device: a form it lacks with the operation that needs
/// it, its refusal at run time, the price, the run's binding (`placement::record`, `record_fallback`).
pub fn executor(key: &str, ran: &'static str, native: Option<String>, device: Vec<String>) {
    if !on() {
        return;
    }
    let mut state = state();
    let b = state.blocked.entry(key.to_owned()).or_default();
    *b.ran.entry(ran).or_default() += 1;
    b.native.extend(native);
    b.device.extend(device);
}

/// Every stratum's executors and hazards: by hazard, costliest first (a stratum counts under each of its hazards),
/// then the strata. What to build next, on the device and in the native engine, by what it costs.
pub fn executors(r: &mut String, state: &State) {
    let cpu_of = |key: &str| -> f64 {
        let name = format!("stratum {key}");
        state.spans.iter().filter(|(p, _)| **p == name || p.ends_with(&format!("/{name}"))).map(|(_, t)| t.total).sum()
    };
    let rows: Vec<(f64, &String, &Blocked)> = state.blocked.iter().map(|(k, b)| (cpu_of(k), k, b)).collect();
    let mut ran: BTreeMap<&str, u64> = BTreeMap::new();
    for (_, _, b) in &rows {
        for (e, n) in &b.ran {
            *ran.entry(e).or_default() += n;
        }
    }
    let _ = writeln!(
        r,
        "\n== Executor hazards: what kept each stratum off Metal and off the native engine ({} strata; runs: {})",
        rows.len(),
        ran.iter().map(|(e, n)| format!("{n} {e}")).collect::<Vec<_>>().join(", ")
    );
    if rows.is_empty() {
        return;
    }
    let mut by: BTreeMap<String, (usize, f64)> = BTreeMap::new();
    for (cpu, _, b) in &rows {
        for h in b.native.iter().map(|n| format!("native: {n}")).chain(b.device.iter().map(|d| format!("metal: {d}"))) {
            let e = by.entry(h).or_default();
            e.0 += 1;
            e.1 += cpu;
        }
    }
    let mut by: Vec<_> = by.into_iter().collect();
    by.sort_by(|a, b| b.1.1.total_cmp(&a.1.1).then(b.1.0.cmp(&a.1.0)));
    let _ = writeln!(r, "By hazard (every one; a stratum counts under each of its hazards):");
    let _ = writeln!(r, "{:>9} {:>7}  hazard", "cpu s", "strata");
    for (h, (n, cpu)) in &by {
        let _ = writeln!(r, "{cpu:>9.3} {n:>7}  {h}");
    }
    let mut rows = rows;
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    let _ = writeln!(r, "Strata, costliest first (ran: where its runs went):");
    let _ = writeln!(r, "{:>9}  stratum: ran; hazards", "cpu s");
    for (cpu, key, b) in rows.iter().take(40) {
        let ran = b.ran.iter().map(|(e, n)| format!("{e} x{n}")).collect::<Vec<_>>().join(", ");
        let hazards: Vec<String> = b.native.iter().map(|n| format!("native: {n}")).chain(b.device.iter().map(|d| format!("metal: {d}"))).collect();
        let _ = writeln!(r, "{cpu:>9.3}  {}: {ran}; {}", short(key, 70), if hazards.is_empty() { "none".into() } else { hazards.join("; ") });
    }
}

pub fn stratum(key: &str, engine: &str, why: impl FnOnce() -> String) {
    if !on() {
        return;
    }
    // The engine changed between chunks: both are kept, the later under its own key.
    let other = format!("{key} (chunk with {engine})");
    let seen = |state: &mut State| -> Option<String> {
        match state.strata.iter_mut().find(|s| s.key == key) {
            Some(s) if s.engine == engine => s.loops += 1,
            Some(_) => match state.strata.iter_mut().find(|s| s.key == other) {
                Some(s) => s.loops += 1,
                None => return Some(other.clone()),
            },
            None => return Some(key.to_owned()),
        }
        None
    };
    let Some(new) = seen(&mut state()) else { return };
    // The reason is made without the state's lock: making it may profile too.
    let why = why();
    let mut state = state();
    match state.strata.iter_mut().find(|s| s.key == new) {
        Some(s) => s.loops += 1,
        None => state.strata.push(Stratum { key: new, engine: engine.to_owned(), why, loops: 1 }),
    }
}

/// A rule plan run in stratum `key`: `kind` (seed, join with its delta, SQL insert …), the rule's number, the CPU
/// its planning took, how often it was replanned, the actual rows into and out of each step where measured, and the
/// rows it produced where known. One loop per call; equal plans add up.
#[allow(clippy::too_many_arguments)]
pub fn planned(
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
    let mut state = state();
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
pub fn device(kernel: &str, launches: u64, gpu: f64, wall: f64, cells: u64) {
    if !on() {
        return;
    }
    let path = current();
    let mut state = state();
    let t = state.devices.entry((path, kernel.to_owned())).or_default();
    t.runs += 1;
    t.launches += launches;
    t.gpu += gpu;
    t.wall += wall;
    t.cells += cells;
}

/// A device run's host-side phases (seconds, in `PHASES` order) and the buffer bytes it made and reused, added to
/// the current span's entry for `kernel`.
pub fn device_phases(kernel: &str, phases: [f64; PHASES.len()], made: u64, reused: u64) {
    if !on() {
        return;
    }
    let path = current();
    let mut state = state();
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
pub fn one_run() -> std::sync::MutexGuard<'static, ()> {
    ONE_RUN.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Holds the profile for one run if no run holds it: a run nested in another (phase 0's, inside a run's preparing)
/// is then the outer run's, and neither resets nor writes the profile.
pub fn one_run_if_free() -> Option<std::sync::MutexGuard<'static, ()>> {
    match ONE_RUN.try_lock() {
        Ok(guard) => Some(guard),
        Err(std::sync::TryLockError::Poisoned(e)) => Some(e.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => None,
    }
}

/// Forgets everything recorded (a run's start).
pub fn reset() {
    *state() = State::default();
}

/// The statements recorded, by SQLite's time, most first: (span, text, totals).
pub(in crate::code_rule_dsl) fn statements() -> Vec<(String, String, SqlTotal)> {
    let state = state();
    let mut all: Vec<_> = state.sql.iter().map(|((p, t), s)| (p.clone(), t.clone(), s.clone())).collect();
    all.sort_by_key(|(_, _, s)| std::cmp::Reverse(s.ns));
    all
}

/// The spans recorded: path and totals.
pub fn spans() -> BTreeMap<String, SpanTotal> {
    state().spans.clone()
}

pub fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_owned() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

/// An estimate of rows: two decimals below 10, else whole.
fn rows(x: f64) -> String {
    if x < 10.0 { format!("{x:.2}") } else { format!("{x:.0}") }
}

pub fn rule_name(rule: Option<usize>, kind: &str) -> String {
    match rule {
        Some(k) => format!("rule #{k:04} ({kind})"),
        None => kind.to_owned(),
    }
}

/// The EXPLAIN ANALYZE section: decisions, then each stratum's engine and its plans, steps with estimates and actuals.
pub fn explain(r: &mut String, state: &State) {
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
                for (lit, per) in &st.passed {
                    let _ = writeln!(r, "           passed over {:<44} {per:.1} rows a binding", short(lit, 44));
                }
            }
        }
    }
    if state.strata.is_empty() {
        r.push_str("(no strata recorded)\n");
    }
}

/// The memory section: the process now, the spans by bytes they allocated themselves, and those that raised the
/// footprint's high-water mark (`memory`).
pub fn memory_report(r: &mut String, spans: &BTreeMap<String, SpanTotal>) {
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
pub fn open_spans() -> String {
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
            let plan = state().notes.get(&format!("plan:{}", normal(text))).cloned();
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

/// Record why expensive input relations are requested, before projection planning.
pub fn projection_plan(prog: &crate::code_rule_dsl::logic::front::Program, reads: &std::collections::BTreeSet<String>) {
    if on() { analysis_report::projection(prog, reads); }
}

#[cfg(test)]
pub mod tests {
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
        // The high-water mark is the process's: other tests in this process may have left it so high that the probe
        // cannot raise it. Then only the span's own allocation can be seen.
        let (now, mark) = memory::footprint();
        let can_rise = mark == 0 || now + (256 << 20) > mark + (32 << 20);
        {
            let _s = span(|| "sampler probe: a big allocation".into());
            // Touched pages, held for several sampling periods.
            let big = std::hint::black_box(vec![7u8; 256 << 20]);
            std::thread::sleep(std::time::Duration::from_millis(120));
            drop(big);
        }
        let s = memory::SAMPLED.lock().unwrap();
        assert!(s.every_ms > 0);
        if !can_rise {
            drop(s);
            let own = spans().into_iter().find(|(p, _)| p.ends_with("sampler probe: a big allocation")).map(|(_, t)| t.alloc);
            set(false);
            assert!(own.is_some_and(|a| a >= 256 << 20) || cfg!(any(windows, target_family = "wasm")), "the probe's own allocation: {own:?}");
            return;
        }
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

}

/// What a stratum's device run did, for the profile.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub rules: u32,
    /// Whether the stratum's joins went out in its level's wave (one commit and wait for the level's strata).
    pub waved: u32,
    /// Batches run again because their text arena filled (in one as big as their texts asked).
    pub arena_reruns: u32,
    /// Stages whose grids were sized on the device, from the rows before them (not the host's guess of their space).
    pub sized: u32,
    /// Text functions made once per distinct symbol (`mapped_text`, chunked), and the names they made.
    pub per_symbol: u32,
    pub per_symbol_names: u64,
    /// Texts made on the device (`cat` into a head) and stored as symbols.
    pub made: u64,
    /// The rounds a recursive component ran on the device (`fixpoint`; 0 for seeds alone).
    pub rounds: u32,
    pub stages: u32,
    pub submissions: u32,
    pub dispatches: u32,
    /// Runs again from an overflowing stage.
    pub reruns: u32,
    pub gpu_seconds: f64,
    /// This thread's CPU: encoding (uploads and sorts' encoding included), waiting, and reading the rows back into
    /// the store.
    pub encode_cpu: f64,
    pub wait_cpu: f64,
    pub wait_wall: f64,
    pub decode_cpu: f64,
    /// Head rows left once made distinct on the device (of `rows`).
    pub distinct: u64,
    pub upload_cpu: f64,
    /// Relations uploaded (and their bytes), found resident; sorts made, found resident.
    pub uploads: u32,
    pub upload_bytes: u64,
    pub resident_hits: u32,
    pub sorts: u32,
    pub sort_hits: u32,
    /// Rows the heads made (before the store's set), and bindings the stages made.
    pub rows: u64,
    pub bindings: u64,
    /// Why the device refused the stratum (at open, or at run time: a table past its budget), which then ran on the
    /// CPU; `None` when it ran here.
    pub refused: Option<String>,
}
