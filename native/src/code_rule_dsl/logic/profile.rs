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
pub mod life;
pub mod memory;
pub mod roofline;
pub mod sources;
// cratify: `watch` is rerail-logic's (it mounts it)
pub use roofline::Unit;
// cratify: `report` is rerail-logic's (it mounts it)

/// Whether the caller's runs are profiled (decision 0442: the caller's choice, `RERAIL_PROFILE` unless it binds one),
/// in the run's context and so in its jobs'.
#[derive(Clone, Copy)]
struct On(bool);
impl Default for On {
    fn default() -> Self {
        On(setting(|| std::env::var_os("RERAIL_PROFILE").is_some()))
    }
}

/// The directory the caller's next report is written to: `RERAIL_PROFILE`'s, unless it binds one.
#[derive(Clone)]
struct Dir(Option<std::path::PathBuf>);
impl Default for Dir {
    fn default() -> Self {
        Dir(setting(|| std::env::var_os("RERAIL_PROFILE").map(Into::into)))
    }
}

/// Whether this run is profiled.
pub fn on() -> bool {
    bound::<On>().0
}

/// Turns profiling of this caller's runs on or off (a test that profiles without the environment variable), for the
/// rest of the extent under way here.
pub fn set(on: bool) {
    bind_here(On(on));
}

/// The directory the next report is written to: `RERAIL_PROFILE`'s, unless set (one per rule in a measurement).
pub fn directory() -> Option<std::path::PathBuf> {
    bound::<Dir>().0
}

/// Sets the directory this caller's next report is written to.
pub fn set_directory(dir: &Path) {
    bind_here(Dir(Some(dir.to_path_buf())));
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
    /// By (span path, plan text): per step, and last its head's rows made, the CPU seconds its walks spent there
    /// (`setfix::step_cpu`), where the engine timed them.
    pub native_step_cpu: BTreeMap<(String, String), Vec<f64>>,
    pub notes: BTreeMap<String, String>,
    /// Run-level decisions by (topic, text): how many times made, or stated by a reused version that made them
    /// (`reused`, decision 0269).
    pub decisions: BTreeMap<(String, String), u64>,
    /// Of those, the ones a reused version stated, by where the run took the version from (`memory`, `store`).
    pub kept: BTreeMap<(String, String), &'static str>,
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

fn add_steps(into: &mut Vec<(u64, u64)>, from: &[(u64, u64)]) {
    if into.len() < from.len() {
        into.resize(from.len(), (0, 0));
    }
    for (a, b) in into.iter_mut().zip(from) {
        a.0 += b.0;
        a.1 += b.1;
    }
}

fn add_cpu(into: &mut Vec<f64>, from: &[f64]) {
    if into.len() < from.len() {
        into.resize(from.len(), 0.0);
    }
    for (a, b) in into.iter_mut().zip(from) {
        *a += b;
    }
}

impl State {
    /// Adds a thread's records (`merged`) as its calls would have added them here: sums summed, the first note,
    /// reason and refusal kept, a stratum's loops under its key (or its key with another engine, as `stratum` keeps).
    fn absorb(&mut self, o: State) {
        for (k, t) in o.spans {
            let s = self.spans.entry(k).or_default();
            s.calls += t.calls;
            s.total += t.total;
            s.own += t.own;
            s.alloc += t.alloc;
            s.own_alloc += t.own_alloc;
            s.freed += t.freed;
            s.rise += t.rise;
            s.own_rise += t.own_rise;
        }
        for (k, t) in o.sql {
            let s = self.sql.entry(k).or_default();
            s.runs += t.runs;
            s.ns += t.ns;
            s.vm += t.vm;
            s.scan += t.scan;
            s.sort += t.sort;
            s.autoindex += t.autoindex;
        }
        for (k, t) in o.native {
            add_steps(self.native.entry(k).or_default(), &t);
        }
        for (k, t) in o.native_cpu {
            *self.native_cpu.entry(k).or_default() += t;
        }
        for (k, t) in o.native_step_cpu {
            add_cpu(self.native_step_cpu.entry(k).or_default(), &t);
        }
        for (k, t) in o.notes {
            self.notes.entry(k).or_insert(t);
        }
        for (k, n) in o.decisions {
            *self.decisions.entry(k).or_default() += n;
        }
        // Where a reused step's decisions came from (decision 0269), by decision.
        for (k, from) in o.kept {
            self.kept.insert(k, from);
        }
        for s in o.strata {
            let other = format!("{} (chunk with {})", s.key, s.engine);
            let key = match self.strata.iter().find(|t| t.key == s.key) {
                Some(t) if t.engine != s.engine => other,
                _ => s.key.clone(),
            };
            match self.strata.iter_mut().find(|t| t.key == key) {
                Some(t) => t.loops += s.loops,
                None => self.strata.push(Stratum { key, ..s }),
            }
        }
        for (k, t) in o.fallbacks {
            self.fallbacks.insert(k, t);
        }
        for (k, t) in o.plans {
            let s = self.plans.entry(k).or_default();
            s.rule = t.rule;
            s.plan = t.plan;
            s.loops += t.loops;
            s.planning += t.planning;
            s.replans += t.replans;
            s.measured |= t.measured;
            add_steps(&mut s.actual, &t.actual);
            add_cpu(&mut s.cpu, &t.cpu);
            if let Some(n) = t.produced {
                *s.produced.get_or_insert(0) += n;
            }
        }
        for (k, t) in o.devices {
            let s = self.devices.entry(k).or_default();
            s.runs += t.runs;
            s.launches += t.launches;
            s.gpu += t.gpu;
            s.wall += t.wall;
            s.cells += t.cells;
            for (a, b) in s.phases.iter_mut().zip(t.phases) {
                *a += b;
            }
            s.made += t.made;
            s.reused += t.reused;
        }
        for (k, n) in o.work {
            *self.work.entry(k).or_default() += n;
        }
        for (k, t) in o.waits {
            let s = self.waits.entry(k).or_default();
            s.count += t.count;
            s.wall += t.wall;
            s.cpu += t.cpu;
        }
        for (k, t) in o.placements {
            let s = self.placements.entry(k).or_default();
            s.runs += t.runs;
            s.on_gpu += t.on_gpu;
            s.gpu_cheaper += t.gpu_cheaper;
            s.device |= t.device;
            s.forms = t.forms;
            s.missing = t.missing;
            s.planned_work += t.planned_work;
            s.planned_scanned += t.planned_scanned;
            s.steps += t.steps;
            s.rules += t.rules;
            s.work += t.work;
            s.cpu_estimate += t.cpu_estimate;
            s.gpu_estimate += t.gpu_estimate;
            s.cpu += t.cpu;
            s.gpu_cpu += t.gpu_cpu;
            s.gpu_work += t.gpu_work;
            for (a, b) in s.joins.iter_mut().zip(t.joins) {
                a.0 += b.0;
                a.1 += b.1;
                a.2 += b.2;
                a.3 += b.3;
            }
            s.gpu.add(&t.gpu);
            for (a, b) in s.gpu_terms.iter_mut().zip(t.gpu_terms) {
                *a += b;
            }
            if t.refused.is_some() {
                s.refused = t.refused;
            }
        }
        for (k, t) in o.hazards {
            let s = self.hazards.entry(k).or_default();
            s.0 += t.0;
            s.1 += t.1;
        }
        for (k, t) in o.blocked {
            let s = self.blocked.entry(k).or_default();
            for (r, n) in t.ran {
                *s.ran.entry(r).or_default() += n;
            }
            s.native.extend(t.native);
            s.device.extend(t.device);
        }
    }
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
    /// Of the planned work, the rows its scans read (`Shape::scanned`), and its rules' steps, over its runs.
    pub planned_scanned: f64,
    pub steps: u64,
    /// Its rules, over its runs.
    pub rules: u64,
    pub work: u64,
    pub cpu_estimate: f64,
    pub gpu_estimate: f64,
    pub cpu: f64,
    /// Of `cpu` and `work`, the runs that ran on the device: the rest ran on the CPU.
    pub gpu_cpu: f64,
    pub gpu_work: u64,
    /// Its join steps by the form placement chose (CPU index, GPU sort and probe, GPU tile): how many, their
    /// estimate on the CPU's index, and on the form chosen (of the steps estimated under `JOIN_ESTIMATE_CAP`), and
    /// how many were estimated over it.
    pub joins: [(u64, f64, f64, u64); 3],
    /// Its runs on the device (decision 0095, step 4), summed: rules, stages, submissions, dispatches, the GPU's
    /// time, this thread's CPU encoding, waiting and reading back, uploads, residency hits; and why the device
    /// refused it, when it was placed there and did not run there.
    pub gpu: GpuTotal,
    pub refused: Option<String>,
    /// The device price's terms over its runs on the device, as priced (`placement::DeviceTerms::array`: rules,
    /// steps, sorts, values read, rows made, joins' device seconds, other work, views read).
    pub gpu_terms: [f64; 8],
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
    /// Uploads by what was read (`UPLOAD_KINDS`): bytes and the host's CPU.
    pub upload_by: [(u64, f64); 4],
    /// Relations read where they lie, in their block (decision 0325): nothing uploaded.
    pub in_place: u64,
    pub sorts: u64,
    pub sort_hits: u64,
    pub rows: u64,
    pub distinct: u64,
    /// Head rows written in place into their head's block (decision 0325 step 4).
    pub placed: u64,
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
        for (a, b) in self.upload_by.iter_mut().zip(&o.upload_by) {
            a.0 += b.0;
            a.1 += b.1;
        }
        self.in_place += o.in_place;
        self.sorts += o.sorts;
        self.sort_hits += o.sort_hits;
        self.rows += o.rows;
        self.distinct += o.distinct;
        self.placed += o.placed;
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
/// The process's instructions retired so far (macOS; 0 elsewhere), as `settings::process_counters` reads them: what a
/// timeline line carries beside its clock.
pub fn retired_instructions() -> u64 {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: a zeroed `rusage_info_v4` is a valid out-value, and the flavour matches it.
        let mut info = unsafe { std::mem::zeroed::<libc::rusage_info_v4>() };
        if unsafe { libc::proc_pid_rusage(std::process::id() as i32, libc::RUSAGE_INFO_V4, (&raw mut info).cast()) } == 0 {
            return info.ri_instructions;
        }
    }
    0
}

/// A timeline's mark (dev, `RERAIL_TIMELINE`): a point outside every span, with the process's instructions retired.
pub fn timeline_mark(what: &str) {
    if std::env::var_os("RERAIL_TIMELINE").is_some() {
        eprintln!("RERAIL_TIMELINE {what}: @{:.3} i={:.3} mb={}/{}", timeline_clock(), retired_instructions() as f64 / 1e9, crate::code_rule_dsl::prof::footprint_mb(), crate::code_rule_dsl::prof::max_footprint_mb());
    }
}

/// The wall clock a timeline line carries (`RERAIL_TIMELINE`): seconds since the epoch, so the lines of nested
/// runs and threads read on one axis.
pub fn timeline_clock() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

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
    /// Its rows when every row is walked: past the step that binds the head the walk keeps one witness (decision
    /// 0229) and `rows` counts that; the device's stages make them all (decision 0277).
    pub all: f64,
    pub passed: Vec<(String, f64, String)>,
    /// The rule the step's rows a binding were priced by (`plan_search`'s `atom_rows`): postings, observed,
    /// absence proof, membership, and the containment share applied, so a misprediction names its rule.
    pub basis: String,
}

/// A rule's plan as the planner chose it: estimated work and rows, and its search (subsets kept, candidates priced).
#[derive(Clone, Debug, Default)]
pub struct RulePlan {
    pub work: f64,
    pub rows: f64,
    pub subsets: u64,
    pub candidates: u64,
    pub steps: Vec<StepPlan>,
    /// The join shape directives that held back a step of its search (`.shape`, O-join-shape-directives).
    pub shaped: Vec<String>,
}

#[derive(Default)]
pub struct PlanTotal {
    pub rule: Option<usize>,
    pub plan: RulePlan,
    pub loops: u64,
    pub planning: f64,
    replans: u64,
    pub actual: Vec<(u64, u64)>,
    /// Per step, and last its head's rows made, the CPU seconds the walks spent there (`planned_cpu`).
    pub cpu: Vec<f64>,
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


/// One thread's records: what its spans, decisions, work and plans add, under a lock only its own thread takes
/// while it records. One lock for every thread's records was a run's hottest lock: sixteen threads ending spans and
/// stating decisions took turns at it (about a tenth of a full corpus run's yields and blocked waits, 2026-11-15).
type Shard = std::sync::Arc<parking_lot::Mutex<State>>;

/// A run's profile (decision 0442: the run's, `local`): its records merged, and every thread's shard of them, an
/// ended thread's too until its records are merged.
#[derive(Default)]
pub struct Records {
    merged: std::sync::Arc<parking_lot::Mutex<State>>,
    shards: parking_lot::Mutex<Vec<Shard>>,
    /// Whether a corpus run of the run holds them (`hold_records`): it resets and writes them.
    held: AtomicBool,
}

thread_local! {
    /// Whether this thread holds the profile's state (`state`, `merged`).
    static HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// This thread's shard of its run's records (`Records`), made and listed in them when it first records for that
    /// run, with the run's records it is listed in.
    static SHARD: std::cell::RefCell<(std::sync::Weak<Records>, Shard)> = std::cell::RefCell::new((std::sync::Weak::new(), Shard::default()));
}

/// The profile's state, held: this thread's shard to record into (`state`), or every record merged to read
/// (`merged`). Its locks are not reentrant, so a profile call made while this thread holds one (from a closure run
/// under it: a decision's text that splits a program, which opens a span) would wait on itself forever. Every
/// closure a profile function takes is run before the lock, and a call that would wait on itself panics here
/// instead, naming the cause.
pub enum Held {
    Own(parking_lot::ArcMutexGuard<parking_lot::RawMutex, State>),
    Merged(parking_lot::ArcMutexGuard<parking_lot::RawMutex, State>),
}

impl std::ops::Deref for Held {
    type Target = State;
    fn deref(&self) -> &State {
        match self {
            Held::Own(g) => g,
            Held::Merged(g) => g,
        }
    }
}

impl std::ops::DerefMut for Held {
    fn deref_mut(&mut self) -> &mut State {
        match self {
            Held::Own(g) => &mut *g,
            Held::Merged(g) => &mut *g,
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        HELD.with(|h| h.set(false));
    }
}

fn hold() {
    assert!(!HELD.with(|h| h.replace(true)), "the profile's state was taken again on a thread holding it: a closure ran under its lock");
}

/// This thread's records, to add to. What is read from them is this thread's alone: a reader of the run's takes
/// `merged`.
pub fn state() -> Held {
    hold();
    let records = local::<Records>();
    Held::Own(SHARD.with(|s| {
        let mut s = s.borrow_mut();
        if !std::ptr::eq(s.0.as_ptr(), std::sync::Arc::as_ptr(&records)) {
            let shard = Shard::default();
            records.shards.lock().push(shard.clone());
            *s = (std::sync::Arc::downgrade(&records), shard);
        }
        s.1.lock_arc()
    }))
}

/// Every thread's records, merged: each shard's taken into the run's (an ended thread's shard then dropped).
pub fn merged() -> Held {
    hold();
    let records = local::<Records>();
    let mut all = records.merged.lock_arc();
    let mut shards = records.shards.lock();
    for s in shards.iter() {
        let taken = std::mem::take(&mut *s.lock());
        all.absorb(taken);
    }
    // A shard held only here is an ended thread's: emptied, it is not kept.
    shards.retain(|s| std::sync::Arc::strong_count(s) > 1);
    Held::Merged(all)
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
    /// Its Mach port (macOS) or its thread id (Linux), for its CPU read from another thread; 0 elsewhere.
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
    /// The run's thread only (`track_run_thread`, decision 0288): its time, charged to its innermost open span.
    track: Option<Track>,
}

/// The run thread's time since its last span opened or closed, and what it was charged to (decision 0288). Each open
/// and close charges the time since the last one to the span innermost then, so a span's own time is split exactly
/// at its children's bounds, and a phase's is split where the phase ends (`run_thread_phase`).
struct Track {
    wall: std::time::Instant,
    cpu: f64,
    /// By the innermost span's path (`OUTSIDE` outside every span): wall, and the thread's CPU. A pool job the thread
    /// runs while it waits opens spans of its own, so its CPU is charged to them.
    by: HashMap<String, [f64; 2]>,
}

/// Where the run's thread was outside every span (`Track`).
const OUTSIDE: &str = "(outside every span)";

/// Charges the time since `track`'s last charge to the innermost of `opens`, with the thread's CPU now (`cpu`).
fn charge(track: &mut Option<Track>, opens: &[Open], cpu: f64) {
    let Some(t) = track.as_mut() else { return };
    let now = std::time::Instant::now();
    let at = opens.last().map_or(OUTSIDE, |o| o.path.as_str());
    let d = [(now - t.wall).as_secs_f64(), cpu - t.cpu];
    match t.by.get_mut(at) {
        Some(e) => e.iter_mut().zip(d).for_each(|(a, b)| *a += b),
        None => {
            t.by.insert(at.to_owned(), d);
        }
    }
    (t.wall, t.cpu) = (now, cpu);
}

/// Each phase of the run's thread (`run_thread_phase`): its name, and by span path the wall and CPU.
/// The run's (decision 0442, `local`).
type RunPhases = Mutex<Vec<(&'static str, HashMap<String, [f64; 2]>)>>;

/// Starts charging this thread's time to its spans, phase by phase (decision 0288): the run's thread, once the run
/// holds the profile. The phases kept so far are forgotten.
pub fn track_run_thread() {
    if !on() {
        return;
    }
    local::<RunPhases>().lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
    STACK.with(|s| s.lock().track = Some(Track { wall: std::time::Instant::now(), cpu: cpu(), by: HashMap::new() }));
}

/// Ends phase `name` on the run's thread: its time since the phase before is kept under `name` (added to a phase of
/// that name already kept, as `local` is per chunk). Nothing on a thread not tracked.
pub fn run_thread_phase(name: &'static str) {
    STACK.with(|s| {
        let mut g = s.lock();
        let s = &mut *g;
        charge(&mut s.track, &s.opens, cpu());
        let Some(t) = s.track.as_mut() else { return };
        let by = std::mem::take(&mut t.by);
        let phases = local::<RunPhases>();
        let mut all = phases.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match all.iter_mut().find(|p| p.0 == name) {
            Some((_, kept)) => {
                for (path, d) in by {
                    let e = kept.entry(path).or_default();
                    e.iter_mut().zip(d).for_each(|(a, b)| *a += b);
                }
            }
            None => all.push((name, by)),
        }
    });
}

/// Stops charging this thread's time (the run's end).
pub fn untrack_run_thread() {
    STACK.with(|s| s.lock().track = None);
}

/// One phase of the run's thread, measured at its bounds (decision 0288), unprofiled as well: the phase's wall; the
/// thread's own CPU in it, and the CPU of the pool's jobs it ran while it waited; the whole process's CPU in it; its
/// threads' runnable time (macOS's `ri_runnable_time`: running, or ready with no core); and the process's involuntary
/// context switches.
#[derive(Clone, Copy, Default, Debug)]
pub struct PhaseClock {
    pub wall: f64,
    pub own: f64,
    pub helped: f64,
    pub process_cpu: f64,
    pub runnable: f64,
    pub involuntary: i64,
}

/// The clocks a `PhaseClock` is the difference of, read on the run's thread (`snapshot`).
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    at: std::time::Instant,
    /// The thread's CPU, less the pool's jobs it ran while it waited (`rerail_accel::exec::helped`), and those.
    own: f64,
    helped: f64,
    /// `process_clock`'s.
    process: (f64, f64, i64),
}

/// This thread's clocks now, and the process's.
pub fn snapshot() -> Snapshot {
    let helped = rerail_accel::exec::helped();
    Snapshot { at: std::time::Instant::now(), own: cpu() - helped, helped, process: process_clock() }
}

thread_local! {
    /// The command's start (`command_begins`) and its run's end (`run_ends`), for the time before and after its run.
    /// The command's, on the thread that runs it (its outermost run runs there).
    static COMMAND: std::cell::Cell<(Option<Snapshot>, Option<Snapshot>, f64)> = const { std::cell::Cell::new((None, None, 0.0)) };
}

/// The command line's command begins (decision 0288): its clocks now, and the process's age (its start's time).
/// Profiling, this thread's time is charged to its spans from here (`track_run_thread`): its run files what came
/// before it as its phase `before`.
pub fn command_begins() {
    COMMAND.with(|c| c.set((Some(snapshot()), None, process_age())));
    track_run_thread();
}

/// Whether this thread's time is charged to its spans (`track_run_thread`).
pub fn tracked() -> bool {
    STACK.with(|s| s.lock().track.is_some())
}

/// The command's start, taken by its first run that holds the profile or runs alone (`before`).
pub fn take_command_start() -> Option<Snapshot> {
    COMMAND.with(|c| {
        let mut v = c.get();
        let start = v.0.take();
        c.set(v);
        start
    })
}

/// The command's run ended with these clocks.
pub fn run_ends(at: Snapshot) {
    COMMAND.with(|c| {
        let mut v = c.get();
        v.1 = Some(at);
        c.set(v);
    });
}

/// The process's age when the command began, and the time since its run ended (`run_ends`), if it ran one.
pub fn command_ends() -> (f64, Option<PhaseClock>) {
    COMMAND.with(|c| {
        let mut v = c.get();
        let age = v.2;
        let end = v.1.take();
        c.set(v);
        (age, end.map(|end| PhaseClock::between(&end, &snapshot())))
    })
}

impl PhaseClock {
    /// The clocks from `a` to `b`, both read on one thread.
    pub fn between(a: &Snapshot, b: &Snapshot) -> PhaseClock {
        PhaseClock {
            wall: (b.at - a.at).as_secs_f64(),
            own: b.own - a.own,
            helped: b.helped - a.helped,
            process_cpu: b.process.0 - a.process.0,
            runnable: b.process.1 - a.process.1,
            involuntary: b.process.2 - a.process.2,
        }
    }
    /// The thread's wall off a core: blocked, or ready with no core.
    pub fn off_core(&self) -> f64 {
        (self.wall - self.own - self.helped).max(0.0)
    }
    /// The other threads' CPU meanwhile.
    pub fn others(&self) -> f64 {
        (self.process_cpu - self.own - self.helped).max(0.0)
    }
    /// The process's threads' time ready to run with no core: their runnable time less their CPU (the machine's load).
    pub fn no_core(&self) -> f64 {
        (self.runnable - self.process_cpu).max(0.0)
    }
    pub fn add(&mut self, o: &PhaseClock) {
        self.wall += o.wall;
        self.own += o.own;
        self.helped += o.helped;
        self.process_cpu += o.process_cpu;
        self.runnable += o.runnable;
        self.involuntary += o.involuntary;
    }
}

/// The process's clocks now: its CPU (user + system), its threads' runnable seconds (macOS's `ri_runnable_time`, which
/// counts a thread from when it is ready until it blocks, running or not; 0 elsewhere), and its involuntary context
/// switches.
pub fn process_clock() -> (f64, f64, i64) {
    #[cfg(unix)]
    let (cpu, involuntary) = {
        let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
        // SAFETY: fills a `rusage` for this process.
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1e6;
        (t(usage.ru_utime) + t(usage.ru_stime), usage.ru_nivcsw as i64)
    };
    #[cfg(not(unix))]
    let (cpu, involuntary) = (0.0, 0);
    #[cfg(target_os = "macos")]
    let runnable = {
        // SAFETY: a zeroed `rusage_info_v4` is a valid out-value, and the flavour matches it.
        let mut info = unsafe { std::mem::zeroed::<libc::rusage_info_v4>() };
        if unsafe { libc::proc_pid_rusage(std::process::id() as i32, libc::RUSAGE_INFO_V4, (&raw mut info).cast()) } == 0 { info.ri_runnable_time as f64 * mach_tick() } else { 0.0 }
    };
    #[cfg(not(target_os = "macos"))]
    let runnable = 0.0;
    (cpu, runnable, involuntary)
}

/// Seconds since this process started (macOS, Linux; 0 elsewhere): the command's wall so far.
pub fn process_age() -> f64 {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: a zeroed `proc_bsdinfo` is a valid out-value of the size passed.
        let mut info = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        if unsafe { libc::proc_pidinfo(std::process::id() as i32, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) } == size {
            let start = info.pbi_start_tvsec as f64 + info.pbi_start_tvusec as f64 / 1e6;
            return (timeline_clock() - start).max(0.0);
        }
        0.0
    }
    #[cfg(not(target_os = "macos"))]
    {
        0.0
    }
}

/// Seconds per Mach absolute-time tick, the unit of `rusage_info`'s times.
#[cfg(target_os = "macos")]
fn mach_tick() -> f64 {
    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }
    unsafe extern "C" {
        fn mach_timebase_info(info: *mut Timebase) -> i32;
    }
    static TICK: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *TICK.get_or_init(|| {
        let mut info = Timebase { numer: 0, denom: 0 };
        // SAFETY: fills the struct.
        unsafe { mach_timebase_info(&mut info) };
        f64::from(info.numer) / f64::from(info.denom.max(1)) / 1e9
    })
}

/// The run thread's phases (`PhaseClock`, measured by the run at each phase's end), with what the thread did in
/// each when profiling (decision 0288): the spans its wall went to, innermost, most first.
pub fn run_thread_report(clocks: &[(&'static str, PhaseClock)]) -> String {
    let mut r = String::new();
    let total = clocks.iter().fold(PhaseClock::default(), |mut a, (_, c)| {
        a.add(c);
        a
    });
    let _ = writeln!(r, "\n== The run's thread, phase by phase (decision 0288): its wall, its own CPU, its CPU in the pool's jobs it ran while it");
    let _ = writeln!(r, "waited, and its wall off a core (blocked, or ready with no core); the other threads' CPU, the process's CPU over the wall");
    let _ = writeln!(r, "(parallel), the process's threads' time ready with no core (the machine's load), and its involuntary switches");
    let _ = writeln!(r, "{:<10} {:>7} {:>7} {:>7} {:>7} {:>7} {:>8} {:>8} {:>9}", "phase", "wall s", "own s", "help s", "off s", "others", "parallel", "no core", "switches");
    for (name, c) in clocks.iter().chain(std::iter::once(&("all", total))) {
        let _ = writeln!(r, "{name:<10} {:>7.3} {:>7.3} {:>7.3} {:>7.3} {:>7.3} {:>8.2} {:>8.3} {:>9}", c.wall, c.own, c.helped, c.off_core(), c.others(), c.process_cpu / c.wall.max(1e-9), c.no_core(), c.involuntary);
    }
    let Some(phases) = local_if_any::<RunPhases>() else { return r };
    let phases = phases.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if phases.is_empty() {
        return r;
    }
    let _ = writeln!(r, "\nWhat the run's thread did in each phase: its wall by the span innermost when it passed, most first, with the thread's");
    let _ = writeln!(r, "CPU (a pool job it ran while it waited is under the job's own spans) and its wall off a core. The profile's own work is");
    let _ = writeln!(r, "in these; the table above, from a run without the profile, is the run's.");
    for (name, by) in phases.iter() {
        let mut rows: Vec<(&String, &[f64; 2])> = by.iter().collect();
        rows.sort_by(|a, b| b.1[0].total_cmp(&a.1[0]));
        let wall: f64 = rows.iter().map(|r| r.1[0]).sum();
        let _ = writeln!(r, "-- {name}: {wall:.3} s over {} spans", rows.len());
        let _ = writeln!(r, "{:>8} {:>8} {:>8}  span (innermost)", "wall s", "cpu s", "off s");
        let shown = rows.iter().take_while(|r| r.1[0] >= 0.005).take(15).count().max(rows.len().min(3));
        for (path, d) in &rows[..shown] {
            let _ = writeln!(r, "{:>8.3} {:>8.3} {:>8.3}  {}", d[0], d[1], (d[0] - d[1]).max(0.0), tail(path, 110));
        }
        if rows.len() > shown {
            let rest = rows[shown..].iter().fold([0.0; 2], |mut a, r| {
                a.iter_mut().zip(r.1).for_each(|(x, y)| *x += y);
                a
            });
            let _ = writeln!(r, "{:>8.3} {:>8.3} {:>8.3}  ({} other spans)", rest[0], rest[1], (rest[0] - rest[1]).max(0.0), rows.len() - shown);
        }
    }
    r
}

/// The last `n` characters of `path`, cut at a `/`: a span's innermost parts.
fn tail(path: &str, n: usize) -> String {
    if path.chars().count() <= n {
        return path.to_owned();
    }
    let skip = path.chars().count() - n;
    let cut: String = path.chars().skip(skip).collect();
    match cut.find('/') {
        Some(k) => format!("…{}", &cut[k..]),
        None => format!("…{cut}"),
    }
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

/// The live threads' Mach ports (macOS) or thread ids (Linux), and names; none elsewhere.
fn live_threads() -> Vec<(u32, String)> {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn mach_port_deallocate(task: libc::mach_port_t, name: libc::mach_port_t) -> libc::kern_return_t;
            // The task's own port (`mach_task_self()` is a macro over it; libc's function is deprecated).
            static mach_task_self_: libc::mach_port_t;
        }
        let mut list: libc::thread_act_array_t = std::ptr::null_mut();
        let mut count: libc::mach_msg_type_number_t = 0;
        // SAFETY: the task's own threads; the list and each port's right are given back below.
        let task = unsafe { mach_task_self_ };
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
    #[cfg(target_os = "linux")]
    {
        let Ok(tasks) = std::fs::read_dir("/proc/self/task") else { return Vec::new() };
        tasks
            .flatten()
            .filter_map(|task| {
                let id: u32 = task.file_name().to_str()?.parse().ok()?;
                // A thread that ended meanwhile has no `comm`.
                let name = std::fs::read_to_string(task.path().join("comm")).ok()?;
                let name = name.trim_end();
                Some((id, if name.is_empty() { "unnamed".to_owned() } else { name.to_owned() }))
            })
            .collect()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
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
    #[cfg(target_os = "linux")]
    // SAFETY: no arguments; the calling thread's id. The system call, not glibc's `gettid` (2.30): the Linux engine
    // links against 2.17.
    let port = unsafe { libc::syscall(libc::SYS_gettid) } as u32;
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
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
        track: None,
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

/// `f` for a parallel iterator's items, each a sizeable piece of work (a file, a tape, a piece of `par_pieces`):
/// on a thread whose innermost open span is not the one open where `carried` was called, under `name` in that one;
/// on the caller's thread, inside its span, as it is. A thread that took the item while it waited inside another
/// span (rayon's join runs any queued piece) so counts it apart: as that span's own, the piece's CPU and
/// allocation were charged to whatever the thread waited in (Q203 item 3: a chunk's joins seemed to allocate
/// 3.7 GB more at the same rows).
pub fn carried<A, R>(name: &'static str, f: impl Fn(A) -> R + Sync + Send) -> impl Fn(A) -> R + Sync + Send {
    let carry = carry();
    move |a| {
        let _in = carry.enter(name);
        f(a)
    }
}

/// What `carried` carries, for a parallel loop whose items `carried` cannot wrap (`map_init`'s, given the item's
/// state too): the run under way here with its context, and the span open, entered by each item (`Carry::enter`).
pub struct Carry {
    under: Option<String>,
    run: Option<std::sync::Arc<dyn RunOwn>>,
}

/// The run under way here, for the items a pool thread takes: their records are the run's (decision 0442).
pub fn carry() -> Carry {
    Carry { under: here(), run: job_run() }
}

impl Carry {
    /// An item on this thread in the carried run, under `name` in the carried span (as `carried`'s), until dropped.
    pub fn enter(&self, name: &'static str) -> impl Drop {
        // The span closes first, inside the run, as two locals would.
        struct In(Option<InRun>, Option<Span>);
        impl Drop for In {
            fn drop(&mut self) {
                self.1.take();
                self.0.take();
            }
        }
        let run = InRun::enter(&self.run);
        // A span when the item runs where the innermost open span is not the carried one: another thread, or one
        // that took it while waiting inside another span (Q203 item 3), so its CPU is counted apart.
        let elsewhere = self.under.as_deref().is_some_and(|u| STACK.with(|s| s.lock().opens.last().is_none_or(|o| o.path != u)));
        let span = elsewhere.then(|| span_at(self.under.as_deref(), || name.to_owned()));
        In(Some(run), span)
    }
}

/// A pool thread in a run's context for an item (`carried`): the run and its context (`job_run`); dropped, the
/// thread's own are back. On a thread that holds them already (an item run inline by its caller), nothing changes.
struct InRun {
    _entered: Option<Box<dyn std::any::Any>>,
}

impl InRun {
    fn enter(run: &Option<std::sync::Arc<dyn RunOwn>>) -> InRun {
        if holds(run) {
            return InRun { _entered: None };
        }
        InRun { _entered: Some(Box::new(enter(run.clone()))) }
    }
}

/// `f` over `items` on the pool in pieces (about `PIECES_PER_THREAD` a thread, at least `min` items each), each
/// piece through [`carried`] (a span when another thread, or one waiting in another span, takes it): `f` maps a
/// piece to its items' results, in order, so a piece's state (a pattern set) is made once a piece.
pub fn par_pieces<T: Sync, R: Send>(name: &'static str, items: &[T], min: usize, f: impl Fn(&[T]) -> Vec<R> + Sync + Send) -> Vec<R> {
    use rayon::prelude::*;
    const PIECES_PER_THREAD: usize = 4;
    // On the machine's pool (decision 0442), whose threads the pieces are sized by.
    let pieces: Vec<Vec<R>> = on_cores(|| {
        let size = items.len().div_ceil(rayon::current_num_threads() * PIECES_PER_THREAD).max(min).max(1);
        items.par_chunks(size).map(carried(name, f)).collect()
    });
    let mut out = Vec::with_capacity(items.len());
    pieces.into_iter().for_each(|p| out.extend(p));
    out
}

/// Opens span `name` under `under`, a path another thread's `here` gave: a pool job's span (`crate::threads`), at
/// that path whichever thread runs it. Without `under`, under this thread's open span, as `span` opens one.
pub fn span_at(under: Option<&str>, name: impl FnOnce() -> String) -> Span {
    match under {
        Some(p) => open(format!("{p}/{}", name())),
        None => span(name),
    }
}

/// Opens span `name` (made only when profiling) under this thread's open span.
pub fn span(name: impl FnOnce() -> String) -> Span {
    if !on() {
        // A timeline (dev, `RERAIL_TIMELINE`): the main thread's spans as they open, nothing measured.
        // `RERAIL_TIMELINE=all`: every thread's, named (a test runs on a thread of its own name).
        let on = rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_TIMELINE").is_some());
        let all = rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_TIMELINE").is_ok_and(|v| v == "all"));
        // `RERAIL_TIMELINE=sections`: the runs' sections alone (`corpus::run_observed`), no span: a line per span
        // costs as much as a small span's work, so a timeline of every span is not the run's instructions.
        let sections = rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_TIMELINE").is_ok_and(|v| v == "sections"));
        if on && !sections && (all || std::thread::current().name() == Some("main")) {
            // With the process's instructions retired so far (macOS): a gap between two spans is then its work, not
            // only its wall time.
            let retired = retired_instructions();
            eprintln!("RERAIL_TIMELINE span {} @{:.3} [{}] i={:.3}", name(), timeline_clock(), std::thread::current().name().unwrap_or("-"), retired as f64 / 1e9);
        }
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
        let mut g = s.lock();
        let s = &mut *g;
        let alloc = s.counters.read();
        let start = cpu();
        charge(&mut s.track, &s.opens, start);
        if s.opens.is_empty() {
            let (after, at) = s.between.take().unwrap_or_else(|| (THREAD_START.to_owned(), 0.0));
            if start > at {
                *s.gaps.entry((after, path.clone())).or_default() += start - at;
            }
        }
        // The heap sampler charges this thread's allocations to the span now (`rerail_prof::heap`), and names the
        // thread for what it allocates with none open.
        let heap = if rerail_prof::heap::on() {
            if s.opens.is_empty() {
                rerail_prof::heap::name_thread(std::thread::current().name().unwrap_or("an unnamed thread"));
            }
            rerail_prof::heap::intern(&path)
        } else {
            0
        };
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
            let mut g = s.lock();
            let s = &mut *g;
            charge(&mut s.track, &s.opens, now);
            let (allocated, freed) = s.counters.read();
            let Some(open) = s.opens.pop() else { return };
            // Its thread's outermost span closed: what it allocates until the next is charged to "between spans,
            // after" it (as the CPU gaps are), not to no span at all.
            let after = match s.opens.last() {
                Some(o) => o.heap,
                None if rerail_prof::heap::on() => rerail_prof::heap::intern(&format!("(between spans) after {}", open.path)),
                None => 0,
            };
            rerail_prof::heap::set_span(after);
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
        let _ = std::fs::write(dir.join("heap.tsv"), memory::heap_tsv());
        if !rerail_prof::heap::snapshot_frames().is_empty() || std::env::var_os("RERAIL_HEAP_FRAMES").is_some() { let _ = std::fs::write(dir.join("heap-frames.tsv"), memory::heap_frames_tsv()); }
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

/// Waves under way on this machine (`waves`): what a run did not read before its first wave it does not read from
/// SQLite, and under the strict mode SQL meanwhile ends the process. Read only by a thread with no run's context
/// (decision 0442: the machine's, as every run on it counts in it).
#[derive(Default)]
struct WavesUnderWay(std::sync::atomic::AtomicUsize);

thread_local! {
    /// This thread works behind the run (the store's writer): what it sends to SQLite no wave waits for.
    static BEHIND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f`, which reads the store inside the waves (decision 0104's one exception, decision 0298 §5: a reached
/// stratum's stored general form, whose key the run knows only once it reaches the stratum), as work behind the run
/// (`behind`), which the strict mode lets through. `f` reads alone: what it would write goes to the store's writer.
pub fn read_in_waves<T>(f: impl FnOnce() -> T) -> T {
    behind(f)
}

/// Waves are under way until dropped: from a run's first wave (its first chunk) to its last (the cross-file part's
/// last stratum). SQL before and after is the run's beginning and end, where it may be; inside, it is a hazard.
/// With the binding of its stage (`Stage`, in the run's context).
/// The count it added to, so its drop takes it off there on any thread.
pub struct Waves(Option<std::sync::Arc<WavesUnderWay>>, Option<Bound>);

pub fn waves() -> Waves {
    let count = machine::<WavesUnderWay>();
    count.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Waves(Some(count), Some(bind(Stage(INSIDE))))
}

/// Where this thread's run is (decision 0192), in the run's context: no run (`0`), a run before its
/// first wave (`BEFORE`), or inside the waves (`INSIDE`). Runs on other threads may be in their waves meanwhile (a
/// typing run beside phase 1): a run before its own may still read, and a thread of a run in its waves may not.
/// A thread with no run's context (a parallel loop's item) goes by the machine's count (`WavesUnderWay`).
const BEFORE: u8 = 1;
const INSIDE: u8 = 2;
#[derive(Clone, Copy, Default)]
struct Stage(u8);

pub use rerail_context::{AnyArc, Cores, is_main_thread, cores_scope, cores_spawn, cores_spawn_fifo, cores_threads, on_cores, Bound, Caller, Machine, bind, bind_here, bound, caller, holds, job_run, with_bound, RUN, RunOwn, RunValues, Shared, default_shared, engine_thread, enter, left_run, run_ended, local, local_if_any, machine, misses, run_own, setting, shared, shared_here, shared_object, starter_object, with_local, with_shared, within};

/// A run starts on this thread (`plan::corpus`): before its first wave, unless it is nested in a run inside its
/// waves, until the guard drops.
pub fn run_begins() -> Option<Bound> {
    (bound::<Stage>().0 != INSIDE).then(|| bind(Stage(BEFORE)))
}

/// Whether a store may be asked here (decision 0104): not while waves are under way for this thread.
pub fn may_read_store() -> bool {
    !waves_here()
}

/// Whether waves are under way for this thread: its run's, or, with no run's context here, any run's.
fn waves_here() -> bool {
    match bound::<Stage>().0 {
        BEFORE => false,
        INSIDE => true,
        _ => machine::<WavesUnderWay>().0.load(std::sync::atomic::Ordering::Relaxed) > 0,
    }
}

/// Whether a run's waves are under way (`waves`): what is not read before the first wave is not read from SQLite.
pub fn inside_waves() -> bool {
    waves_here()
}


/// `RERAIL_SQL_STRICT=1`, the benchmark's mode: SQL only where decision 0104 puts it. A run reads what it needs
/// before its first wave (each run of the process, outside every wave), every wave is given what it reads in memory, and what the run keeps is written behind it by
/// the store's writer. Any other statement, on any connection of the process, ends it at once (`strict_statement`), naming
/// the statement, its thread and span, and where it was sent from; so does a wave's wait for the writer.
pub fn strict() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_SQL_STRICT").is_ok_and(|v| !matches!(v.as_str(), "" | "0" | "off")))
}

/// `RERAIL_WARM`, a development check for a run that is truly warm: one over a store that already holds everything
/// the run computes, under rules and inputs that have not changed since. Such a run should only read. Each of
/// these is then a recomputation, and a fault:
///
/// - a write to SQLite: any transaction that commits a write, on any connection of the process (the store's, the
///   run's, an in-memory one), the store writer's included;
/// - a write queued for the store's writer (`ContentStore::write_back`): what a cache miss computed, kept;
/// - the whole-run cutoff (`corpus::reuse`) asked for and not applying: a rule file, a document or a read changed,
///   or no run left a record;
/// - a maintained run's non-empty delta: an input's or a stratum's rows gone or new (`__minus`, `__plus`), or a
///   maintained relation's overdeleted or inserted rows (`__del`, `__ins`, `__gone`).
///
/// `RERAIL_WARM=1` ends the process at the first fault, with its stack (`warm_fault`). `RERAIL_WARM=list` reports
/// every fault and goes on, a line each as it happens (the first 60 with their stacks), and ends the process with
/// status 3 if there was one, after its own exit. A switch of how a run is checked, not of what it outputs: the cutoff's key leaves it
/// out (`reuse::NOT_OUTPUTS`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Warm {
    Off,
    Fail,
    List,
}

pub fn warm_mode() -> Warm {
    // The switch, read once as `strict` reads its own: 0 off, 1 fail, 2 list.
    match rerail_prof::code_rule_dsl::logic::profile::setting(|| match std::env::var("RERAIL_WARM").as_deref() {
        Err(_) | Ok("" | "0" | "off") => 0,
        Ok("list") => 2,
        Ok(_) => 1,
    }) {
        0 => Warm::Off,
        2 => Warm::List,
        _ => Warm::Fail,
    }
}

/// Whether the warm check is on (`warm_mode`).
pub fn warm() -> bool {
    warm_mode() != Warm::Off
}

/// The writes a warm run may still make, by what the store's writer calls the job (`ContentStore::write_back`'s
/// `what`), and why each is allowed: locks against another process's collection, the timestamps by which
/// collection judges recency, and the cost statistics. Nothing else is exempt from the warm check.
///
/// Changing this list needs the user's approval (decision 0211). `scripts/check/check_warm_mode.py` holds a copy
/// and refuses a commit that changes one without the other.
pub const WARM_WRITES_ALLOWED: &[(&str, &str)] = &[
    ("the combined program's pins released", "a lock: the release of the combined program's leases"),
    ("the run's manifest", "a timestamp, only for a manifest identical to one stored: its last run and time move, by which collection keeps a root's last two. A new manifest is a fault (`warm_fault`, in the job)"),
    ("the journal's aged cursor", "a timestamp: a root's journal cursor, or a run record's (the cutoff's checks), stored again once it is a million events behind with nothing named since (`observe::CURSOR_AGE`), as a replay from far back is slow"),
    ("the cost statistics", "the cost statistics' own file (`costs`): the samples, SQL plan reports and kernel forms a run measured, and the model trained on them; what a run costs is never a result it recomputes (approved 2026-11-15)"),
];

/// Whether the store's writer may run a job called `what` in a warm run (`WARM_WRITES_ALLOWED`).
pub fn warm_write_allowed(what: &str) -> bool {
    WARM_WRITES_ALLOWED.iter().any(|(w, _)| *w == what)
}

thread_local! {
    /// On the store's writer: whether a transaction is open for listed jobs (`warm_scope`), and whether every job
    /// it ran so far is listed (`warm_job`).
    static WARM_SCOPE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// While it lives, the commits on this thread are a listed write's (`WARM_WRITES_ALLOWED`) if every job named to
/// `warm_job` inside it is listed; they are not a fault then. The store's writer holds one around each transaction.
pub struct WarmScope(Option<bool>);

pub fn warm_scope() -> WarmScope {
    WarmScope(WARM_SCOPE.with(|s| s.replace(Some(true))))
}

impl Drop for WarmScope {
    fn drop(&mut self) {
        WARM_SCOPE.with(|s| s.set(self.0));
    }
}

/// The store's writer runs the job called `what` in the open `warm_scope`.
pub fn warm_job(what: &str) {
    if warm() && !warm_write_allowed(what) {
        WARM_SCOPE.with(|s| s.set(s.get().map(|_| false)));
    }
}

/// How many faults a `RERAIL_WARM=list` run has had so far: its exit status is 3 if any.
static WARM_FAULTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A recomputation in a warm run (`warm_mode`): `kind` is what happened (a SQLite write, a queued write, ...),
/// `detail` which one. Nothing without `RERAIL_WARM`. Safe inside SQLite's callbacks: it aborts, never unwinds.
pub fn warm_fault(kind: &'static str, detail: impl FnOnce() -> String) {
    let mode = warm_mode();
    if mode == Warm::Off {
        return;
    }
    let mut detail = detail();
    if detail.len() > 600 {
        detail.truncate(detail.floor_char_boundary(600));
        detail.push('…');
    }
    let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
    if mode == Warm::Fail {
        eprintln!("rerail: RERAIL_WARM: {kind} in a warm run\n  {detail}\n  thread {thread}, span {}\n{}", current(), std::backtrace::Backtrace::force_capture());
        std::process::abort();
    }
    // Each fault is a line as it happens (`sort | uniq -c` groups them), the first 60 with their stacks.
    let n = WARM_FAULTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    #[cfg(not(windows))]
    if n == 1 {
        extern "C" fn report() {
            eprintln!("rerail: RERAIL_WARM: {} recomputations in a warm run", WARM_FAULTS.load(std::sync::atomic::Ordering::Relaxed));
            // SAFETY: ends the process with the warm check's status, after its other exit handlers ran.
            unsafe { libc::_exit(3) };
        }
        // SAFETY: registers a function without arguments, run once as the process exits.
        unsafe { libc::atexit(report) };
    }
    if n <= 60 {
        eprintln!("rerail: RERAIL_WARM: {kind} in a warm run\n  {detail}\n  thread {thread}, span {}\n{}", current(), std::backtrace::Backtrace::force_capture());
    } else {
        eprintln!("rerail: RERAIL_WARM: {kind}: {detail} (thread {thread}, span {})", current());
    }
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
    let inside = waves_here();
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

/// A write transaction committing on connection `db`: a fault of the warm check (`warm_fault`), naming the database
/// and the statement that commits it. SQLite calls a commit hook for a transaction that wrote (or took the write
/// lock, `BEGIN IMMEDIATE`), never for a read.
///
/// # Safety
/// `db` is an open connection, and this runs on its thread inside its commit hook.
pub unsafe fn warm_commit(db: *mut rusqlite::ffi::sqlite3) {
    use rusqlite::ffi;
    if !warm() || WARM_SCOPE.with(|s| s.get() == Some(true)) {
        return;
    }
    // SAFETY: the caller's connection.
    let file = unsafe {
        let file = ffi::sqlite3_db_filename(db, c"main".as_ptr());
        if file.is_null() { String::new() } else { std::ffi::CStr::from_ptr(file).to_string_lossy().into_owned() }
    };
    let kind = if file.is_empty() { "a SQLite write committed in memory" } else { "a SQLite write committed to a file" };
    warm_fault(kind, || {
        // SAFETY: the caller's connection; its statements are listed under its mutex, which the hook holds.
        unsafe {
            let mut statement = String::new();
            let mut s = ffi::sqlite3_next_stmt(db, std::ptr::null_mut());
            while !s.is_null() {
                if ffi::sqlite3_stmt_busy(s) != 0 {
                    let sql = ffi::sqlite3_sql(s);
                    if !sql.is_null() {
                        statement = std::ffi::CStr::from_ptr(sql).to_string_lossy().split_whitespace().collect::<Vec<_>>().join(" ");
                    }
                }
                s = ffi::sqlite3_next_stmt(db, s);
            }
            format!("{}: {statement}", if file.is_empty() { "(in memory)" } else { file.as_str() })
        }
    });
}

/// In the strict mode, every SQLite connection the process opens checks its statements (`strict_statement`): one that
/// sets a trace of its own (the store's, the run's) checks them there. In the warm mode (`warm`), each checks its
/// commits (`warm_commit`).
pub fn strict_on_every_connection() {
    use rusqlite::ffi;
    unsafe extern "C" fn trace(event: u32, _: *mut std::ffi::c_void, p: *mut std::ffi::c_void, _: *mut std::ffi::c_void) -> std::ffi::c_int {
        if event == ffi::SQLITE_TRACE_STMT {
            // SAFETY: for SQLITE_TRACE_STMT, P is the statement.
            let text = unsafe { std::ffi::CStr::from_ptr(ffi::sqlite3_sql(p.cast::<ffi::sqlite3_stmt>())) }.to_string_lossy();
            strict_statement("sqlite", &text);
        }
        0
    }
    unsafe extern "C" fn committed(db: *mut std::ffi::c_void) -> std::ffi::c_int {
        // SAFETY: the connection SQLite commits on, passed as the hook's argument, on its thread.
        unsafe { warm_commit(db.cast()) };
        0
    }
    unsafe extern "C" fn opened(db: *mut ffi::sqlite3, _: *mut *mut std::ffi::c_char, _: *const ffi::sqlite3_api_routines) -> std::ffi::c_int {
        // SAFETY: SQLite calls this with a connection it has just opened.
        unsafe {
            if strict() {
                ffi::sqlite3_trace_v2(db, ffi::SQLITE_TRACE_STMT, Some(trace), std::ptr::null_mut());
            }
            // A connection that sets a commit hook of its own (the store's, for its atoms) calls `warm_commit` there.
            if warm() {
                ffi::sqlite3_commit_hook(db, Some(committed), db.cast());
            }
        }
        ffi::SQLITE_OK
    }
    if strict() || warm() {
        // SAFETY: registers a function with the signature SQLite calls; it outlives every connection.
        unsafe { ffi::sqlite3_auto_extension(Some(opened)) };
    }
}

impl Drop for Waves {
    fn drop(&mut self) {
        if let Some(count) = self.0.take() {
            count.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            drop(self.1.take());
        }
    }
}

/// Marks this thread as working behind the run (the store's writer).
pub fn behind_the_run() {
    BEHIND.with(|b| b.set(true));
}

/// Runs `f` behind the run on this thread, which is then as it was: work after the command's end (its cost
/// samples written) on a thread that otherwise is the run's.
pub fn behind<T>(f: impl FnOnce() -> T) -> T {
    let was = BEHIND.with(|b| b.replace(true));
    let out = f();
    BEHIND.with(|b| b.set(was));
    out
}

/// Runs `f`, the command's own store bookkeeping around its run (an analysis scope's begin before the run, its
/// publication after it: `AnalysisRun`), as the store's writer does its work: outside the waves it writes as a writer
/// may (decision 0104: a run reads once before its first wave and writes behind it, and this is neither the run's read
/// nor its write). Inside the waves it is the run's own write, which the strict mode refuses.
pub fn around_the_run<T>(f: impl FnOnce() -> T) -> T {
    if strict() && waves_here() && !is_behind() {
        violation("inside the waves", "the command's own bookkeeping (`around_the_run`)");
    }
    behind(f)
}

/// Whether this thread works behind the run (`behind`, `behind_the_run`).
pub fn is_behind() -> bool {
    BEHIND.with(std::cell::Cell::get)
}

fn in_waves() -> bool {
    on() && waves_here() && !BEHIND.with(std::cell::Cell::get)
}

/// In the strict mode a wave's wait for the writer ends the process, as its SQL would (`strict_statement`); the run's last
/// wait, for every queued write, is the end of the run.
fn strict_wait(what: &str) {
    if strict() && !BEHIND.with(std::cell::Cell::get) && waves_here() {
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
    merged().hazards.iter().map(|((_, path, text), (runs, ns))|
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

/// A native plan's work from its rows in and out per step: the bindings each step was entered with, and the rows
/// its last step made, as the planner prices a plan (`est work`). Summing the bindings in alone charged a scan that
/// made 243,092 rows as 1 (`kind__sup0`, O125).
pub fn plan_work(steps: &[(u64, u64)]) -> u64 {
    steps.iter().map(|(a, _)| a).sum::<u64>() + steps.last().map_or(0, |s| s.1)
}

/// Adds a native plan's rows in and out per step, its probes' CPU seconds, and their CPU seconds per step and last
/// its head's rows made (`step_cpu`, empty where not timed), to the current span.
pub fn native(plan: String, steps: &[(u64, u64)], cpu_s: f64, step_cpu: &[f64]) {
    let path = current();
    let mut state = state();
    *state.native_cpu.entry((path.clone(), plan.clone())).or_default() += cpu_s;
    if !step_cpu.is_empty() {
        add_cpu(state.native_step_cpu.entry((path.clone(), plan.clone())).or_default(), step_cpu);
    }
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

/// A run-level decision or optimization under `topic` (counted when made again, as per chunk). Inside a step that
/// keeps its decisions with what it produces (`Deciding`), it is made with profiling off too: the kept rows are a
/// function of the step's inputs, not of whether the process that made them profiled.
pub fn decision(topic: &str, text: impl FnOnce() -> String) {
    let deciding = DECIDING.with(|d| !d.borrow().is_empty());
    // A debug build's tree check beside a pass decides nothing (`quiet`): the pass itself records its decisions.
    if QUIET.get() || (!on() && !deciding) {
        return;
    }
    // The text first: making it may profile too (a split's span), and the state's lock is not reentrant.
    let text = text();
    if deciding {
        DECIDING.with(|d| {
            for step in d.borrow_mut().iter_mut() {
                *step.entry((topic.to_owned(), text.clone())).or_default() += 1;
            }
        });
    }
    if on() {
        *state().decisions.entry((topic.to_owned(), text)).or_default() += 1;
    }
}

/// A decision (topic, text) and how often it was made.
pub type Decided = ((String, String), u64);

thread_local! {
    /// The open steps' decisions on this thread (`Deciding`), outermost first: a decision is each one's.
    static DECIDING: std::cell::RefCell<Vec<BTreeMap<(String, String), u64>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A step whose decisions are kept with what it produces (decision 0269): open from `open` to `close`, every
/// decision made on this thread is the step's (and every enclosing step's). A step's passes decide on its thread; a
/// pool job's decision is the profile's alone. Dropped without `close` (the step failed), it keeps nothing.
pub struct Deciding {
    depth: usize,
}

impl Deciding {
    pub fn open() -> Self {
        let depth = DECIDING.with(|d| {
            let mut d = d.borrow_mut();
            d.push(BTreeMap::new());
            d.len()
        });
        Deciding { depth }
    }

    /// The step's decisions, in order.
    pub fn close(self) -> Vec<Decided> {
        let made = DECIDING.with(|d| {
            let mut d = d.borrow_mut();
            assert_eq!(d.len(), self.depth, "steps keeping their decisions close innermost first");
            d.pop().expect("an open step")
        });
        std::mem::forget(self);
        made.into_iter().collect()
    }
}

impl Drop for Deciding {
    fn drop(&mut self) {
        DECIDING.with(|d| d.borrow_mut().truncate(self.depth - 1));
    }
}

/// Decisions made before, stated again as made now (decision 0385): a pass run over an edit, where the decisions it
/// made over the whole stand, says what it said then, to the profile and to every open step (`Deciding`).
pub fn restate(made: &[Decided]) {
    if made.is_empty() || QUIET.get() {
        return;
    }
    DECIDING.with(|d| {
        for step in d.borrow_mut().iter_mut() {
            for (k, n) in made {
                *step.entry(k.clone()).or_default() += n;
            }
        }
    });
    if on() {
        let mut state = state();
        for (k, n) in made {
            *state.decisions.entry(k.clone()).or_default() += n;
        }
    }
}

/// The decisions a version this run reused made when it was made (decision 0269), taken from `from` (`memory`,
/// `store`): the run states them as its own, as a run that made the version would.
pub fn reused(from: &'static str, made: &[Decided]) {
    if !on() || made.is_empty() {
        return;
    }
    let mut state = state();
    for (k, n) in made {
        *state.decisions.entry(k.clone()).or_default() += n;
        state.kept.insert(k.clone(), from);
    }
}

thread_local! {
    static QUIET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` with this thread's decisions unrecorded: a debug build's check that runs a pass's tree form beside it,
/// whose decisions the pass itself records.
pub fn quiet<T>(f: impl FnOnce() -> T) -> T {
    let was = QUIET.replace(true);
    let out = f();
    QUIET.set(was);
    out
}

/// The decisions made so far (none when profiling is off).
pub fn decisions() -> BTreeMap<(String, String), u64> {
    if !on() {
        return BTreeMap::new();
    }
    merged().decisions.clone()
}

/// The decisions made so far, taken: the next call returns those made after this one (a measurement's steps).
pub fn take_decisions() -> BTreeMap<(String, String), u64> {
    if !on() {
        return BTreeMap::new();
    }
    // Every thread's (`merged`): a step's decisions made on the pool are its too.
    let mut all = merged();
    all.kept.clear();
    std::mem::take(&mut all.decisions)
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
    let cpu: f64 = ran.iter().map(|(_, t)| t.gpu_cpu).sum();
    let elsewhere: f64 = ran.iter().map(|(_, t)| t.cpu - t.gpu_cpu).sum();
    let all_runs: u64 = ran.iter().map(|(_, t)| t.runs).sum();
    let ms = |s: f64| s * 1e3;
    let _ = writeln!(r, "Strata run on the GPU: {} strata, {} runs, {} rules in {} stages; those runs' CPU {:.2} s (the whole run: planning its replans, the store, the device's host part); the same strata's other {} runs, on the CPU: {:.2} s",
        ran.len(), sum.runs, sum.rules, sum.stages, cpu, all_runs - sum.runs, elsewhere);
    let _ = writeln!(r, "  GPU {:.1} ms (command buffers' timestamps) in {} submissions, {} dispatches ({} stages run again after an overflow, {} after the text arena filled); {} runs sent in their level's wave; {} stages' grids sized on the device",
        ms(sum.gpu), sum.submissions, sum.dispatches, sum.reruns, sum.arena_reruns, sum.waved, sum.sized);
    let _ = writeln!(r, "  host CPU {:.1} ms: encode {:.1}, uploads {:.1}, wait {:.1} ({:.1} ms wall), rows back into the store {:.1}",
        ms(sum.host()), ms(sum.encode), ms(sum.upload), ms(sum.wait), ms(sum.wait_wall), ms(sum.decode));
    let _ = writeln!(r, "  relations uploaded {} ({:.1} MB), read in place {}; sorts made {}, found in their blocks {}; bindings {}, head rows {} ({} distinct, made so on the device; {} written in place into their heads' blocks); texts made {}",
        sum.uploads, sum.upload_bytes as f64 / 1e6, sum.in_place, sum.sorts, sum.sort_hits, sum.bindings, sum.rows, sum.distinct, sum.placed, sum.made);
    let by: Vec<String> = UPLOAD_KINDS.iter().zip(&sum.upload_by).map(|(k, (b, c))| format!("{k} {:.1} MB in {:.1} ms", *b as f64 / 1e6, ms(*c))).collect();
    let _ = writeln!(r, "  uploads by what they read: {}", by.join(", "));
    let mut top = ran.clone();
    top.sort_by(|a, b| b.1.cpu.total_cmp(&a.1.cpu));
    // A stratum runs once per chunk and may run some on each side: its GPU runs' CPU and work beside its CPU runs'.
    let _ = writeln!(r, "{:>8} {:>8} {:>8} {:>8} {:>8} {:>9} {:>9} {:>6} {:>6} {:>10}  stratum (costliest 25 on the GPU; g = its GPU runs, c = its CPU runs)",
        "g cpu ms", "host ms", "gpu ms", "c cpu ms", "est ms", "runs g/all", "work g/c", "rules", "subm", "rows");
    for (key, t) in top.iter().take(25) {
        let g = &t.gpu;
        let _ = writeln!(r, "{:>8.2} {:>8.2} {:>8.3} {:>8.2} {:>8.3} {:>9} {:>9} {:>6} {:>6} {:>10}  {}", ms(t.gpu_cpu), ms(g.host()), ms(g.gpu), ms(t.cpu - t.gpu_cpu), ms(t.gpu_estimate),
            format!("{}/{}", g.runs, t.runs), format!("{}/{}", si(t.gpu_work), si(t.work - t.gpu_work)), g.rules, g.submissions, g.rows, short(key, 60));
    }
    if !refused.is_empty() {
        // A reason's strata, their CPU, and each stratum's CPU and key.
        type ByReason<'k> = (u64, f64, Vec<(f64, &'k str)>);
        let mut by: BTreeMap<&str, ByReason<'_>> = BTreeMap::new();
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

/// The strata that fell back to SQLite so far, by key: the kind of literal declined (a test's check that a form runs
/// natively).
pub fn declined() -> BTreeMap<String, String> {
    state().fallbacks.clone()
}

/// The strata a span path names as its last part: what follows each `stratum ` at the path's start or after a `/`
/// (`local/stratum a/b` names `a/b`; `stratum p/engine run` names `p/engine run`, which no stratum is).
fn stratum_tails(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices("stratum ").filter(|&(i, _)| i == 0 || path.as_bytes()[i - 1] == b'/').map(|(i, m)| &path[i + m.len()..])
}

/// The strata that fell back to SQLite, costliest first, and the same by the kind of literal declined: each one's
/// CPU (its `stratum …` spans, wherever they ran) and SQLite's VM steps. What the native engine lacks, by its cost.
pub fn fallbacks(r: &mut String, state: &State) {
    let roots: f64 = state.spans.iter().filter(|(p, _)| !p.contains('/')).map(|(_, t)| t.total).sum();
    // One pass over the spans and one over the statements, not one each per stratum.
    let mut spans: HashMap<&str, (f64, u64)> = HashMap::new();
    for (path, t) in &state.spans {
        for key in stratum_tails(path) {
            let e = spans.entry(key).or_default();
            e.0 += t.total;
            e.1 += t.calls;
        }
    }
    // A statement charges the stratum it ran in (a path's last part) and every stratum it ran under
    // (`stratum <key>/…`): a tail and each of its prefixes ending before a `/`, each key once.
    let mut vms: HashMap<&str, u64> = HashMap::new();
    let mut keys: Vec<&str> = Vec::new();
    for ((path, _), s) in &state.sql {
        keys.clear();
        for tail in stratum_tails(path) {
            keys.push(tail);
            keys.extend(tail.match_indices('/').map(|(j, _)| &tail[..j]));
        }
        keys.sort_unstable();
        keys.dedup();
        for key in &keys {
            *vms.entry(key).or_default() += s.vm;
        }
    }
    let mut rows: Vec<(f64, u64, u64, &String, &String)> = state
        .fallbacks
        .iter()
        .map(|(key, class)| {
            let (cpu, calls) = spans.get(key.as_str()).copied().unwrap_or((0.0, 0));
            (cpu, vms.get(key.as_str()).copied().unwrap_or(0), calls, key, class)
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
    // One pass over the spans, not one per stratum (thousands of strata over tens of thousands of spans).
    let mut by_key: HashMap<&str, f64> = HashMap::new();
    for (path, t) in &state.spans {
        for key in stratum_tails(path) {
            *by_key.entry(key).or_default() += t.total;
        }
    }
    let rows: Vec<(f64, &String, &Blocked)> = state.blocked.iter().map(|(k, b)| (by_key.get(k.as_str()).copied().unwrap_or(0.0), k, b)).collect();
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

/// A plan's CPU seconds per step, and last its head's rows made, over the walks `planned`'s `actual` counted.
pub fn planned_cpu(key: &str, kind: &str, plan: &RulePlan, cpu: &[f64]) {
    if !on() || cpu.is_empty() {
        return;
    }
    let text = plan.steps.iter().map(|s| s.literal.as_str()).collect::<Vec<_>>().join(" , ");
    add_cpu(&mut state().plans.entry((key.to_owned(), kind.to_owned(), text)).or_default().cpu, cpu);
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

/// The run's records held by one corpus run of it (`hold_records`), until the guard drops.
pub struct Holding(std::sync::Arc<Records>);

impl Drop for Holding {
    fn drop(&mut self) {
        self.0.held.store(false, Ordering::Release);
    }
}

/// Holds the run's profile for one corpus run of it, if no other corpus run of it holds it (decision 0442: the records
/// are the run's, so a run never waits on another's): a corpus run nested in another (phase 0's, inside a run's
/// preparing) is then the outer one's, and neither resets nor writes the profile. Outside a run, nothing.
pub fn hold_records() -> Option<Holding> {
    run_own()?;
    let records = local::<Records>();
    (!records.held.swap(true, Ordering::AcqRel)).then(|| Holding(records))
}

/// A run of its own for a test of the crates below `logic` (whose runs are `logic::run`'s), where none is under way.
#[cfg(any(test, feature = "test-hooks"))]
pub use rerail_context::test_run;

/// Forgets everything recorded (a run's start).
pub fn reset() {
    *merged() = State::default();
}

/// The statements recorded, by SQLite's time, most first: (span, text, totals).
pub fn statements() -> Vec<(String, String, SqlTotal)> {
    let state = merged();
    let mut all: Vec<_> = state.sql.iter().map(|((p, t), s)| (p.clone(), t.clone(), s.clone())).collect();
    all.sort_by_key(|(_, _, s)| std::cmp::Reverse(s.ns));
    all
}

/// The spans recorded: path and totals.
pub fn spans() -> BTreeMap<String, SpanTotal> {
    merged().spans.clone()
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
        let times = if *n > 1 { format!("  (x{n})") } else { String::new() };
        let kept = state.kept.get(&(t.clone(), text.clone())).map_or(String::new(), |from| format!("  (kept: {from})"));
        let _ = writeln!(r, "  - {text}{times}{kept}");
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
        // The stratum's plans, a range of the map (keyed by stratum first), not a scan of every plan per stratum.
        let plans: Vec<(&(String, String, String), &PlanTotal)> =
            state.plans.range((s.key.clone(), String::new(), String::new())..).take_while(|((k, _, _), _)| *k == s.key).collect();
        let planning: f64 = plans.iter().map(|(_, t)| t.planning).sum();
        let _ = writeln!(r, "Stratum {}  [engine: {}]  (loops={}, planning {planning:.3} s CPU)", short(&s.key, 120), s.engine, s.loops);
        if !s.why.is_empty() {
            let _ = writeln!(r, "  why: {}", s.why);
        }
        for ((_, kind, _), t) in plans {
            let produced = t.produced.map_or(String::new(), |n| format!(" produced={n}"));
            let shaped = if t.plan.shaped.is_empty() { String::new() } else { format!("; shaped by {}", t.plan.shaped.iter().map(|s| format!("`.shape {s}`")).collect::<Vec<_>>().join(", ")) };
            let _ = writeln!(
                r,
                "  -> {}  (est work={:.0} rows={:.1}) (loops={} replans={}{produced})  search: {} subsets, {} candidates{shaped}; planning {:.3} s",
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
                let basis = if st.basis.is_empty() { String::new() } else { format!("  [{}]", st.basis) };
                // Its CPU, and a row in's (Q36: a view's row can cost far more than a unit of work).
                let cpu = match (t.cpu.get(i), t.actual.get(i)) {
                    (Some(&c), Some(&(a, _))) if a > 0 => format!(" cpu {:.1} ms ({:.0} ns a row in)", c * 1e3, c * 1e9 / a as f64),
                    (Some(&c), _) => format!(" cpu {:.1} ms", c * 1e3),
                    _ => String::new(),
                };
                let _ = writeln!(r, "       {:>2}. {:<44} {:<30} {:<20} est {:>11}  {actual}{cpu}{basis}", i + 1, short(&st.literal, 44), short(&st.access, 30), short(&st.source, 20), rows(st.rows));
                for (lit, per, why) in &st.passed {
                    let why = if why.is_empty() { String::new() } else { format!("  [{why}]") };
                    let _ = writeln!(r, "           passed over {:<44} {per:.1} rows a binding{why}", short(lit, 44));
                }
            }
            // The head's rows made (each offered to its relation's new rows), after the last step's.
            if t.cpu.len() == t.actual.len() + 1 && let Some(c) = t.cpu.last() {
                let _ = writeln!(r, "           head's rows made: cpu {:.1} ms", c * 1e3);
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
    // The count the run is weighed by (decision 0435 §3, §5): beside the footprint, their gap names what it misses.
    let w = rerail_accel::budget::written();
    let _ = writeln!(
        r,
        "What the run wrote (decision 0435): {} MB now, {} MB at most read, of a limit of {} MB; heap {} MB, host blocks {} MB, the GPU pool's leases {} MB and idle {} MB, rerail-metal's ledger {} MB",
        memory::mb(w.total() as f64),
        memory::mb(rerail_accel::budget::written_most() as f64),
        memory::mb(rerail_accel::budget::limit() as f64),
        memory::mb(w.heap as f64),
        memory::mb(w.host_blocks as f64),
        memory::mb(w.leased as f64),
        memory::mb(w.idle as f64),
        memory::mb(w.ledger as f64)
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

/// CPU (user + system) of the thread with Mach port (macOS) or thread id (Linux) `port`, read from another thread;
/// `None` for a thread that has ended, and elsewhere.
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
    #[cfg(target_os = "linux")]
    {
        // Thread id 0 would name the calling thread's clock.
        if port == 0 {
            return None;
        }
        // The thread's CPU clock as glibc's `pthread_getcpuclockid` makes it (`CPUCLOCK_SCHED | CPUCLOCK_PERTHREAD`):
        // any thread of the process may read it; one that has ended gives EINVAL.
        let clock = (!(port as libc::clockid_t) << 3) | 6;
        let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: fills `t` with that clock's time.
        if unsafe { libc::clock_gettime(clock, &mut t) } != 0 {
            return None;
        }
        Some(t.tv_sec as f64 + t.tv_nsec as f64 / 1e9)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
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
            let plan = merged().notes.get(&format!("plan:{}", normal(text))).cloned();
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

/// Records why expensive input relations are requested, before projection planning: from the relations' readers and
/// the outputs (`analysis_report::projection_of`).
pub fn projection_plan_of(users: impl FnOnce() -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>>, outputs: &std::collections::BTreeSet<String>, reads: &std::collections::BTreeSet<String>) {
    if on() { analysis_report::projection_of(&users(), outputs, reads); }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Each stratum's CPU in the executors' report is its spans' (`stratum <key>`, alone or the last part of a path),
    /// a key of several relations (`a/b`) among them, and no other span's.
    #[test]
    fn executors_charge_each_stratum_its_own_spans() {
        let mut state = State::default();
        let t = |total: f64| SpanTotal { calls: 1, total, ..Default::default() };
        for (path, cpu) in [
            ("stratum p", 1.0),
            ("local/run/stratum p", 2.0),
            ("cross/stratum a/b", 4.0),
            ("stratum a/b", 8.0),
            ("local/stratum pq", 16.0),
            ("stratum p/engine run", 32.0),
            ("other", 64.0),
        ] {
            state.spans.insert(path.into(), t(cpu));
        }
        for key in ["p", "a/b", "pq", "none"] {
            state.blocked.entry(key.into()).or_default().ran.insert("native CPU", 1);
        }
        let mut r = String::new();
        executors(&mut r, &state);
        let cpu = |key: &str| r.lines().find(|l| l.trim_start().split_once("  ").is_some_and(|(_, rest)| rest.starts_with(&format!("{key}: ")))).map(|l| l.split_whitespace().next().unwrap().to_owned());
        assert_eq!(cpu("p").as_deref(), Some("3.000"), "{r}");
        assert_eq!(cpu("a/b").as_deref(), Some("12.000"), "{r}");
        assert_eq!(cpu("pq").as_deref(), Some("16.000"), "{r}");
        assert_eq!(cpu("none").as_deref(), Some("0.000"), "{r}");
    }

    /// A declined stratum's row in the SQLite fallbacks: its spans' CPU and runs (`stratum <key>`, alone or the last
    /// part of a path), and the VM steps of SQL run there or under it (`stratum <key>/…`); a key of several relations
    /// (`a/b`) beside one that is its prefix (`a`).
    #[test]
    fn fallbacks_charge_each_stratum_its_own_spans_and_sql() {
        let mut state = State::default();
        for (path, cpu) in [("stratum p", 1.0), ("local/stratum p", 2.0), ("stratum p/x", 4.0), ("stratum a/b", 8.0), ("other", 16.0)] {
            state.spans.insert(path.into(), SpanTotal { calls: 1, total: cpu, ..Default::default() });
        }
        for (path, vm) in [("stratum p", 10), ("x/stratum p/q", 100), ("stratum p/x", 1000), ("stratum a/b/c", 10000), ("stratum a/b", 5), ("other", 7)] {
            state.sql.insert((path.into(), "select 1".into()), SqlTotal { vm, ..Default::default() });
        }
        for key in ["p", "a/b", "a", "none"] {
            state.fallbacks.insert(key.into(), "negation".into());
        }
        let mut r = String::new();
        fallbacks(&mut r, &state);
        let row = |key: &str| {
            r.lines().find(|l| l.split_whitespace().nth(3) == Some(&format!("{key}:"))).map(|l| l.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
        };
        assert_eq!(row("p").as_deref(), Some("3.000 2 1110"), "{r}");
        assert_eq!(row("a/b").as_deref(), Some("8.000 1 10005"), "{r}");
        assert_eq!(row("a").as_deref(), Some("0.000 0 10005"), "{r}");
        assert_eq!(row("none").as_deref(), Some("0.000 0 0"), "{r}");
    }

    /// Decision 0269: a step keeps what it decides, with profiling off too, nested steps each theirs, a failed step
    /// nothing (its enclosing step keeps what it decided); a run that reuses the step's output states them, kept.
    #[test]
    fn a_step_keeps_its_decisions_and_a_reuse_states_them() {
        let _serial = test_run();
        set(false);
        let outer = Deciding::open();
        decision("t", || "a".into());
        let inner = Deciding::open();
        decision("t", || "b".into());
        let inner = inner.close();
        {
            let _failed = Deciding::open();
            decision("t", || "c".into());
        }
        decision("t", || "a".into());
        let outer = outer.close();
        let d = |text: &str, n: u64| (("t".to_owned(), text.to_owned()), n);
        assert_eq!(inner, vec![d("b", 1)]);
        assert_eq!(outer, vec![d("a", 2), d("b", 1), d("c", 1)]);
        set(true);
        reset();
        reused("store", &inner);
        let mut r = String::new();
        explain(&mut r, &state());
        set(false);
        assert!(r.contains("  - b  (kept: store)\n"), "{r}");
    }

    /// A thread records into its own shard: another thread holding its records does not hold it up (one lock for
    /// every thread's records made sixteen threads take turns at it), and an ended thread's records are read with
    /// the rest.
    #[test]
    fn a_thread_records_while_another_holds_its_records() {
        let _serial = test_run();
        set(true);
        reset();
        let held = state();
        let (tx, rx) = std::sync::mpsc::channel();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let tx = tx.clone();
                // Each thread in the test's run, profiled (decision 0442: the records and the switch are the run's
                // and its caller's).
                let run = job_run();
                std::thread::spawn(move || {
                    let _run = InRun::enter(&run);
                    for _ in 0..10 {
                        let _s = span(|| "xsharded".into());
                        decision("xshard", || "on another thread".into());
                    }
                    let _ = tx.send(());
                })
            })
            .collect();
        let recorded = (0..8).all(|_| rx.recv_timeout(std::time::Duration::from_secs(5)).is_ok());
        drop(held);
        for t in threads {
            t.join().unwrap();
        }
        let (d, s) = (decisions(), spans());
        set(false);
        assert!(recorded, "a thread waited for another's records to record its own");
        assert_eq!(d.get(&("xshard".to_owned(), "on another thread".to_owned())), Some(&80));
        assert_eq!(s.get("xsharded").map(|t| t.calls), Some(80));
    }

    #[test]
    fn spans_nest_and_charge_their_own_cpu() {
        let _serial = test_run();
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
        let _serial = test_run();
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

    /// A carried piece taken by a thread waiting inside another span is that piece's, under the span it was made
    /// in, not the other span's own (Q203 item 3); the caller's thread runs its own pieces as its own.
    #[test]
    fn a_piece_taken_inside_another_span_is_charged_apart() {
        let _serial = test_run();
        set(true);
        reset();
        let spin = |ms: u64| {
            let t = cpu();
            while cpu() - t < ms as f64 / 1e3 {
                std::hint::black_box(0u64);
            }
        };
        let piece = {
            let _made = span(|| "made".into());
            let piece = carried("a piece", move |ms: u64| spin(ms));
            // The caller's own thread, in the span it was made in: as it is, no span of its own.
            piece(1);
            piece
        };
        // The other thread in the test's run, profiled (decision 0442), its own span open when it takes the piece.
        let run = job_run();
        std::thread::scope(|t| {
            t.spawn(|| {
                let _run = InRun::enter(&run);
                let _waiting = span(|| "waiting".into());
                piece(30);
            });
        });
        let got: Vec<i64> = par_pieces("pieces", &(0..1000).collect::<Vec<i64>>(), 10, |p: &[i64]| p.iter().map(|x| x * 2).collect());
        let s = spans();
        set(false);
        assert_eq!(got, (0..1000).map(|x| x * 2).collect::<Vec<_>>(), "in order");
        assert!(s.contains_key("made/a piece"), "{:?}", s.keys().collect::<Vec<_>>());
        assert!(!s.keys().any(|k| k.starts_with("made/a piece") && s[k].calls > 1), "the caller's own piece opened none");
        assert!(s["made/a piece"].own >= 0.025, "{}", s["made/a piece"].own);
        assert!(s["waiting"].own < 0.015, "the waiting span's own holds none of the piece: {}", s["waiting"].own);
    }

    #[test]
    fn spans_charge_memory_and_a_dump_lists_what_is_open() {
        let _serial = test_run();
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
        let run = job_run();
        let holder = std::thread::Builder::new()
            .name("holder".into())
            .spawn(move || {
                let _run = InRun::enter(&run);
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

/// What a device's uploads read (`Report::upload_by`): a flat relation's rows (the store's), a view's rows (a
/// source's, a tape view's read row by row), and the names of symbols (text tests, `str_cmp`).
pub const UPLOAD_KINDS: [&str; 4] = ["flat relations", "views", "names", "views made on the device"];
pub const UPLOAD_FLAT: usize = 0;
pub const UPLOAD_VIEW: usize = 1;
pub const UPLOAD_NAMES: usize = 2;
/// A view its batch's chunk block made on the device (decision 0289 §2): its bytes, and the host's CPU encoding it.
pub const UPLOAD_MADE: usize = 3;

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
    pub stages: u32,
    pub submissions: u32,
    pub dispatches: u32,
    /// Runs again from an overflowing stage.
    pub reruns: u32,
    /// Times it waited for the device's memory before it encoded (`gpu::shared::admit`, Q203), and the seconds.
    pub admit_waits: u32,
    pub admit_seconds: f64,
    pub gpu_seconds: f64,
    /// This thread's CPU: encoding (uploads and sorts' encoding included), waiting, and reading the rows back into
    /// the store.
    pub encode_cpu: f64,
    pub wait_cpu: f64,
    pub wait_wall: f64,
    pub decode_cpu: f64,
    /// Head rows left once made distinct on the device (of `rows`).
    pub distinct: u64,
    /// Of those, rows the device wrote in place into their head's block, which the store takes as they lie
    /// (decision 0325 step 4): none read back.
    pub placed: u64,
    pub upload_cpu: f64,
    /// Relations uploaded (and their bytes), found resident; sorts made, found resident.
    pub uploads: u32,
    pub upload_bytes: u64,
    /// The uploads by what they read (`UPLOAD_KINDS`, in order): bytes and this thread's CPU.
    pub upload_by: [(u64, f64); 4],
    /// Relations read where they lie, in their block (decision 0325).
    pub in_place: u32,
    pub sorts: u32,
    pub sort_hits: u32,
    /// Rows the heads made (before the store's set), and bindings the stages made.
    pub rows: u64,
    pub bindings: u64,
    /// Texts made on the device (`cat` into a head) and stored as symbols.
    pub made: u64,
    /// The rounds a recursive component ran on the device (`fixpoint`; 0 for seeds alone).
    pub rounds: u32,
    /// Why the device refused the stratum (at open, or at run time: a table past its budget), which then ran on the
    /// CPU; `None` when it ran here.
    pub refused: Option<String>,
}

/// A count in three significant figures, with k, M or G.
fn si(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.3}k", n as f64 / 1e3),
        1_000_000..1_000_000_000 => format!("{:.3}M", n as f64 / 1e6),
        _ => format!("{:.3}G", n as f64 / 1e9),
    }
}
