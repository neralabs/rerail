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

mod memory;
mod roofline;
mod watch;
pub(in crate::code_rule_dsl) use roofline::Unit;
pub use watch::watch;

static ON: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(std::env::var_os("RERAIL_PROFILE").is_some()));

/// Whether this run is profiled.
pub(in crate::code_rule_dsl) fn on() -> bool {
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
    let (u, s) = super::plan::corpus::thread_usage();
    u + s
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
    notes: BTreeMap<String, String>,
    /// Run-level decisions by (topic, text): how many times made.
    decisions: BTreeMap<(String, String), u64>,
    /// Strata in the order first run: key, engine, why, loops.
    strata: Vec<Stratum>,
    /// Plans by (stratum, kind, plan text).
    plans: BTreeMap<(String, String, String), PlanTotal>,
    /// Device kernels by (span path, kernel).
    devices: BTreeMap<(String, String), DeviceTotal>,
    /// Work charged by (span path, unit) (`work`).
    work: BTreeMap<(String, Unit), u64>,
    /// Waits by (the waiter's span path, what it waited on, what that was doing when the wait began) (`wait`).
    waits: BTreeMap<(String, String, String), WaitTotal>,
}

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
    STACK.with(|s| {
        let mut s = s.lock();
        let path = match s.opens.last() {
            Some(top) => format!("{}/{name}", top.path),
            None => name,
        };
        let alloc = s.counters.read();
        s.opens.push(Open { path, start: cpu(), children: 0.0, alloc, peak: memory::footprint().1, children_alloc: 0, children_rise: 0 });
    });
    Span(true)
}

impl Drop for Span {
    fn drop(&mut self) {
        if !self.0 {
            return;
        }
        let now = cpu();
        let peak = memory::footprint().1;
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

/// Charges `n` units of `unit` work to the current span (the roofline). Off, it costs one load.
pub(in crate::code_rule_dsl) fn work(unit: Unit, n: u64) {
    if !on() || n == 0 {
        return;
    }
    let path = current();
    *STATE.lock().unwrap().work.entry((path, unit)).or_default() += n;
}

/// Adds a native plan's rows in and out per step to the current span.
pub(in crate::code_rule_dsl) fn native(plan: String, steps: &[(u64, u64)]) {
    let path = current();
    let mut state = STATE.lock().unwrap();
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
    let mut native_tsv = String::from("span\twork\tsteps_in_out\tplan\n");
    for ((p, plan), steps) in &native {
        let s: Vec<String> = steps.iter().map(|(a, b)| format!("{a}:{b}")).collect();
        let _ = writeln!(native_tsv, "{p}\t{}\t{}\t{plan}", work(steps), s.join(","));
    }
    std::fs::write(dir.join("native.tsv"), native_tsv)?;

    // The report.
    let mut r = String::new();
    r.push_str(header);
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

    let _ = writeln!(r, "\n== Native joins by work (rows into every step, summed over chunks); steps in:out");
    for ((p, plan), steps) in native.iter().take(30) {
        let s: Vec<String> = steps.iter().map(|(a, b)| format!("{a}:{b}")).collect();
        let _ = writeln!(r, "{:>12}  {p}\n{:>14}steps {}\n{:>14}{}", work(steps), "", s.join(" "), "", short(plan, 600));
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

    // Warnings: the shapes that usually explain a slow rule.
    let _ = writeln!(r, "\n== Warnings");
    let mut warned = 0;
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
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("explain");
        write(&dir, "header\n").unwrap();
        let report = std::fs::read_to_string(dir.join("profile.txt")).unwrap();
        set(false);
        assert!(report.contains("Stratum xstratum  [engine: native set engine]  (loops=2"), "{report}");
        assert!(report.contains("why: every rule has a native step"));
        assert!(report.contains("rule #0003 (seed xrule)") && report.contains("loops=2 replans=1"));
        // Actual rows add over the loops.
        assert!(report.contains("actual 9 -> 3"), "{report}");
        assert!(report.contains("xkernel  (xspan)"));
        assert!(report.contains("xtopic:\n  - 2 chunks"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
