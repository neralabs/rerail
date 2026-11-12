//! The executor: every job of the process's relational work, run on every lane at once, and scored.
//!
//! **Jobs.** A [`Batch`] is a level's launches: launches that do not read each other's outputs (decision 0095 §5).
//! The executor runs them as jobs on the lanes' workers:
//!
//! - **The cores** are the process's one pool ([`configure`]: [`threads`] threads, `RERAIL_THREADS`, else the
//!   machine's cores), shared by every executor and by every other job of the process. Each launch placed on the
//!   cores is a job of its own, so a batch's launches on the cores run on all the pool's threads at once, beside
//!   every other batch's and every other job. A kernel that splits its work (the reference's parallel loops) splits
//!   it over the same threads.
//! - **SME, the GPU and the Neural Engine** each have one worker thread for the executor's life. A batch's share of
//!   such a lane is one job: **one submission** ([`Backend::submit`]), so the lane's fixed cost is paid once a batch
//!   (one command buffer on the GPU, one streaming session per SME unit, batched predictions on the Neural Engine).
//! - **Host work** the caller hands over ([`Executor::spawn`]): a closure, a job on the cores' pool.
//!
//! Every lane runs at the same time as the others, and [`Executor::begin`] returns as soon as every job is queued,
//! so the caller works meanwhile; [`Running::wait`] collects the outputs (decision 0100 §3).
//!
//! **Placement** (decision 0100 §9). Each launch, largest first, goes to the lane [`Prices::choose_host`] picks:
//! the least host CPU among the lanes that finish by the deadline, the later of the parity band and the batch's
//! slack. What weighs on a lane ([`Executor::load`]) is what the scoreboard says is outstanding on it (every batch
//! begun and not finished), plus what this batch placed on it; its measured latency (the GPU's submit, schedule and
//! wake); and on the cores, the machine's load. A batch's slack is how long callers of its kind (`begin_as`) worked
//! before they waited, measured: a lane done by then costs the caller no wall time.
//! A launch may be pinned to a lane, and `RERAIL_LANE_PIN` pins every launch a lane can run to it, to measure.
//!
//! **The scoreboard** ([`crate::score`]) counts every job: queued, running, finished, its wait and its time, and the
//! price's estimate against it, by lane and by form. [`Executor::score`] reads it; `RERAIL_SCOREBOARD=<file>` traces
//! every job.
//!
//! There is no fallback. A launch no present lane can run is an error that names it, and so is a lane's failure.

use crate::kernel::{Kernel, Output};
use crate::lanes::{self, Lane};
use crate::price::{Cost, Load, Prices, WATTS};
use crate::score::{Item, Score, Scoreboard, Ticket};
use std::collections::BTreeMap;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

/// The process's threads: `RERAIL_THREADS`, a whole number of at least 1; else the cores the machine offers.
///
/// It sizes the one pool ([`configure`]): every job of the process's work runs on it, so the process never uses more
/// threads for its work than this.
pub fn threads() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("RERAIL_THREADS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|&n| n >= 1)
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
    })
}

/// Makes the one pool: rayon's global pool, [`threads`] threads named `rerail-cpu <i>`. Every job runs on it: the
/// cores' launches, host jobs ([`Executor::spawn`]), and every parallel loop and job of the caller's (rayon's
/// functions reach the global pool).
///
/// With `this_thread`, the calling thread is the pool's thread 0, so the work it does itself, and its waits that run
/// other jobs ([`help_until`]), are the pool's too: a program's main thread, called before any other thread uses the
/// pool. Without it (a library, a test), the pool's threads are its own. A pool already made stays as it is.
pub fn configure(this_thread: bool) {
    static MADE: OnceLock<()> = OnceLock::new();
    MADE.get_or_init(|| {
        let b = rayon::ThreadPoolBuilder::new().num_threads(threads()).thread_name(|i| format!("rerail-cpu {i}"));
        let _ = if this_thread { b.use_current_thread() } else { b }.build_global();
    });
}

thread_local! {
    /// This thread's CPU seconds in other jobs it ran while it waited (`help_once`).
    static HELPED: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
    /// The name of the last job ([`as_job`]) to end on this thread: the job a wait just ran.
    static LAST_JOB: std::cell::Cell<&'static str> = const { std::cell::Cell::new("") };
    /// The wait this thread is in, and the job it last ran for others: (wall, name).
    static LAST_RUN: std::cell::Cell<(f64, &'static str)> = const { std::cell::Cell::new((0.0, "")) };
}

/// Runs `f` as the job `name`: a wait that runs it for another knows what made it late ([`waits`]).
pub fn as_job<R>(name: &'static str, f: impl FnOnce() -> R) -> R {
    let r = f();
    LAST_JOB.with(|j| j.set(name));
    r
}

/// Runs one of the pool's other jobs on this thread, if one is queued (a pool thread's wait): whether it ran one. Its
/// CPU is counted as helped ([`helped`]), not as the waiter's own, and its wall is the job the wait last ran (how late
/// the wait may be, if what it waits for was ready during it: [`Waiting`]).
pub fn help_once() -> bool {
    let c0 = crate::score::thread_cpu();
    let t0 = Instant::now();
    LAST_JOB.with(|j| j.set(""));
    let ran = rayon::yield_now() == Some(rayon::Yield::Executed);
    if ran {
        let spent = crate::score::thread_cpu() - c0;
        HELPED.with(|h| h.set(h.get() + spent));
        let name = LAST_JOB.with(std::cell::Cell::get);
        LAST_RUN.with(|l| l.set((t0.elapsed().as_secs_f64(), if name.is_empty() { "(unnamed)" } else { name })));
    } else {
        LAST_RUN.with(|l| l.set((0.0, "")));
    }
    ran
}

/// One site's waits ([`Waiting`]): how many ran others' jobs, for how long, and how late those jobs made them.
#[derive(Clone, Debug, Default)]
pub struct WaitStats {
    pub waits: u64,
    /// Waits that ran at least one job of another's.
    pub helping: u64,
    pub jobs: u64,
    /// Wall and CPU seconds of the jobs run.
    pub jobs_wall: f64,
    pub jobs_cpu: f64,
    /// Wall seconds of the waits, start to end.
    pub wall: f64,
    /// Waits that ended right after a job they ran: what they waited for was ready during it, so the job made them
    /// late by up to its wall (`late`, an upper bound, its longest `late_max`), or by `late_exact` where the waited
    /// work said when it ended.
    pub late: u64,
    pub late_wall: f64,
    pub late_max: f64,
    pub late_exact: f64,
    /// The longest job run in a wait, and its name.
    pub longest: f64,
    pub longest_job: &'static str,
}

/// The jobs that made waits late: by job, how many waits and their late seconds (the bound).
#[derive(Clone, Debug, Default)]
pub struct LateBy {
    pub waits: u64,
    pub late_wall: f64,
    pub late_max: f64,
}

static WAITS: std::sync::Mutex<(BTreeMap<&'static str, WaitStats>, BTreeMap<&'static str, LateBy>)> = std::sync::Mutex::new((BTreeMap::new(), BTreeMap::new()));

/// Every site's waits so far, and the jobs that made them late: for the profile.
pub fn waits() -> (Vec<(&'static str, WaitStats)>, Vec<(&'static str, LateBy)>) {
    let w = WAITS.lock().unwrap_or_else(|p| p.into_inner());
    (w.0.iter().map(|(k, v)| (*k, v.clone())).collect(), w.1.iter().map(|(k, v)| (*k, v.clone())).collect())
}

/// A wait at a named site that may run others' jobs ([`help_once`] through [`Waiting::help`]), recorded when it ends
/// ([`waits`]).
pub struct Waiting {
    site: &'static str,
    began: Instant,
    s: WaitStats,
    /// The last job run, if nothing was polled since: (wall, name, when it ended).
    last: Option<(f64, &'static str, Instant)>,
}

impl Waiting {
    pub fn new(site: &'static str) -> Self {
        Waiting { site, began: Instant::now(), s: WaitStats { waits: 1, ..Default::default() }, last: None }
    }

    /// Runs one of the pool's other jobs, if one is queued: whether it ran one.
    pub fn help(&mut self) -> bool {
        let c0 = HELPED.with(std::cell::Cell::get);
        let ran = help_once();
        if ran {
            let (wall, name) = LAST_RUN.with(std::cell::Cell::get);
            self.s.jobs += 1;
            self.s.jobs_wall += wall;
            self.s.jobs_cpu += HELPED.with(std::cell::Cell::get) - c0;
            if wall > self.s.longest {
                self.s.longest = wall;
                self.s.longest_job = name;
            }
            self.last = Some((wall, name, Instant::now()));
        } else {
            self.last = None;
        }
        ran
    }

    /// Nothing was ready at a poll: the last job run did not make the wait late.
    pub fn polled(&mut self) {
        self.last = None;
    }

    /// Ends the wait, its waited work ready: at `ready` if the work said when it ended, else some time during the
    /// last job run, if one was.
    pub fn end(mut self, ready: Option<Instant>) {
        self.s.wall = self.began.elapsed().as_secs_f64();
        self.s.helping = u64::from(self.s.jobs > 0);
        let mut late_by = None;
        if let Some((wall, name, ended)) = self.last {
            self.s.late = 1;
            self.s.late_wall = wall;
            self.s.late_max = wall;
            self.s.late_exact = ready.map_or(0.0, |r| ended.saturating_duration_since(r).as_secs_f64().min(wall));
            late_by = Some((name, wall));
        }
        let mut w = WAITS.lock().unwrap_or_else(|p| p.into_inner());
        let t = w.0.entry(self.site).or_default();
        t.waits += self.s.waits;
        t.helping += self.s.helping;
        t.jobs += self.s.jobs;
        t.jobs_wall += self.s.jobs_wall;
        t.jobs_cpu += self.s.jobs_cpu;
        t.wall += self.s.wall;
        t.late += self.s.late;
        t.late_wall += self.s.late_wall;
        t.late_max = t.late_max.max(self.s.late_max);
        t.late_exact += self.s.late_exact;
        if self.s.longest > t.longest {
            t.longest = self.s.longest;
            t.longest_job = self.s.longest_job;
        }
        if let Some((name, wall)) = late_by {
            let l = w.1.entry(name).or_default();
            l.waits += 1;
            l.late_wall += wall;
            l.late_max = l.late_max.max(wall);
        }
    }
}

/// This thread's CPU seconds so far in jobs it ran for others while it waited: a caller that measures its own
/// work by its thread's CPU takes them out.
pub fn helped() -> f64 {
    HELPED.with(std::cell::Cell::get)
}

/// Whether this thread is one of the pool's.
pub fn on_pool() -> bool {
    rayon::current_thread_index().is_some()
}

/// Waits until `poll` gives a value, running the pool's other jobs meanwhile when this is one of its threads, so a
/// job that waits for others never holds its thread idle while they queue. Between polls with nothing to run, it
/// sleeps a little more each time (20 µs, up to 1 ms): a wait is bounded, and wakes for what it waits for at most
/// that late.
pub fn help_until<T>(poll: impl FnMut() -> Option<T>) -> T {
    help_until_as("wait", poll)
}

/// [`help_until`], its waits recorded under `site` ([`waits`]).
pub fn help_until_as<T>(site: &'static str, mut poll: impl FnMut() -> Option<T>) -> T {
    let pool = on_pool();
    let mut nap = 20e-6f64;
    let mut wait = Waiting::new(site);
    loop {
        if let Some(t) = poll() {
            wait.end(None);
            return t;
        }
        wait.polled();
        if pool && wait.help() {
            nap = 20e-6;
            continue;
        }
        std::thread::sleep(std::time::Duration::from_secs_f64(nap));
        nap = (nap * 2.0).min(1e-3);
    }
}

/// The cores' slowdown under the machine's load: the 1-minute load average over the cores, at least 1, read at most
/// every 100 ms. On a shared machine the cores' threads get less than a core each, so their wall time stretches; a
/// device's does not.
pub fn cores_slowdown() -> f64 {
    static LAST: std::sync::Mutex<Option<(Instant, f64)>> = std::sync::Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((at, v)) = *last {
        if at.elapsed().as_secs_f64() < 0.1 {
            return v;
        }
    }
    let ncpu = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
    #[cfg(unix)]
    let load = {
        let mut l = [0f64; 3];
        // SAFETY: three doubles out.
        if unsafe { libc::getloadavg(l.as_mut_ptr(), 3) } >= 1 { l[0] } else { 0.0 }
    };
    #[cfg(not(unix))]
    let load = 0.0;
    let v = (load / ncpu).max(1.0);
    *last = Some((Instant::now(), v));
    v
}

/// `RERAIL_LANE_PIN=<cpu|sme|gpu|ane>`: a measurement, not a setting. Every launch the named lane can run, and that
/// its caller did not pin, is placed there, so one run's launches can be measured on each lane in turn.
pub fn measured_lane() -> Option<Lane> {
    static PIN: OnceLock<Option<Lane>> = OnceLock::new();
    *PIN.get_or_init(|| {
        let v = std::env::var("RERAIL_LANE_PIN").ok()?;
        Lane::ALL.into_iter().find(|l| l.name() == v.trim())
    })
}

/// A submission begun ([`Backend::start`]) and not yet read.
pub trait Pending {
    /// Waits for it to complete: its outputs, in its launches' order.
    fn finish(self: Box<Self>) -> Result<Vec<Output>, String>;
}

/// A submission that ran as it began.
struct Ready(Result<Vec<Output>, String>);

impl Pending for Ready {
    fn finish(self: Box<Self>) -> Result<Vec<Output>, String> {
        self.0
    }
}

/// A lane's implementation.
pub trait Backend: Send + Sync {
    fn lane(&self) -> Lane;
    /// Whether this lane can run `k`, its shape's limits included (the Neural Engine's exact range, a constant it
    /// needs fixed, a threadgroup's memory).
    fn can(&self, k: &Kernel) -> bool;
    /// Runs `ks` as one submission. The outputs are in `ks`' order, each equal to [`crate::cpu::run`]'s.
    fn submit(&self, ks: &[&Kernel]) -> Result<Vec<Output>, String>;
    /// Begins `ks` as one submission and returns before it completes: the lane's worker begins its next one while
    /// the device runs this, and reads this one's outputs with [`Pending::finish`] after. A lane that cannot
    /// overlap its submissions runs it here (the default).
    fn start<'a>(&'a self, ks: &[&Kernel]) -> Result<Box<dyn Pending + 'a>, String> {
        Ok(Box::new(Ready(self.submit(ks))))
    }
    /// [`Backend::submit`], with where the submission's time went ([`Phases`]); a lane that does not split its work
    /// reports the default.
    fn submit_timed(&self, ks: &[&Kernel]) -> Result<(Vec<Output>, Phases), String> {
        self.submit(ks).map(|o| (o, Phases::default()))
    }
    /// Drops what the lane keeps for the join id `id` (decision 0100 §4).
    fn forget(&self, _id: u64) {}
    /// Drops everything the lane keeps (a chunk's end).
    fn clear(&self) {}
    /// What the lane keeps, and how it served.
    fn space(&self) -> SpaceCounts {
        SpaceCounts::default()
    }
}

/// What a lane's space holds and how it served, for the profile (decision 0100 §4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpaceCounts {
    pub entries: usize,
    pub bytes: usize,
    /// Joins whose right side was kept, and joins that built it.
    pub hits: u64,
    pub misses: u64,
    /// Entries dropped for the limit.
    pub evicted: u64,
}

/// Where one lane's submission went, in seconds, each phase summed over the submission. A lane that overlaps its
/// host work with the device (the Neural Engine, tile by tile) has phases that run at the same time, so they sum
/// to more than its wall time.
///
/// - `encode`: wall time writing inputs in the device's form (the Neural Engine: `a`'s bits expanded to fp16 in
///   place into IOSurfaces);
/// - `device`: wall time on the device, where the lane can see it (the Neural Engine: its prediction calls);
/// - `decode`: wall time reading outputs back (the Neural Engine: fp16 partial counts summed, each launch's share
///   and thresholds);
/// - `worker_cpu`: CPU (user + sys) spent for the submission off the lane's own thread (the Neural Engine: its
///   predictor thread and Core ML's threads).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Phases {
    pub encode: f64,
    pub device: Option<f64>,
    pub decode: f64,
    pub worker_cpu: f64,
}

/// One kernel to run, optionally pinned to a lane.
#[derive(Clone, Debug)]
pub struct Launch {
    pub kernel: Kernel,
    pub pin: Option<Lane>,
}

impl Launch {
    pub fn new(kernel: Kernel) -> Self {
        Launch { kernel, pin: None }
    }

    pub fn on(kernel: Kernel, lane: Lane) -> Self {
        Launch { kernel, pin: Some(lane) }
    }
}

/// Independent launches: one level's.
#[derive(Clone, Debug, Default)]
pub struct Batch(pub Vec<Launch>);

/// A launch's result, and where it ran.
#[derive(Clone, Debug)]
pub struct Done {
    pub out: Output,
    pub lane: Lane,
    /// The price's estimate of its time on that lane, and of the fastest lane's.
    pub estimate: f64,
    pub fastest: f64,
    /// The price's estimate of its host CPU seconds there.
    pub host: f64,
}

/// What a batch did on each lane: for the profile's per-lane report (decision 0098 §2).
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Per lane: launches, and the wall seconds from its first job's start to its last job's end.
    pub launches: [usize; 4],
    pub wall: [f64; 4],
    /// Estimated joules per lane: the price's power times the seconds its jobs ran.
    pub joules: [f64; 4],
    /// Estimated seconds the off-CPU lanes took from the CPU lane: what the CPU would have spent on their launches.
    pub cpu_freed: f64,
    /// Estimated seconds lost against the fastest lane, summed over launches placed elsewhere.
    pub lost: f64,
    /// The process's CPU time (user + sys) over the batch. Other threads' work counts too: a profile reads
    /// `lane_cpu` and `phases` instead.
    pub cpu: f64,
    /// Wall seconds of [`Executor::run`] as a whole (from `begin` to `wait`'s return), and of its placement.
    pub total: f64,
    pub place: f64,
    /// Per lane: CPU seconds of the lane's worker thread over its submission (`CLOCK_THREAD_CPUTIME_ID`), and what
    /// the backend says of its phases.
    pub lane_cpu: [f64; 4],
    pub phases: [Phases; 4],
}

impl Report {
    /// Wall seconds of the run that were neither placement nor the slowest lane's submission: handing the shares to
    /// the lanes' workers, the caller's work meanwhile (`begin` to `wait`), and gathering the outputs.
    pub fn overhead(&self) -> f64 {
        (self.total - self.place - self.wall.iter().copied().fold(0.0, f64::max)).max(0.0)
    }
}

/// A job's outputs, and when it started and ended.
type Reply = (Result<Vec<Output>, String>, Instant, Instant);

/// A device lane's job: a batch's claims, from which its worker takes its launches.
type Task = Arc<Claims>;

/// What a lane ran of a batch: its lane, its launches (indices into the batch), and the reply.
type Got = (Lane, Vec<usize>, Reply);

/// How far behind its plan a lane must be before another takes its launch: by more than the taker's own time for
/// it, and at least this (seconds).
const LATE_MIN: f64 = 200e-6;

/// The lanes this process has, their prices, their workers and their scoreboard.
pub struct Executor {
    backends: Vec<Arc<dyn Backend>>,
    /// Per backend: its worker thread's queue, for a device lane; `None` for the cores (their jobs go to the pool).
    workers: Vec<Option<std::sync::mpsc::Sender<Task>>>,
    board: Arc<Scoreboard>,
    pub prices: Prices,
}

/// Runs `ks` on `backend` as one job: started and finished on the scoreboard, a panic made an error.
fn run_job(backend: &dyn Backend, board: &Scoreboard, ks: &[&Kernel], mut ticket: Ticket) -> Reply {
    board.start(&mut ticket);
    let start = Instant::now();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| backend.submit(ks))).unwrap_or_else(|_| Err("its submission panicked".into()));
    board.finish(ticket, r.is_ok());
    (r, start, Instant::now())
}

/// A device lane's submission in flight: begun, its outputs not yet read.
struct Flight<'a> {
    claims: Arc<Claims>,
    ids: Vec<usize>,
    ticket: Ticket,
    start: Instant,
    pending: Result<Box<dyn Pending + 'a>, String>,
}

impl Flight<'_> {
    /// Waits for it and hands its outputs to the batch's waiter.
    fn end(self, board: &Scoreboard, lane: Lane) {
        let Flight { claims, ids, ticket, start, pending } = self;
        let r = pending.and_then(|p| std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.finish())).unwrap_or_else(|_| Err("its submission panicked".into())));
        board.finish(ticket, r.is_ok());
        claims.reply((lane, ids, (r, start, Instant::now())));
    }
}

/// A thread of the process's own that is no job: a service that waits (a writer on its queue, a sampler on its
/// clock, a watcher on a signal, a device lane's worker on its batches), so it may not hold one of the pool's threads
/// (decision 0100 §12: nothing that waits for another long job may itself be a job). Every thread the process makes
/// outside the one pool is made here, named, and listed while it lives ([`services`]); the engine's source guard
/// (`executors_are_shared`) refuses a thread made anywhere else. `stack`: its stack's bytes, else the default.
pub fn service<T: Send + 'static>(name: &str, stack: Option<usize>, f: impl FnOnce() -> T + Send + 'static) -> std::io::Result<std::thread::JoinHandle<T>> {
    let mut b = std::thread::Builder::new().name(name.to_owned());
    if let Some(s) = stack {
        b = b.stack_size(s);
    }
    let named = name.to_owned();
    b.spawn(move || {
        struct Listed(String);
        impl Drop for Listed {
            fn drop(&mut self) {
                let mut l = SERVICES.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(i) = l.iter().position(|n| *n == self.0) {
                    l.swap_remove(i);
                }
            }
        }
        SERVICES.lock().unwrap_or_else(|e| e.into_inner()).push(named.clone());
        let _listed = Listed(named);
        f()
    })
}

/// Runs `f` on scoped threads of the caller's (`std::thread::scope`), each one a [`service`]'s kind: a device's
/// worker per unit (SME's, the Neural Engine's predictor) that waits on its device, not a job of the pool's.
pub fn scoped<'env, T>(f: impl for<'scope> FnOnce(&'scope std::thread::Scope<'scope, 'env>) -> T) -> T {
    std::thread::scope(f)
}

static SERVICES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// The names of the [`service`] threads alive now.
pub fn services() -> Vec<String> {
    SERVICES.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Drops `x` beside the caller, as a job on the pool: freeing what it holds is no step's work, and a drop waits for
/// nothing.
pub fn drop_beside<T: Send + 'static>(x: T) {
    rayon::spawn(move || drop(x));
}

/// The worker of a device lane, until the executor drops: two submissions deep (decision 0100 §11).
///
/// It keeps every batch it is sent while that batch has a launch it may yet take. Its next submission is the
/// oldest batch's launches planned on it, else what it may take from a lane behind its plan; it begins that one
/// before it waits for the one in flight, so the device runs one while the host encodes the next. With nothing to
/// begin it waits for the one in flight, else sleeps until the soonest launch it may take falls due, or a batch
/// comes.
fn worker(backend: Arc<dyn Backend>, board: Arc<Scoreboard>) -> std::sync::mpsc::Sender<Task> {
    use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
    let (send, take) = std::sync::mpsc::channel::<Task>();
    service(&format!("rerail-accel {}", backend.lane().name()), None, move || {
            let lane = backend.lane();
            let mut open: std::collections::VecDeque<Arc<Claims>> = Default::default();
            let mut flight: Option<Flight<'_>> = None;
            let mut ended = false;
            loop {
                if flight.is_none() && open.is_empty() {
                    if ended {
                        return;
                    }
                    match take.recv() {
                        Ok(c) => open.push_back(c),
                        Err(_) => return,
                    }
                }
                loop {
                    match take.try_recv() {
                        Ok(c) => open.push_back(c),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            ended = true;
                            break;
                        }
                    }
                }
                let next = open.iter().find_map(|c| c.next_device(lane).map(|ids| (Arc::clone(c), ids)));
                if let Some((claims, ids)) = next {
                    let mut ticket = board.claimed(lane, ids.iter().map(|&i| claims.item(i, lane)).collect(), claims.begun);
                    board.start(&mut ticket);
                    let start = Instant::now();
                    let pending = {
                        let ks: Vec<&Kernel> = ids.iter().map(|&i| &claims.kernels[i]).collect();
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| backend.start(&ks))).unwrap_or_else(|_| Err("its submission panicked".into()))
                    };
                    if let Some(f) = flight.replace(Flight { claims, ids, ticket, start, pending }) {
                        f.end(&board, lane);
                    }
                    continue;
                }
                if let Some(f) = flight.take() {
                    f.end(&board, lane);
                    continue;
                }
                open.retain(|c| c.takeable_in(lane).is_some());
                let Some(wait) = open.iter().filter_map(|c| c.takeable_in(lane)).reduce(f64::min) else { continue };
                let wait = std::time::Duration::from_secs_f64(wait.clamp(20e-6, 0.05));
                if ended {
                    std::thread::sleep(wait);
                    continue;
                }
                match take.recv_timeout(wait) {
                    Ok(c) => open.push_back(c),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => ended = true,
                }
            }
        })
        .expect("a lane's worker thread");
    send
}

/// A batch's launches, claimed as the lanes reach them (decision 0100 §11).
///
/// Placement makes a **plan**: each launch's lane, and when that lane should start it. Every lane that can run a
/// launch has it on its list. A lane takes its own launches first, in its order: the cores largest first, a device
/// all of them in one submission. It takes another lane's launch only while that launch is free, and only when the
/// other lane is **behind its plan** by more than the taker's own time for the launch. Whoever claims a launch
/// first runs it; a claim is one atomic exchange, so no launch runs twice.
///
/// So a spike on one lane moves its work to the others as it happens, and without one the plan stands. The cores
/// take from a device whose worker is still busy with earlier batches; a device, after each submission, takes the
/// cores' launches their pool has not reached; and the caller, when it waits, runs free launches itself rather than
/// stall. A pinned launch stays on its lane. A device is sent every batch it could take a launch of, planned there
/// or not, and keeps it until nothing in it is left for it ([`worker`]).
///
/// When a launch is due: on a device, once the work outstanding there is done; on the cores, once a pool thread
/// is free, the outstanding work spread over the pool's threads and the batch's launches, largest first, each on
/// the thread free soonest.
pub struct Claims {
    kernels: Vec<Kernel>,
    /// Per launch: 0 while free, else 1 + the index of the lane that claimed it.
    state: Vec<std::sync::atomic::AtomicU8>,
    plan: Vec<Lane>,
    pinned: Vec<bool>,
    /// Per launch and lane: its cost there; `None` where the lane cannot run it.
    costs: Vec<[Option<Cost>; 4]>,
    /// Per launch: the fastest lane's cost.
    fastest: Vec<Cost>,
    /// Per lane: the launches planned on it, in its order.
    order: [Vec<usize>; 4],
    /// Per launch: seconds after `begun` by which its planned lane should have started it.
    due: Vec<f64>,
    /// Per device lane: its worker started this batch, and claimed its own launches.
    started: [std::sync::atomic::AtomicBool; 4],
    begun: Instant,
    send: std::sync::Mutex<std::sync::mpsc::Sender<Got>>,
    board: Arc<Scoreboard>,
}

impl Claims {
    fn item(&self, i: usize, lane: Lane) -> Item {
        let k = &self.kernels[i];
        let c = self.costs[i][lane.index()].unwrap_or_default();
        Item { form: k.name(), units: k.units(), estimate: c.wall, host: c.host }
    }

    /// Claims launch `i` for `lane`: its plan's reservation is released.
    fn claim(&self, i: usize, lane: Lane) -> bool {
        use std::sync::atomic::Ordering::AcqRel;
        let won = self.state[i].compare_exchange(0, 1 + lane.index() as u8, AcqRel, std::sync::atomic::Ordering::Acquire).is_ok();
        if won {
            let p = self.plan[i];
            self.board.plan(p, -1, -self.costs[i][p.index()].map_or(0.0, |c| c.wall));
        }
        won
    }

    fn free(&self, i: usize) -> bool {
        self.state[i].load(std::sync::atomic::Ordering::Acquire) == 0
    }

    /// How far behind its plan launch `i`'s lane is now (seconds; negative while on time). A device that started
    /// the batch claimed its launches, so a free one is never its.
    fn late(&self, i: usize) -> f64 {
        let p = self.plan[i];
        if p != Lane::Cpu && self.started[p.index()].load(std::sync::atomic::Ordering::Acquire) {
            return f64::NEG_INFINITY;
        }
        self.begun.elapsed().as_secs_f64() - self.due[i]
    }

    /// Whether `lane` may take launch `i`, planned elsewhere: free, not pinned, `lane` can run it, and its lane is
    /// behind its plan by more than `lane`'s time for it.
    fn may_take(&self, i: usize, lane: Lane) -> bool {
        self.threshold(i, lane).is_some_and(|t| self.late(i) > t)
    }

    /// How late launch `i`'s lane must be for `lane` to take it: `lane`'s own time for it, at least `LATE_MIN`;
    /// `None` when `lane` may never take it (planned on `lane`, pinned, claimed, or `lane` cannot run it).
    fn threshold(&self, i: usize, lane: Lane) -> Option<f64> {
        if self.plan[i] == lane || self.pinned[i] || !self.free(i) {
            return None;
        }
        let c = self.costs[i][lane.index()]?;
        let own = c.wall + if lane == Lane::Cpu { 0.0 } else { c.finish - c.wall - self.due_on(lane) };
        Some(own.max(LATE_MIN))
    }

    /// Seconds from now until `lane` may take a launch of this batch (0 when it may now); `None` when there is
    /// none it may ever take.
    fn takeable_in(&self, lane: Lane) -> Option<f64> {
        let now = self.begun.elapsed().as_secs_f64();
        (0..self.plan.len())
            .filter_map(|i| {
                let t = self.threshold(i, lane)?;
                let late = self.late(i);
                // A lane whose worker claimed its launches never leaves one free: nothing to wait for there.
                (late > f64::NEG_INFINITY).then(|| (self.due[i] + t - now).max(0.0))
            })
            .reduce(f64::min)
    }

    /// A device's next submission from this batch: its own launches the first time it looks (it has started the
    /// batch after), else what it may take from a lane behind its plan; `None` when there is nothing now.
    fn next_device(&self, lane: Lane) -> Option<Vec<usize>> {
        use std::sync::atomic::Ordering::AcqRel;
        if !self.started[lane.index()].swap(true, AcqRel) {
            let own: Vec<usize> = self.order[lane.index()].iter().copied().filter(|&i| self.free(i) && self.claim(i, lane)).collect();
            if !own.is_empty() {
                return Some(own);
            }
        }
        let late: Vec<usize> = std::iter::from_fn(|| self.take_late(lane)).collect();
        (!late.is_empty()).then_some(late)
    }

    /// When `lane`'s plan had it start (a device's latency is part of its finish, not of its start).
    fn due_on(&self, lane: Lane) -> f64 {
        (0..self.plan.len()).find(|&i| self.plan[i] == lane).map_or(0.0, |i| self.due[i])
    }

    /// The next launch for a pool thread: the cores' own, in their order, else one taken from a lane behind its plan.
    fn next_cpu(&self) -> Option<usize> {
        if let Some(i) = self.order[Lane::Cpu.index()].iter().copied().find(|&i| self.free(i) && self.claim(i, Lane::Cpu)) {
            return Some(i);
        }
        self.take_late(Lane::Cpu)
    }

    /// A launch `lane` takes from a lane behind its plan, from the end of that lane's order (what it reaches last).
    fn take_late(&self, lane: Lane) -> Option<usize> {
        for o in &self.order {
            if let Some(i) = o.iter().rev().copied().find(|&i| self.may_take(i, lane) && self.claim(i, lane)) {
                return Some(i);
            }
        }
        None
    }

    fn reply(&self, got: Got) {
        let _ = self.send.lock().unwrap_or_else(|p| p.into_inner()).send(got);
    }

    /// Runs launch `i` on the cores, on this thread, as a job of its own.
    fn run_cpu(&self, backend: &dyn Backend, i: usize) {
        let ticket = self.board.claimed(Lane::Cpu, vec![self.item(i, Lane::Cpu)], self.begun);
        let r = run_job(backend, &self.board, &[&self.kernels[i]], ticket);
        self.reply((Lane::Cpu, vec![i], r));
    }

    /// A pool thread's share: launches until none is left for it.
    fn serve_cpu(&self, backend: &dyn Backend) {
        while let Some(i) = self.next_cpu() {
            self.run_cpu(backend, i);
        }
    }
}

/// A batch the lanes are running ([`Executor::begin`]).
pub struct Running {
    claims: Arc<Claims>,
    take: Receiver<Got>,
    /// The cores' backend, when the executor has the cores: the caller runs free launches on it while it waits.
    cpu: Option<Arc<dyn Backend>>,
    tag: &'static str,
    report: Report,
    cpu0: f64,
}


/// A host job ([`Executor::spawn`]) the pool is running.
pub struct Job<R> {
    name: &'static str,
    take: Receiver<(std::thread::Result<R>, Instant)>,
}

impl<R> Job<R> {
    /// Waits for the job: its result, or an error when it panicked. On one of the pool's own threads, the wait runs
    /// the pool's other jobs meanwhile, so jobs that wait for jobs cannot take every thread and stall.
    pub fn wait(self) -> Result<R, String> {
        let mut wait = Waiting::new(self.name);
        let got = if on_pool() {
            loop {
                match self.take.try_recv() {
                    Ok(r) => break Ok(r),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break Err(()),
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        wait.polled();
                        if !wait.help() {
                            match self.take.recv_timeout(std::time::Duration::from_micros(100)) {
                                Ok(r) => break Ok(r),
                                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                    break Err(());
                                }
                                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                            }
                        }
                    }
                }
            }
        } else {
            self.take.recv().map_err(|_| ())
        };
        let got = got.map(|(r, ended)| {
            wait.end(Some(ended));
            r
        });
        match got {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(_)) => Err(format!("host job {}: it panicked", self.name)),
            Err(()) => Err(format!("host job {}: its worker ended", self.name)),
        }
    }
}

impl Executor {
    /// The executor's scoreboard, shared: a host stream ([`crate::stream`]) scores its jobs on it.
    pub fn board(&self) -> Arc<Scoreboard> {
        Arc::clone(&self.board)
    }

    /// Every lane this machine has, with the default prices.
    pub fn new() -> Self {
        configure(false);
        let present = lanes::present();
        let mut backends: Vec<Arc<dyn Backend>> = vec![Arc::new(CpuBackend::default())];
        #[cfg(all(feature = "sme", target_os = "macos", target_arch = "aarch64"))]
        if present[Lane::Sme.index()] {
            backends.extend(crate::sme::backend());
        }
        #[cfg(all(feature = "gpu", target_os = "macos"))]
        if present[Lane::Gpu.index()] {
            backends.extend(crate::gpu::backend());
        }
        #[cfg(all(feature = "ane", target_os = "macos"))]
        if present[Lane::Ane.index()] {
            backends.extend(crate::ane::backend());
        }
        let _ = present;
        Executor::with(backends)
    }

    /// Only the given lanes' backends (tests; a binding that allows only some lanes).
    pub fn with(backends: Vec<Arc<dyn Backend>>) -> Self {
        configure(false);
        let mut slots = [0; 4];
        for b in &backends {
            slots[b.lane().index()] = if b.lane() == Lane::Cpu { threads() } else { 1 };
        }
        let board = Arc::new(Scoreboard::new(slots));
        let workers = backends.iter().map(|b| (b.lane() != Lane::Cpu).then(|| worker(Arc::clone(b), Arc::clone(&board)))).collect();
        Executor { backends, workers, board, prices: Prices::default() }
    }

    pub fn lanes(&self) -> Vec<Lane> {
        self.backends.iter().map(|b| b.lane()).collect()
    }

    /// The scoreboard now: every job this executor ran or runs, by lane and by form.
    pub fn score(&self) -> Score {
        self.board.score()
    }

    /// Runs `f` on the caller's thread as one job of `lane`, under `form`: queued, started and finished on the
    /// scoreboard as a worker's job is, its thread CPU and phases ([`crate::score::add`]) its own. For work a caller
    /// encodes and submits itself to the lane's one queue (a device stratum's command buffers), so the lane's
    /// scoreboard holds every job it ran (decision 0100: no executor beside it).
    pub fn inline<R, E>(&self, lane: Lane, form: &'static str, units: f64, f: impl FnOnce() -> Result<R, E>) -> Result<R, E> {
        let mut ticket = self.board.queue(lane, vec![Item { form, units, estimate: 0.0, host: 0.0 }]);
        self.board.start(&mut ticket);
        let r = f();
        self.board.finish(ticket, r.is_ok());
        r
    }

    /// Appends the scoreboard to the trace (`RERAIL_SCOREBOARD`) under `title`.
    pub fn note(&self, title: &str) {
        self.board.note(title);
    }

    /// Drops what every lane keeps for the join id `id`: its relation changed, or no later launch reads it. A lane
    /// still running a batch that reads it keeps it until that batch ends.
    pub fn forget(&self, id: u64) {
        for b in &self.backends {
            b.forget(id);
        }
    }

    /// Drops everything every lane keeps: the chunk's end.
    pub fn clear(&self) {
        for b in &self.backends {
            b.clear();
        }
    }

    /// What each lane keeps, by lane.
    pub fn space(&self) -> [SpaceCounts; 4] {
        let mut out = [SpaceCounts::default(); 4];
        for b in &self.backends {
            out[b.lane().index()] = b.space();
        }
        out
    }

    /// The load placement weighs a batch of kind `tag` against: each lane's outstanding seconds and measured
    /// latency, the cores' slowdown under the machine's load, and the kind's slack.
    pub fn load(&self, tag: &'static str) -> Load {
        let measured = self.board.measured();
        let mut latency = self.prices.latency;
        let mut host_scale = [1.0; 4];
        for ((l, h), (lat, _, ratio)) in latency.iter_mut().zip(host_scale.iter_mut()).zip(measured) {
            if let Some(v) = lat {
                *l = v;
            }
            if let Some(r) = ratio {
                *h = r;
            }
        }
        Load { queued: self.board.outstanding(), latency, open: [false; 4], cores: cores_slowdown(), slack: self.board.slack(tag), host_scale }
    }

    /// The prices with what the scoreboard measured: a device job's host CPU.
    fn measured_prices(&self) -> Prices {
        let mut p = self.prices.clone();
        // A device job's fixed host CPU, measured; the per-row part is the price's, scaled by the lane's ratio.
        for (l, (_, cpu, _)) in Lane::ALL.into_iter().zip(self.board.measured()) {
            if let (Some(c), true) = (cpu, l == Lane::Gpu) {
                p.host[l.index()].submission = c;
            }
        }
        p
    }

    /// Places each launch of a batch of kind `tag`, largest first, by host CPU (`Prices::choose_host`): per launch,
    /// in the batch's order, its lane, its cost there and the fastest lane's; and the load it was weighed against.
    pub fn place(&self, tag: &'static str, batch: &Batch) -> Result<(Vec<(Lane, Cost, Cost)>, Load), String> {
        let predicted = self.predict(batch, &self.load(tag));
        self.place_predicted(tag, batch, &predicted)
    }

    /// The learned model's cost of every launch of `batch` on every lane that can run it ([`crate::cost_model`]),
    /// one call for the batch under `load`.
    pub fn predict(&self, batch: &Batch, load: &Load) -> Vec<[Option<crate::cost_model::Predicted>; 4]> {
        let kernels: Vec<(&Kernel, Vec<Lane>)> = batch.0.iter().map(|l| (&l.kernel, self.backends.iter().filter(|b| b.can(&l.kernel)).map(|b| b.lane()).collect())).collect();
        crate::cost_model::predict_batch(&kernels, load.cores)
    }

    /// [`Executor::place`] with the model's costs (`predicted`, per launch by lane).
    pub fn place_predicted(&self, tag: &'static str, batch: &Batch, predicted: &[[Option<crate::cost_model::Predicted>; 4]]) -> Result<(Vec<(Lane, Cost, Cost)>, Load), String> {
        let mut order: Vec<usize> = (0..batch.0.len()).collect();
        order.sort_by(|&a, &b| batch.0[b].kernel.units().total_cmp(&batch.0[a].kernel.units()));
        let prices = self.measured_prices();
        let start = self.load(tag);
        let mut load = start;
        let mut placed = vec![(Lane::Cpu, Cost::default(), Cost::default()); batch.0.len()];
        for i in order {
            let l = &batch.0[i];
            // A launch the caller did not pin goes to the measurement's lane when that lane can run it.
            let pin = l.pin.or_else(|| measured_lane().filter(|&m| self.backends.iter().any(|b| b.lane() == m && b.can(&l.kernel))));
            let able: Vec<Lane> = self.backends.iter().filter(|b| pin.is_none_or(|p| p == b.lane()) && b.can(&l.kernel)).map(|b| b.lane()).collect();
            let (lane, cost, fastest) = prices.choose_host_predicted(&l.kernel, &able, &load, &predicted[i]).ok_or_else(|| {
                format!("no lane can run launch {i} ({}, {:.3e} units){}", l.kernel.form().name(), l.kernel.units(), l.pin.map_or(String::new(), |p| format!(" on {}", p.name())))
            })?;
            let at = lane.index();
            if !load.open[at] {
                load.open[at] = true;
                load.queued[at] += load.latency[at];
            }
            load.queued[at] += cost.wall;
            placed[i] = (lane, cost, fastest);
        }
        Ok((placed, start))
    }

    /// Runs `f` on the caller's thread, scored as `lane`'s phase `name`: host work around a batch, such as filling
    /// its staged columns after `begin`.
    pub fn phase<R>(&self, lane: Lane, name: &'static str, f: impl FnOnce() -> R) -> R {
        let (c0, t0) = (crate::score::thread_cpu(), Instant::now());
        let r = f();
        self.board.add_phase(lane, name, crate::score::thread_cpu() - c0, t0.elapsed().as_secs_f64());
        r
    }

    /// Runs a batch: placed, its jobs on every lane at once; returns when every job is done.
    pub fn run(&self, batch: Batch) -> Result<(Vec<Done>, Report), String> {
        self.begin(batch)?.wait()
    }

    /// [`Executor::begin`] for a batch of kind `tag`: its placement, the overlap its caller gives it before it waits
    /// and its stall are scored under the kind, and the kind's measured overlap is its slack.
    pub fn run_as(&self, tag: &'static str, batch: Batch) -> Result<(Vec<Done>, Report), String> {
        self.begin_as(tag, batch)?.wait()
    }

    /// Places a batch and queues its jobs, without waiting: the lanes run while the caller works.
    pub fn begin(&self, batch: Batch) -> Result<Running, String> {
        self.begin_as("batch", batch)
    }

    /// Places a batch of kind `tag` and queues its jobs, without waiting: the plan, and the claims the lanes take
    /// their launches from (decision 0100 §11).
    pub fn begin_as(&self, tag: &'static str, batch: Batch) -> Result<Running, String> {
        let cpu0 = process_cpu();
        let begun = Instant::now();
        let predicted = self.predict(&batch, &self.load(tag));
        let (costs, load) = self.place_predicted(tag, &batch, &predicted)?;
        let place = begun.elapsed().as_secs_f64();
        self.board.placed(tag, &costs, load.slack);
        let prices = self.measured_prices();
        let n = batch.0.len();
        // Every lane's cost of every launch, for the claims; the plan's lane, and when it should start each.
        let mut all: Vec<[Option<Cost>; 4]> = vec![[None; 4]; n];
        let mut pinned = vec![false; n];
        let mut order: [Vec<usize>; 4] = Default::default();
        let mut due = vec![0.0; n];
        let mut per_lane = [0usize; 4];
        for (i, l) in batch.0.iter().enumerate() {
            let measured = measured_lane().filter(|&m| self.backends.iter().any(|b| b.lane() == m && b.can(&l.kernel)));
            pinned[i] = l.pin.is_some() || measured.is_some();
            for b in &self.backends {
                if b.can(&l.kernel) {
                    all[i][b.lane().index()] = prices.cost_predicted(&l.kernel, b.lane(), &load, predicted[i][b.lane().index()]);
                }
            }
            let (lane, c, _) = costs[i];
            all[i][lane.index()] = Some(c);
            if lane != Lane::Cpu {
                due[i] = load.queued[lane.index()];
            }
            per_lane[lane.index()] += 1;
        }
        let mut by_size: Vec<usize> = (0..n).collect();
        by_size.sort_by(|&a, &b| batch.0[b].kernel.units().total_cmp(&batch.0[a].kernel.units()));
        for &i in &by_size {
            order[costs[i].0.index()].push(i);
        }
        // The cores' launches start as the pool's threads free up: the work outstanding on the cores spread over the
        // threads, then each launch, largest first, on the thread free soonest (the cores' slowdown is in its wall).
        let mut free_at = vec![load.queued[Lane::Cpu.index()] / threads() as f64; threads()];
        for &i in &order[Lane::Cpu.index()] {
            let t = free_at.iter_mut().min_by(|a, b| a.total_cmp(b)).expect("a thread");
            due[i] = *t;
            *t += costs[i].1.wall;
        }
        for lane in Lane::ALL {
            let est: f64 = order[lane.index()].iter().map(|&i| costs[i].1.wall).sum();
            self.board.plan(lane, per_lane[lane.index()] as i64, est);
        }
        // The engine's launch budget, checked on the submitting thread (decision 0100 §2).
        #[cfg(all(feature = "gpu", target_os = "macos"))]
        if per_lane[Lane::Gpu.index()] > 0 {
            crate::gpu::shared::guard().map_err(|e| format!("gpu lane: {e}"))?;
        }
        let (send, take) = std::sync::mpsc::channel();
        let claims = Arc::new(Claims {
            kernels: batch.0.into_iter().map(|l| l.kernel).collect(),
            state: (0..n).map(|_| std::sync::atomic::AtomicU8::new(0)).collect(),
            plan: costs.iter().map(|c| c.0).collect(),
            pinned,
            costs: all,
            fastest: costs.iter().map(|c| c.2).collect(),
            order,
            due,
            started: Default::default(),
            begun,
            send: std::sync::Mutex::new(send),
            board: Arc::clone(&self.board),
        });
        let mut cpu = None;
        for (at, b) in self.backends.iter().enumerate() {
            let lane = b.lane();
            match &self.workers[at] {
                // The cores: as many pool jobs as their planned launches (at most the pool's threads), each taking
                // launches until none is left for it.
                None => {
                    cpu = Some(Arc::clone(b));
                    for _ in 0..per_lane[lane.index()].min(threads()) {
                        let (claims, backend) = (Arc::clone(&claims), Arc::clone(b));
                        rayon::spawn_fifo(move || as_job("the cores' launches", || claims.serve_cpu(&*backend)));
                    }
                }
                // A device lane: its worker claims its share when it reaches the batch, and a device with nothing
                // planned may still take the others' launches once they fall behind.
                Some(w) => {
                    if per_lane[lane.index()] > 0 || (0..n).any(|i| !claims.pinned[i] && claims.costs[i][lane.index()].is_some()) {
                        w.send(Arc::clone(&claims)).map_err(|_| format!("{} lane: its worker ended", lane.name()))?;
                    }
                }
            }
        }
        Ok(Running { claims, take, cpu, tag, report: Report { place, ..Report::default() }, cpu0 })
    }


    /// Runs `f` as a job on the cores' pool, scored as host work named `name` (`estimate`: the caller's guess of
    /// its seconds, for the placement of what follows; 0 when unknown).
    pub fn spawn<R: Send + 'static>(&self, name: &'static str, estimate: f64, f: impl FnOnce() -> R + Send + 'static) -> Job<R> {
        let mut ticket = self.board.queue(Lane::Cpu, vec![Item { form: name, units: 0.0, estimate, host: estimate }]);
        let (reply, take) = std::sync::mpsc::sync_channel(1);
        let board = Arc::clone(&self.board);
        rayon::spawn_fifo(move || {
            board.start(&mut ticket);
            let r = as_job(name, || std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
            board.finish(ticket, r.is_ok());
            let _ = reply.send((r, Instant::now()));
        });
        Job { name, take }
    }
}

impl Running {
    /// Waits for every launch: each one's output, in the batch's order, and the batch's report. Meanwhile the
    /// caller runs free launches itself: the cores' that the pool has not reached, and those of a lane behind its
    /// plan (decision 0100 §11).
    pub fn wait(self) -> Result<(Vec<Done>, Report), String> {
        let Running { claims, take, cpu, tag, mut report, cpu0 } = self;
        let asked = Instant::now();
        let on_pool = on_pool();
        let n = claims.kernels.len();
        let mut outs: Vec<Option<(Lane, Output)>> = vec![None; n];
        let mut failed: Option<String> = None;
        let mut span: [Option<(Instant, Instant)>; 4] = [None; 4];
        let mut ran = [0.0f64; 4];
        let mut got = 0usize;
        let mut helped = 0u64;
        let mut wait = Waiting::new("a batch's launches");
        let mut store = |(lane, ids, (r, start, end)): Got, got: &mut usize| {
            *got += ids.len();
            let s = &mut span[lane.index()];
            *s = Some(s.map_or((start, end), |(a, b)| (a.min(start), b.max(end))));
            ran[lane.index()] += end.duration_since(start).as_secs_f64();
            match r {
                Ok(r) if r.len() == ids.len() => {
                    for (&i, o) in ids.iter().zip(r) {
                        outs[i] = Some((lane, o));
                    }
                }
                Ok(r) => {
                    failed.get_or_insert(format!("{} lane: {} outputs for {} launches", lane.name(), r.len(), ids.len()));
                }
                Err(e) => {
                    failed.get_or_insert(format!("{} lane: {e}", lane.name()));
                }
            }
        };
        while got < n {
            match take.try_recv() {
                Ok(g) => {
                    store(g, &mut got);
                    // More to wait for: the job last run did not hold up the batch's end.
                    if got < n {
                        wait.polled();
                    }
                    continue;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                Err(std::sync::mpsc::TryRecvError::Empty) => wait.polled(),
            }
            // Free launches the caller can run: the cores' own, or one a lane behind its plan has not reached.
            let mine = cpu.as_ref().and_then(|cpu| {
                let i = claims.order[Lane::Cpu.index()].iter().rev().copied().find(|&i| claims.free(i) && claims.claim(i, Lane::Cpu)).or_else(|| claims.take_late(Lane::Cpu))?;
                Some((cpu, i))
            });
            if let Some((cpu, i)) = mine {
                let ticket = claims.board.claimed(Lane::Cpu, vec![claims.item(i, Lane::Cpu)], claims.begun);
                let r = run_job(&**cpu, &claims.board, &[&claims.kernels[i]], ticket);
                store((Lane::Cpu, vec![i], r), &mut got);
                helped += 1;
                continue;
            }
            // On one of the pool's threads, the pool's other jobs meanwhile.
            if on_pool && wait.help() {
                continue;
            }
            match take.recv_timeout(std::time::Duration::from_micros(100)) {
                Ok(g) => store(g, &mut got),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        if let Some(e) = failed {
            return Err(e);
        }
        if got < n {
            return Err(format!("{} of {n} launches were never run: a lane's worker ended", n - got));
        }
        // Ready when the last lane's launch ended.
        wait.end(span.iter().flatten().map(|(_, b)| *b).max());
        for lane in Lane::ALL {
            if let Some((a, b)) = span[lane.index()] {
                report.wall[lane.index()] = b.duration_since(a).as_secs_f64();
            }
            report.joules[lane.index()] = WATTS[lane.index()] * ran[lane.index()];
        }
        report.cpu = process_cpu() - cpu0;
        let mut on = [0u64; 4];
        let mut moved = 0u64;
        let mut done = Vec::with_capacity(n);
        for (i, o) in outs.into_iter().enumerate() {
            let (lane, out) = o.expect("every launch ran");
            let c = claims.costs[i][lane.index()].unwrap_or_default();
            let fastest = claims.fastest[i];
            on[lane.index()] += 1;
            moved += u64::from(lane != claims.plan[i]);
            report.launches[lane.index()] += 1;
            report.lost += (c.wall - fastest.wall).max(0.0);
            if lane != Lane::Cpu {
                if let Some(cc) = claims.costs[i][Lane::Cpu.index()] {
                    report.cpu_freed += cc.wall;
                }
            }
            done.push(Done { out, lane, estimate: c.wall, fastest: fastest.wall, host: c.host });
        }
        report.total = claims.begun.elapsed().as_secs_f64();
        claims.board.collected(tag, asked.duration_since(claims.begun).as_secs_f64(), asked.elapsed().as_secs_f64(), on, moved, helped);
        Ok((done, report))
    }
}


/// A batch whose launches are taken one by one as each ends ([`Running::each`]): one submission to the lanes, and
/// a consumer of launch `i` waits for `i` alone, not for the batch's slowest (decision 0114 §2).
pub struct Each {
    claims: Arc<Claims>,
    cpu: Option<Arc<dyn Backend>>,
    state: std::sync::Mutex<EachState>,
    tag: &'static str,
}

struct EachState {
    take: Receiver<Got>,
    outs: Vec<Option<Result<Output, String>>>,
    ended: bool,
}

impl Running {
    /// The batch, its launches taken one at a time ([`Each::take`]).
    pub fn each(self) -> Each {
        let Running { claims, take, cpu, tag, .. } = self;
        let n = claims.kernels.len();
        Each { claims, cpu, state: std::sync::Mutex::new(EachState { take, outs: (0..n).map(|_| None).collect(), ended: false }), tag }
    }
}

impl EachState {
    fn store(&mut self, (lane, ids, (r, _, _)): Got) {
        match r {
            Ok(r) if r.len() == ids.len() => {
                for (&i, o) in ids.iter().zip(r) {
                    self.outs[i] = Some(Ok(o));
                }
            }
            Ok(r) => {
                let e = format!("{} lane: {} outputs for {} launches", lane.name(), r.len(), ids.len());
                ids.iter().for_each(|&i| self.outs[i] = Some(Err(e.clone())));
            }
            Err(e) => ids.iter().for_each(|&i| self.outs[i] = Some(Err(format!("{} lane: {e}", lane.name())))),
        }
    }
}

impl Each {
    /// Launch `i`'s output, once it ended: run here if no lane claimed it yet; meanwhile the outputs that come in
    /// are kept for their takers, and the pool's jobs are helped.
    pub fn take(&self, i: usize) -> Result<Output, String> {
        let mut wait = Waiting::new("a launch of a batch");
        loop {
            {
                let mut st = self.state.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(o) = st.outs[i].take() {
                    return o;
                }
                while !st.ended {
                    match st.take.try_recv() {
                        Ok(g) => st.store(g),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => st.ended = true,
                    }
                }
                if let Some(o) = st.outs[i].take() {
                    return o;
                }
                if st.ended {
                    return Err(format!("{}: launch {i} was never run: a lane's worker ended", self.tag));
                }
            }
            // Its own launch, if no lane has it: the caller runs it on the cores.
            if let Some(cpu) = &self.cpu
                && self.claims.free(i)
                && self.claims.claim(i, Lane::Cpu)
            {
                let ticket = self.claims.board.claimed(Lane::Cpu, vec![self.claims.item(i, Lane::Cpu)], self.claims.begun);
                let r = run_job(&**cpu, &self.claims.board, &[&self.claims.kernels[i]], ticket);
                self.state.lock().unwrap_or_else(|p| p.into_inner()).store((Lane::Cpu, vec![i], r));
                continue;
            }
            if on_pool() && wait.help() {
                continue;
            }
            wait.polled();
            std::thread::sleep(std::time::Duration::from_micros(50));
        }
    }
}

impl Default for Executor {
    fn default() -> Self {
        Executor::new()
    }
}

/// The CPU lane: the reference kernels, a job a launch on the executor's pool (one parallel pass when called with
/// several). Its space keeps a join's right side sorted by key 0, by the join's id.
#[derive(Default)]
pub struct CpuBackend {
    space: std::sync::Mutex<CpuSpace>,
}

#[derive(Default)]
struct CpuSpace {
    sorted: std::collections::HashMap<u64, (usize, Arc<crate::rows::Keyed>)>,
    bytes: usize,
    counts: SpaceCounts,
}

impl CpuBackend {
    /// The sort of a join's right side: kept by its id, or made (and kept when it has one).
    fn keyed(&self, right: &crate::kernel::Keys, id: Option<u64>) -> Result<Arc<crate::rows::Keyed>, String> {
        let rows = right[0].len();
        let Some(id) = id else {
            return Ok(Arc::new(crate::rows::Keyed::new(&right[0])));
        };
        let mut s = self.space.lock().map_err(|_| "the cpu lane's space was poisoned")?;
        if let Some((n, k)) = s.sorted.get(&id) {
            if *n != rows {
                return Err(format!("join: id {id} was kept with {n} rows, now names {rows}"));
            }
            let k = Arc::clone(k);
            s.counts.hits += 1;
            return Ok(k);
        }
        s.counts.misses += 1;
        drop(s);
        let k = Arc::new(crate::rows::Keyed::new(&right[0]));
        let mut s = self.space.lock().map_err(|_| "the cpu lane's space was poisoned")?;
        // The sort's order, keys and runs.
        s.bytes += rows * 12 + k.runs.len() * 16;
        s.sorted.insert(id, (rows, Arc::clone(&k)));
        Ok(k)
    }

    fn join(&self, k: &Kernel) -> Result<Output, String> {
        if let Kernel::Program(p) = k {
            return p.0.run(Lane::Cpu).map(|r| Output::Program(crate::kernel::ProgramResult(r))).map_err(|e| format!("program {}: {e}", p.0.name()));
        }
        let Kernel::Join { left, right, right_id, .. } = k else {
            return Ok(crate::cpu::run(k));
        };
        Kernel::join_shape(left, right)?;
        let keyed = self.keyed(right, *right_id)?;
        Ok(Output::Pairs(crate::cpu::join(left, right, Some(&keyed))))
    }
}

impl Backend for CpuBackend {
    fn lane(&self) -> Lane {
        Lane::Cpu
    }
    fn can(&self, k: &Kernel) -> bool {
        match k {
            Kernel::Join { left, right, .. } => Kernel::join_shape(left, right).is_ok(),
            Kernel::Program(p) => p.0.per_unit(Lane::Cpu).is_some(),
            _ => true,
        }
    }
    fn submit(&self, ks: &[&Kernel]) -> Result<Vec<Output>, String> {
        self.submit_timed(ks).map(|(o, _)| o)
    }
    /// One launch runs on the calling thread; several on the rayon pool, each worker's CPU counted.
    fn submit_timed(&self, ks: &[&Kernel]) -> Result<(Vec<Output>, Phases), String> {
        use rayon::prelude::*;
        if ks.len() == 1 {
            // The kernel parallelizes inside, on the pool; its workers' CPU is the scoreboard's (each job timed on its
            // thread). A `broadcast` to read the pool's threads waits for every one of them, the main thread's slot
            // too (`threads`: the main thread is one of the pool's), which never takes it: the run hung.
            return Ok((vec![self.join(ks[0])?], Phases::default()));
        }
        let me = std::thread::current().id();
        let ran: Vec<(Output, f64)> = ks
            .par_iter()
            .map(|k| {
                let c = thread_cpu();
                let o = self.join(k)?;
                Ok((o, if std::thread::current().id() == me { 0.0 } else { thread_cpu() - c }))
            })
            .collect::<Result<_, String>>()?;
        let worker_cpu = ran.iter().map(|r| r.1).sum();
        Ok((ran.into_iter().map(|r| r.0).collect(), Phases { worker_cpu, ..Phases::default() }))
    }
    fn forget(&self, id: u64) {
        if let Ok(mut s) = self.space.lock() {
            if let Some((rows, k)) = s.sorted.remove(&id) {
                s.bytes -= rows * 12 + k.runs.len() * 16;
            }
        }
    }
    fn clear(&self) {
        if let Ok(mut s) = self.space.lock() {
            s.sorted.clear();
            s.bytes = 0;
        }
    }
    fn space(&self) -> SpaceCounts {
        self.space.lock().map(|s| SpaceCounts { entries: s.sorted.len(), bytes: s.bytes, ..s.counts }).unwrap_or_default()
    }
}

/// The rayon pool's CPU seconds: every worker's thread clock, summed.
/// This thread's CPU seconds (`CLOCK_THREAD_CPUTIME_ID`; 0 where there is none).
pub fn thread_cpu() -> f64 {
    #[cfg(unix)]
    {
        let mut t: libc::timespec = unsafe { std::mem::zeroed() };
        if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) } != 0 {
            return 0.0;
        }
        t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
    }
    #[cfg(not(unix))]
    {
        0.0
    }
}

/// The process's user + system CPU seconds (0 where there is no `getrusage`).
pub fn process_cpu() -> f64 {
    #[cfg(unix)]
    {
        let mut u: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
        let s = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 * 1e-6;
        s(u.ru_utime) + s(u.ru_stime)
    }
    #[cfg(not(unix))]
    {
        0.0
    }
}
