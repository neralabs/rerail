//! The scoreboard: what every job of an executor did, on which lane, and how long it waited and ran.
//!
//! A **job** is what a lane's worker takes at once:
//!
//! - on the cores, one launch, or one host closure ([`crate::Executor::spawn`]), on the shared pool's threads;
//! - on SME, the GPU and the Neural Engine, a batch's share of the lane: one submission.
//!
//! Each job is counted three times: when it is queued, when a worker starts it, and when it finishes. From those,
//! per lane:
//!
//! - what is queued and running now, the most that ever ran at once, and the estimated seconds not yet done (the
//!   placement starts from these, so a batch begun while another runs is spread over the lanes);
//! - jobs, launches and failures; seconds busy (summed over jobs) and seconds waited in the queue;
//! - the price's estimate against the time taken, and its host CPU estimate against the job's thread CPU, per lane
//!   and per form;
//! - the lane's latency and a device job's thread CPU, as moving averages, which placement reads.
//!
//! And per kind of batch (`Executor::begin_as`): where its launches went, the host CPU placement estimated against
//! placing each on its fastest lane, the slack it used, and how long its caller worked before it waited (the
//! overlap, whose average is the next batch's slack) and then stalled.
//!
//! **Phases.** A lane's code names the parts of a job's host work ([`phase`]): its thread CPU and its wall seconds,
//! summed per lane and phase. The GPU lane also adds each command buffer's timeline as wall-only phases (`gpu
//! submit`: commit to the system's scheduling; `gpu schedule`: to the device's start; `gpu run`; `gpu wake`: the
//! device's end to the host's wait returning). And every job's own thread CPU is kept: on a device lane, that is the
//! host CPU the device cost, which on a loaded machine is the price that matters, not the device's latency. Metal's
//! own threads (its completion handlers, the driver's) are not on the job's thread, so they are not in it.
//!
//! **The trace.** With `RERAIL_SCOREBOARD=<file>`, every finished job appends a line to the file (tab-separated,
//! times in seconds from the process's first job):
//!
//! ```text
//! job  lane  thread  launches  forms  units  estimate  queued  start  end  ok  cpu  phases
//! ```
//!
//! `cpu` is the job's thread CPU; `phases` lists each phase as `name=cpu/wall` in microseconds.
//!
//! and [`Scoreboard::note`] appends the summary (lines that start with `#`), at a chunk's end for the engine.

use crate::lanes::Lane;
use std::collections::BTreeMap;
use std::io::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// One lane's counts.
///
/// On the cores, a job blocked in its own parallel loop may run another queued job meanwhile (the pool steals work
/// while it waits), so `running` and `peak` can pass `slots`, and `busy` counts the nested job's time twice.
#[derive(Clone, Debug, Default)]
pub struct Unit {
    /// Workers the lane has: the pool's threads for the cores, 1 for a device lane.
    pub slots: usize,
    /// Jobs waiting and running now, and the most that ran at once.
    pub queued: usize,
    pub running: usize,
    pub peak: usize,
    /// Jobs and launches finished, and jobs failed.
    pub jobs: u64,
    pub launches: u64,
    pub failed: u64,
    /// Seconds run and seconds waited, summed over finished jobs.
    pub busy: f64,
    pub waited: f64,
    /// The jobs' own thread CPU seconds, summed: on a device lane, the host CPU the device cost.
    pub cpu: f64,
    /// The price's estimate of the finished jobs, summed.
    pub estimated: f64,
    /// The estimate of the jobs queued or running now, and of the launches planned on the lane and not yet
    /// claimed.
    pub outstanding: f64,
    /// Launches planned on the lane and not yet claimed (decision 0100 §11).
    pub planned: usize,
    /// When the lane's first job started and its last ended (seconds from the epoch).
    pub first: Option<f64>,
    pub last: f64,
    /// Measured, as moving averages: the lane's latency (the GPU's submit, schedule and wake), and a job's thread
    /// CPU on a device lane. Placement reads them (`Executor::place`).
    pub latency: Option<f64>,
    pub job_cpu: Option<f64>,
    /// The share clock (seconds from the epoch): each second the lane has jobs running advances it by the second over
    /// the jobs running then. A job's share of the lane is the clock's advance from its start to its end
    /// ([`Unit::share`]); `share_at` is when it was last advanced.
    pub share_clock: f64,
    pub share_at: f64,
    /// A job's thread CPU over the price's host CPU estimate, as a moving average: placement scales the lane's
    /// host estimates by it.
    pub host_ratio: Option<f64>,
}

impl Unit {
    /// Advances the share clock to `at`, before the jobs running change.
    fn advance(&mut self, at: f64) {
        if self.running > 0 && at > self.share_at {
            self.share_clock += (at - self.share_at) / self.running as f64;
        }
        self.share_at = self.share_at.max(at);
    }

    /// What a job that ran from share clock `from` to `to` (`ran` seconds of wall) occupied of the lane. A device
    /// lane (one slot) runs jobs side by side: two waves in flight each take the device's time while both run, so
    /// a job's own wall counted that shared time twice. The shares of a lane's jobs sum to the time it had any job
    /// running. A pool lane's jobs run on threads of their own: their wall.
    fn share(&self, from: f64, to: f64, ran: f64) -> f64 {
        if self.slots == 1 { (to - from).clamp(0.0, ran) } else { ran }
    }

    /// Busy seconds over the lane's capacity in the span it worked: 1 when every worker ran from first to last.
    pub fn utilization(&self) -> f64 {
        match self.first {
            Some(f) if self.last > f && self.slots > 0 => self.busy / ((self.last - f) * self.slots as f64),
            _ => 0.0,
        }
    }
}

/// One form's counts on one lane.
#[derive(Clone, Debug, Default)]
pub struct FormRow {
    pub launches: u64,
    pub units: f64,
    pub estimated: f64,
    /// Seconds taken: a submission's time shared among its launches by their estimates.
    pub actual: f64,
    /// The price's host CPU estimate, and the job's thread CPU shared by those estimates.
    pub host_estimated: f64,
    pub host: f64,
}

/// One kind of batch's counts (`Executor::begin_as`): how its launches were placed, and how long its caller worked
/// before it waited and then stalled.
#[derive(Clone, Debug, Default)]
pub struct TagRow {
    pub batches: u64,
    pub launches: [u64; Lane::COUNT],
    /// Launches whose lane is not the first to finish: the slack or the band let a lane of less host CPU take them.
    pub not_fastest: u64,
    /// The host CPU the placement estimated, and what placing every launch on its fastest lane would have cost.
    pub host: f64,
    pub host_fastest: f64,
    /// The wall seconds the placement estimated it adds against the fastest lanes.
    pub later: f64,
    /// The slack placement used, summed (seconds), and the deadline's.
    pub slack: f64,
    /// Seconds from `begin` to the caller's `wait` (the caller's own work meanwhile), and from the `wait` to the
    /// outputs (the caller stalled), summed; the longest stall.
    pub overlap: f64,
    pub stall: f64,
    pub max_stall: f64,
    /// Batches whose caller stalled more than 0.1 ms.
    pub stalled: u64,
    /// Launches run on each lane, as claimed (decision 0100 §11); those run off the lane they were planned on,
    /// and those the waiting caller ran itself.
    pub ran: [u64; Lane::COUNT],
    pub moved: u64,
    pub helped: u64,
    /// The last overlaps (at most 32): the next batch's slack is their lower quartile, so a kind whose callers
    /// sometimes wait at once is not given the slack of its long overlaps.
    pub recent: std::collections::VecDeque<f64>,
}

impl TagRow {
    /// The slack: the lower quartile of the recent overlaps.
    pub fn slack(&self) -> f64 {
        if self.recent.is_empty() {
            return 0.0;
        }
        let mut v: Vec<f64> = self.recent.iter().copied().collect();
        v.sort_by(f64::total_cmp);
        v[v.len() / 4]
    }
}

/// One phase's counts on one lane.
#[derive(Clone, Copy, Debug, Default)]
pub struct PhaseRow {
    pub count: u64,
    /// Thread CPU seconds (0 for a device's own timeline) and wall seconds.
    pub cpu: f64,
    pub wall: f64,
}

/// The scoreboard's state at one moment.
#[derive(Clone, Debug, Default)]
pub struct Score {
    pub units: [Unit; Lane::COUNT],
    /// By (lane, form).
    pub forms: BTreeMap<(Lane, &'static str), FormRow>,
    /// By (lane, phase).
    pub phases: BTreeMap<(Lane, &'static str), PhaseRow>,
    /// By kind of batch.
    pub tags: BTreeMap<&'static str, TagRow>,
    /// Seconds since the executor's first job.
    pub elapsed: f64,
}

impl std::fmt::Display for Score {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "lane  slots  jobs  launches  failed  busy s  cpu s   waited s  estimate s  util  peak  queued  running")?;
        for l in Lane::ALL {
            let u = &self.units[l.index()];
            if u.slots == 0 {
                continue;
            }
            writeln!(
                f,
                "{:<4}  {:>5}  {:>4}  {:>8}  {:>6}  {:>6.3}  {:>6.3}  {:>8.3}  {:>10.3}  {:>4.2}  {:>4}  {:>6}  {:>7}",
                l.name(),
                u.slots,
                u.jobs,
                u.launches,
                u.failed,
                u.busy,
                u.cpu,
                u.waited,
                u.estimated,
                u.utilization(),
                u.peak,
                u.queued,
                u.running
            )?;
        }
        let measured: Vec<String> = Lane::ALL
            .iter()
            .filter(|l| {
                let u = &self.units[l.index()];
                u.latency.is_some() || u.job_cpu.is_some() || u.host_ratio.is_some()
            })
            .map(|l| {
                let u = &self.units[l.index()];
                format!(
                    "{}: latency {:.0} us, job cpu {:.0} us, host cpu / estimate {:.2}",
                    l.name(),
                    u.latency.unwrap_or(0.0) * 1e6,
                    u.job_cpu.unwrap_or(0.0) * 1e6,
                    u.host_ratio.unwrap_or(1.0)
                )
            })
            .collect();
        if !measured.is_empty() {
            writeln!(f, "measured (moving averages): {}", measured.join("; "))?;
        }
        writeln!(f, "lane  form                  launches  units       estimate s  actual s  host est ms  host cpu ms")?;
        for ((l, form), r) in &self.forms {
            writeln!(
                f,
                "{:<4}  {:<20}  {:>8}  {:>10.3e}  {:>10.4}  {:>8.4}  {:>11.3}  {:>11.3}",
                l.name(),
                form,
                r.launches,
                r.units,
                r.estimated,
                r.actual,
                r.host_estimated * 1e3,
                r.host * 1e3
            )?;
        }
        if !self.phases.is_empty() {
            writeln!(f, "lane  phase                         count     cpu ms   wall ms  cpu us each")?;
            for ((l, name), p) in &self.phases {
                writeln!(f, "{:<4}  {:<28}  {:>6}  {:>9.3}  {:>8.3}  {:>11.1}", l.name(), name, p.count, p.cpu * 1e3, p.wall * 1e3, p.cpu * 1e6 / p.count.max(1) as f64)?;
            }
        }
        if !self.tags.is_empty() {
            writeln!(
                f,
                "batch kind      batches  planned c/s/g/a  ran c/s/g/a  moved  helped  not fastest  host est ms  if fastest ms  later ms  slack ms  overlap ms  stall ms  stalled  max stall ms  slack now ms"
            )?;
            for (tag, t) in &self.tags {
                let l = t.launches;
                writeln!(
                    f,
                    "{:<14}  {:>7}  {:>15}  {:>11}  {:>5}  {:>6}  {:>11}  {:>11.3}  {:>13.3}  {:>8.3}  {:>8.3}  {:>10.3}  {:>8.3}  {:>7}  {:>12.3}  {:>12.3}",
                    tag,
                    t.batches,
                    format!("{}/{}/{}/{}", l[0], l[1], l[2], l[3]),
                    format!("{}/{}/{}/{}", t.ran[0], t.ran[1], t.ran[2], t.ran[3]),
                    t.moved,
                    t.helped,
                    t.not_fastest,
                    t.host * 1e3,
                    t.host_fastest * 1e3,
                    t.later * 1e3,
                    t.slack * 1e3,
                    t.overlap * 1e3,
                    t.stall * 1e3,
                    t.stalled,
                    t.max_stall * 1e3,
                    t.slack() * 1e3
                )?;
            }
        }
        write!(f, "elapsed {:.3} s", self.elapsed)
    }
}

/// This thread's CPU seconds.
pub fn thread_cpu() -> f64 {
    #[cfg(unix)]
    {
        let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: a valid timespec out.
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
        t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
    }
    #[cfg(not(unix))]
    {
        0.0
    }
}

thread_local! {
    /// The phases of the job this thread runs: (name, count, cpu, wall), in first-seen order.
    static PHASES: std::cell::RefCell<Vec<(&'static str, u64, f64, f64)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Adds `cpu` and `wall` seconds to this thread's job under `name`.
pub fn add(name: &'static str, cpu: f64, wall: f64) {
    PHASES.with_borrow_mut(|p| match p.iter_mut().find(|e| e.0 == name) {
        Some(e) => {
            e.1 += 1;
            e.2 += cpu;
            e.3 += wall;
        }
        None => p.push((name, 1, cpu, wall)),
    });
}

/// Counts `n` events of `name` on this thread's job (a phase with no time: sorts of a kind, command buffers).
pub fn count(name: &'static str, n: u64) {
    if n == 0 {
        return;
    }
    PHASES.with_borrow_mut(|p| match p.iter_mut().find(|e| e.0 == name) {
        Some(e) => e.1 += n,
        None => p.push((name, n, 0.0, 0.0)),
    });
}

/// Runs `f` as the phase `name` of this thread's job: its thread CPU and wall seconds are counted.
pub fn phase<R>(name: &'static str, f: impl FnOnce() -> R) -> R {
    let (c0, t0) = (thread_cpu(), Instant::now());
    let r = f();
    add(name, thread_cpu() - c0, t0.elapsed().as_secs_f64());
    r
}

/// This thread's phases so far, taken (a job's start clears them, its end takes them).
pub fn take_phases() -> Vec<(&'static str, u64, f64, f64)> {
    PHASES.with_borrow_mut(std::mem::take)
}

/// A finished job, for the cost statistics (decision 0100 §14): what it was, what the price expected, and what it
/// cost. The engine keeps them ([`keep_samples`], [`take_samples`]) and writes them to a store, where they are fitted
/// back into the prices ([`crate::price::learn_from`]); a regressor can be trained on them later.
#[derive(Clone, Debug)]
pub struct Sample {
    pub lane: Lane,
    /// Its launches' names ([`crate::Kernel::name`]), comma-joined in order.
    pub forms: String,
    pub launches: u32,
    pub units: f64,
    /// The price's wall and host CPU estimates.
    pub estimate: f64,
    pub host_estimate: f64,
    /// Its wall from start to end (on a device lane, its share of the lane while other jobs ran beside it:
    /// [`Unit::share`]), its thread's CPU, and its wait in the queue.
    pub wall: f64,
    pub cpu: f64,
    pub waited: f64,
    /// Its phases: `name=cpu/wall` in microseconds, comma-joined (a device's timeline among them).
    pub phases: String,
    /// It compiled a pipeline or made a buffer: not what the lane costs warm.
    pub cold: bool,
    pub ok: bool,
    /// The cores' slowdown under the machine's load when it ended ([`crate::exec::cores_slowdown`]).
    pub load: f64,
    /// The executor's pool threads ([`crate::exec::threads`]).
    pub threads: u32,
}

impl Sample {
    /// What a price learns from in it ([`crate::price::fit_rates`]).
    pub fn measured(&self) -> crate::price::Measured<'_> {
        let Sample { lane, ref forms, launches, units, wall, cpu, cold, ok, load, .. } = *self;
        crate::price::Measured { lane, forms, launches, units, wall, cpu, load, cold, ok }
    }
}

/// The machine's samples for its cost model, kept for the engine to take (decision 0442).
#[derive(Default)]
struct Samples {
    keep: std::sync::atomic::AtomicBool,
    held: Mutex<Vec<Sample>>,
    dropped: std::sync::atomic::AtomicU64,
}

fn samples() -> std::sync::Arc<Samples> {
    rerail_context::machine::<Samples>()
}
/// The most samples held for the engine to take: past it, new ones are dropped (and counted).
const HELD: usize = 100_000;

/// Keeps every finished job's sample from now on, for [`take_samples`].
pub fn keep_samples(on: bool) {
    samples().keep.store(on, Ordering::Relaxed);
}

/// The samples kept since the last take, and how many were dropped since (more than [`HELD`] waiting).
pub fn take_samples() -> (Vec<Sample>, u64) {
    let samples = samples();
    let s = std::mem::take(&mut *samples.held.lock().unwrap_or_else(|p| p.into_inner()));
    (s, samples.dropped.swap(0, Ordering::Relaxed))
}

/// A launch of a job: its form's name, units and estimate.
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub form: &'static str,
    pub units: f64,
    pub estimate: f64,
    /// The price's host CPU estimate.
    pub host: f64,
}

/// A job between its queueing and its end.
pub struct Ticket {
    id: u64,
    lane: Lane,
    items: Vec<Item>,
    queued: Instant,
    started: Option<Instant>,
    /// The lane's share clock when it started ([`Unit::share_clock`]).
    share0: f64,
    /// The CPU seconds counted so far, on every thread the job ran on.
    cpu: f64,
    /// The thread it runs on now and its clock then: `None` while it is handed between threads.
    cpu0: Option<(std::thread::ThreadId, f64)>,
    /// Its phases so far, gathered from each thread it ran on.
    phases: Vec<(&'static str, u64, f64, f64)>,
}

impl Ticket {
    /// The job leaves this thread: its CPU and phases here are counted. A device job begins on its lane's thread and
    /// runs its host work on the lanes' pool; reading one thread's clock at the start and another's at the end gave
    /// a difference of two unrelated clocks (158 ms a job, the lane's total past the process's).
    pub fn pause(&mut self) {
        // On another thread than the one that resumed it, the clock is not this job's: nothing is counted.
        if let Some((at, c0)) = self.cpu0.take()
            && at == std::thread::current().id()
        {
            self.cpu += thread_cpu() - c0;
        }
        for (name, n, c, w) in take_phases() {
            match self.phases.iter_mut().find(|e| e.0 == name) {
                Some(e) => {
                    e.1 += n;
                    e.2 += c;
                    e.3 += w;
                }
                None => self.phases.push((name, n, c, w)),
            }
        }
    }

    /// The job runs on this thread from now: its clock is read here, and what the thread counted before is not its.
    /// CPU seconds the job spent on a thread that did not resume it (its submission run on the lanes' pool).
    pub fn add_cpu(&mut self, seconds: f64) {
        self.cpu += seconds.max(0.0);
    }

    pub fn resume(&mut self) {
        take_phases();
        self.cpu0 = Some((std::thread::current().id(), thread_cpu()));
    }
}

/// An executor's scoreboard.
pub struct Scoreboard {
    score: Mutex<Score>,
}

/// The first job's time: the trace's zero, shared by every executor of the process.
fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

static NEXT: AtomicU64 = AtomicU64::new(1);

/// The weight of a new sample in the moving averages.
const AVG: f64 = 0.2;

/// The trace's file, opened once, its header written (`RERAIL_SCOREBOARD`).
/// The run's own (decision 0442): its setting's file.
fn trace() -> Option<std::sync::Arc<Mutex<std::fs::File>>> {
    #[derive(Default)]
    struct Trace(OnceLock<Option<std::sync::Arc<Mutex<std::fs::File>>>>);
    rerail_context::local::<Trace>()
        .0
        .get_or_init(|| {
            let path = std::env::var("RERAIL_SCOREBOARD").ok().filter(|p| !p.is_empty())?;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
            let _ = writeln!(f, "job\tlane\tthread\tlaunches\tforms\tunits\testimate\tqueued\tstart\tend\tok\tcpu\tphases");
            Some(std::sync::Arc::new(Mutex::new(f)))
        })
        .clone()
}

fn since(t: Instant) -> f64 {
    t.saturating_duration_since(epoch()).as_secs_f64()
}

impl Scoreboard {
    /// A scoreboard for lanes with `slots` workers each (0: the lane is absent).
    pub fn new(slots: [usize; Lane::COUNT]) -> Self {
        let _ = epoch();
        let mut score = Score::default();
        for (u, s) in score.units.iter_mut().zip(slots) {
            u.slots = s;
        }
        Scoreboard { score: Mutex::new(score) }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Score) -> R) -> R {
        let mut s = self.score.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut s)
    }

    /// A launch planned on `lane` and not yet claimed (decision 0100 §11): its estimate counts as outstanding on
    /// the lane, so a batch begun meanwhile sees it; `n` launches of `est` seconds in all, added or (negative)
    /// released when a lane claims them.
    pub fn plan(&self, lane: Lane, n: i64, est: f64) {
        self.with(|s| {
            let u = &mut s.units[lane.index()];
            u.planned = (u.planned as i64 + n).max(0) as usize;
            u.outstanding = (u.outstanding + est).max(0.0);
        });
    }

    /// A job a lane claimed from a batch begun at `queued`: queued since then, started at once by the caller.
    pub fn claimed(&self, lane: Lane, items: Vec<Item>, queued: Instant) -> Ticket {
        let mut t = self.queue(lane, items);
        t.queued = queued;
        t
    }

    /// A job queued on `lane`.
    pub fn queue(&self, lane: Lane, items: Vec<Item>) -> Ticket {
        let est: f64 = items.iter().map(|i| i.estimate).sum();
        self.with(|s| {
            let u = &mut s.units[lane.index()];
            u.queued += 1;
            u.outstanding += est;
        });
        Ticket { id: NEXT.fetch_add(1, Ordering::Relaxed), lane, items, queued: Instant::now(), started: None, share0: 0.0, cpu: 0.0, cpu0: None, phases: Vec::new() }
    }

    /// A worker took the job.
    pub fn start(&self, t: &mut Ticket) {
        let now = Instant::now();
        t.started = Some(now);
        t.resume();
        self.with(|s| {
            let u = &mut s.units[t.lane.index()];
            let at = since(now);
            u.advance(at);
            t.share0 = u.share_clock;
            u.queued -= 1;
            u.running += 1;
            u.peak = u.peak.max(u.running);
            u.first = Some(u.first.map_or(at, |f| f.min(at)));
        });
    }

    /// The job ended, well or not: its seconds run, and its CPU seconds on every thread it ran on.
    pub fn finish(&self, mut t: Ticket, ok: bool) -> (f64, f64) {
        let end = Instant::now();
        let mut share = 0.0;
        t.pause();
        let cpu = t.cpu;
        let phases = std::mem::take(&mut t.phases);
        let start = t.started.unwrap_or(end);
        let ran = end.duration_since(start).as_secs_f64();
        let waited = start.saturating_duration_since(t.queued).as_secs_f64();
        let est: f64 = t.items.iter().map(|i| i.estimate).sum();
        let host_est: f64 = t.items.iter().map(|i| i.host).sum();
        self.with(|s| {
            let u = &mut s.units[t.lane.index()];
            u.advance(since(end));
            share = u.share(t.share0, u.share_clock, ran);
            u.running -= 1;
            u.outstanding = (u.outstanding - est).max(0.0);
            u.jobs += 1;
            u.launches += t.items.len() as u64;
            u.failed += u64::from(!ok);
            // A device lane's busy seconds are its jobs' shares: they sum to the time it had a job running.
            u.busy += share;
            u.cpu += cpu;
            u.waited += waited;
            u.estimated += est;
            u.last = u.last.max(since(end));
            for i in &t.items {
                let r = s.forms.entry((t.lane, i.form)).or_default();
                r.launches += 1;
                r.units += i.units;
                r.estimated += i.estimate;
                r.actual += if est > 0.0 { share * i.estimate / est } else { share / t.items.len().max(1) as f64 };
                r.host_estimated += i.host;
                r.host += if host_est > 0.0 { cpu * i.host / host_est } else { cpu / t.items.len().max(1) as f64 };
            }
            // The lane's latency, from a device's own timeline; a device job's thread CPU.
            // A cold job (a pipeline compiled, a pool buffer made) is not what the lane costs warm: it stays out of
            // the averages.
            let cold = phases.iter().any(|p| matches!(p.0, "pipeline compile" | "pipeline wait" | "new buffer"));
            let lat: f64 = phases.iter().filter(|p| matches!(p.0, "gpu submit" | "gpu schedule" | "gpu wake")).map(|p| p.3).sum();
            let u = &mut s.units[t.lane.index()];
            if lat > 0.0 && !cold {
                u.latency = Some(u.latency.map_or(lat, |a| a + AVG * (lat - a)));
            }
            if t.lane != Lane::Cpu && ok && !cold {
                u.job_cpu = Some(u.job_cpu.map_or(cpu, |a| a + AVG * (cpu - a)));
            }
            if ok && !cold && host_est > 0.0 && t.items.iter().all(|i| i.units > 0.0 && !crate::price::host_learned(i.form, t.lane)) {
                let r = (cpu / host_est).clamp(0.05, 50.0);
                u.host_ratio = Some(u.host_ratio.map_or(r, |a| a + AVG * (r - a)));
            }
            for &(name, n, c, w) in &phases {
                let p = s.phases.entry((t.lane, name)).or_default();
                p.count += n;
                p.cpu += c;
                p.wall += w;
            }
        });
        // A job with no units (a stream's: `strata`, the whole strata loop) prices nothing, and its wall would be
        // fitted as its lane's submission: not kept.
        let units: f64 = t.items.iter().map(|i| i.units).sum();
        let cold = phases.iter().any(|p| matches!(p.0, "pipeline compile" | "pipeline wait" | "new buffer"));
        // The run prices its later launches from this one (`price::observe`): one kernel name's jobs only.
        if units > 0.0 && t.items.windows(2).all(|w| w[0].form == w[1].form) && let Some(first) = t.items.first() {
            crate::price::observe(crate::price::Measured {
                lane: t.lane,
                forms: first.form,
                launches: t.items.len() as u32,
                units,
                wall: share,
                cpu,
                load: crate::exec::cores_slowdown(),
                cold,
                ok,
            });
        }
        let kept = samples();
        if kept.keep.load(Ordering::Relaxed) && units > 0.0 {
            let sample = Sample {
                lane: t.lane,
                forms: t.items.iter().map(|i| i.form).collect::<Vec<_>>().join(","),
                launches: t.items.len() as u32,
                units,
                estimate: est,
                host_estimate: host_est,
                wall: share,
                cpu,
                waited,
                phases: phases.iter().map(|&(name, _, c, w)| format!("{name}={:.1}/{:.1}", c * 1e6, w * 1e6)).collect::<Vec<_>>().join(","),
                cold,
                ok,
                load: crate::exec::cores_slowdown(),
                threads: crate::exec::threads() as u32,
            };
            let mut held = kept.held.lock().unwrap_or_else(|p| p.into_inner());
            if held.len() < HELD {
                held.push(sample);
            } else {
                kept.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        if let Some(f) = trace() {
            let mut forms: Vec<&str> = t.items.iter().map(|i| i.form).collect();
            forms.dedup();
            let thread = std::thread::current().name().unwrap_or("?").to_owned();
            let ph: Vec<String> = phases.iter().map(|&(name, _, c, w)| format!("{name}={:.1}/{:.1}", c * 1e6, w * 1e6)).collect();
            let line = format!(
                "{}\t{}\t{thread}\t{}\t{}\t{units:.0}\t{est:.6}\t{:.6}\t{:.6}\t{:.6}\t{}\t{:.6}\t{}\n",
                t.id,
                t.lane.name(),
                t.items.len(),
                forms.join(","),
                since(t.queued),
                since(start),
                since(end),
                u8::from(ok),
                cpu,
                ph.join(",")
            );
            let _ = f.lock().unwrap_or_else(|p| p.into_inner()).write_all(line.as_bytes());
        }
        (ran, cpu)
    }

    /// A batch of kind `tag` placed: per launch its lane, its cost there and the fastest lane's, and the slack used.
    pub fn placed(&self, tag: &'static str, placed: &[(Lane, crate::price::Cost, crate::price::Cost)], slack: f64) {
        self.with(|s| {
            let t = s.tags.entry(tag).or_default();
            t.batches += 1;
            t.slack += slack;
            for (lane, c, fastest) in placed {
                t.launches[lane.index()] += 1;
                t.host += c.host;
                t.host_fastest += fastest.host;
                if c.finish > fastest.finish {
                    t.not_fastest += 1;
                    t.later += c.finish - fastest.finish;
                }
            }
        });
    }

    /// A batch of kind `tag` collected: its caller worked `overlap` seconds before it waited, then stalled `stall`;
    /// its launches ran on `ran` per lane, `moved` of them off their planned lane and `helped` by the caller.
    pub fn collected(&self, tag: &'static str, overlap: f64, stall: f64, ran: [u64; Lane::COUNT], moved: u64, helped: u64) {
        self.with(|s| {
            let t = s.tags.entry(tag).or_default();
            for (a, b) in t.ran.iter_mut().zip(ran) {
                *a += b;
            }
            t.moved += moved;
            t.helped += helped;
            t.overlap += overlap;
            t.stall += stall;
            t.max_stall = t.max_stall.max(stall);
            t.stalled += u64::from(stall > 1e-4);
            if t.recent.len() == 32 {
                t.recent.pop_front();
            }
            t.recent.push_back(overlap);
        });
        if let Some(f) = trace() {
            let line = format!(
                "# batch\t{tag}\toverlap_us={:.1}\tstall_us={:.1}\tran={}\tmoved={moved}\thelped={helped}\n",
                overlap * 1e6,
                stall * 1e6,
                Lane::ALL.iter().filter(|l| ran[l.index()] > 0).map(|l| format!("{}:{}", l.name(), ran[l.index()])).collect::<Vec<_>>().join(",")
            );
            let _ = f.lock().unwrap_or_else(|p| p.into_inner()).write_all(line.as_bytes());
        }
    }

    /// The slack a batch of kind `tag` has: how long its callers worked before they waited (the lower quartile of
    /// the recent batches).
    pub fn slack(&self, tag: &'static str) -> f64 {
        self.with(|s| s.tags.get(tag).map_or(0.0, TagRow::slack))
    }

    /// The lanes' measured latency, device job CPU, and host CPU over its estimate, where measured.
    pub fn measured(&self) -> [(Option<f64>, Option<f64>, Option<f64>); Lane::COUNT] {
        self.with(|s| std::array::from_fn(|i| (s.units[i].latency, s.units[i].job_cpu, s.units[i].host_ratio)))
    }

    /// Adds a phase the caller ran outside any job (its host work around a batch) to `lane`'s phases.
    pub fn add_phase(&self, lane: Lane, name: &'static str, cpu: f64, wall: f64) {
        self.with(|s| {
            let p = s.phases.entry((lane, name)).or_default();
            p.count += 1;
            p.cpu += cpu;
            p.wall += wall;
        });
    }

    /// `f` of `lane`'s unit now (its jobs queued and running), read under the board's lock.
    pub fn with_units<R>(&self, lane: Lane, f: impl FnOnce(&Unit) -> R) -> R {
        self.with(|s| f(&s.units[lane.index()]))
    }

    /// The estimated seconds not yet done, per lane.
    pub fn outstanding(&self) -> [f64; Lane::COUNT] {
        self.with(|s| std::array::from_fn(|i| s.units[i].outstanding))
    }

    /// The scoreboard now.
    pub fn score(&self) -> Score {
        self.with(|s| {
            let mut s = s.clone();
            s.elapsed = epoch().elapsed().as_secs_f64();
            s
        })
    }

    /// Appends `title` and the scoreboard to the trace, each line after `#`, when there is a trace.
    pub fn note(&self, title: &str) {
        let Some(f) = trace() else { return };
        let mut text = format!("# {title}\n");
        for line in self.score().to_string().lines() {
            text.push_str("# ");
            text.push_str(line);
            text.push('\n');
        }
        let _ = f.lock().unwrap_or_else(|p| p.into_inner()).write_all(text.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A job's estimate is outstanding from its queueing to its end, and its counts land on its lane and form.
    #[test]
    fn a_job_is_outstanding_until_it_ends_and_counted_on_its_lane() {
        let b = Scoreboard::new([2, 0, 1, 0, 0]);
        let mut t = b.queue(Lane::Gpu, vec![Item { form: "probe", units: 10.0, estimate: 0.5, host: 0.0 }, Item { form: "tile", units: 4.0, estimate: 1.5, host: 0.0 }]);
        assert_eq!(b.outstanding(), [0.0, 0.0, 2.0, 0.0, 0.0]);
        assert_eq!(b.score().units[Lane::Gpu.index()].queued, 1);
        b.start(&mut t);
        assert_eq!((b.score().units[Lane::Gpu.index()].running, b.score().units[Lane::Gpu.index()].peak), (1, 1));
        let (ran, _) = b.finish(t, true);
        let s = b.score();
        let u = &s.units[Lane::Gpu.index()];
        assert_eq!((u.queued, u.running, u.jobs, u.launches, u.failed), (0, 0, 1, 2, 0));
        assert_eq!(b.outstanding(), [0.0; Lane::COUNT]);
        assert!((u.estimated - 2.0).abs() < 1e-12);
        // The submission's time shared by the estimates: a quarter to the probe.
        assert!((s.forms[&(Lane::Gpu, "probe")].actual - ran / 4.0).abs() < 1e-12);
        assert!(s.to_string().contains("gpu"));
    }

    /// Two jobs side by side on a device lane share its time: each is charged the seconds it ran over the jobs
    /// running then, so their shares sum to the time the lane had a job running, not to both jobs' walls.
    #[test]
    fn jobs_side_by_side_on_a_device_lane_share_its_time() {
        let b = Scoreboard::new([2, 0, 1, 0, 0]);
        let item = || vec![Item { form: "wave", units: 1.0, estimate: 0.0, host: 0.0 }];
        let pause = || std::thread::sleep(std::time::Duration::from_millis(30));
        let (mut first, mut second) = (b.queue(Lane::Gpu, item()), b.queue(Lane::Gpu, item()));
        let begun = Instant::now();
        b.start(&mut first);
        pause();
        let before_second = Instant::now();
        b.start(&mut second);
        let after_second = Instant::now();
        pause();
        let before_first = Instant::now();
        let (ran_first, _) = b.finish(first, true);
        let after_first = Instant::now();
        pause();
        let (ran_second, _) = b.finish(second, true);
        let span = begun.elapsed().as_secs_f64();
        let busy = b.score().units[Lane::Gpu.index()].busy;
        // Each ran about 60 ms; together the lane was busy about 90. The walls overlap by the time both ran: from the
        // second's start to the first's finish, bounded by the clock read on either side of each (a stall of the
        // machine anywhere stretches the span and the walls alike, never the overlap's bounds).
        let overlap = ran_first + ran_second - span;
        let (least, most) = ((before_first - after_second).as_secs_f64(), (after_first - before_second).as_secs_f64());
        assert!(least > 0.025, "the jobs ran side by side for {least} s");
        assert!(overlap > least - 0.002 && overlap < most + 0.002, "the walls overlap by {overlap}, not within {least}..{most}");
        assert!((busy - span).abs() < 0.005, "shares {busy} against the lane's span {span}");
        // The cores run jobs on threads of their own: each is charged its wall.
        let (mut a, mut c) = (b.queue(Lane::Cpu, item()), b.queue(Lane::Cpu, item()));
        b.start(&mut a);
        b.start(&mut c);
        pause();
        let (ra, _) = b.finish(a, true);
        let (rc, _) = b.finish(c, true);
        assert!((b.score().units[Lane::Cpu.index()].busy - (ra + rc)).abs() < 1e-9);
    }

    /// A device job begins on its lane's thread and ends on a pool thread: its CPU is the work it did on each, never
    /// the difference of two threads' clocks, and its phases are its own, wherever they were counted.
    #[test]
    fn a_job_handed_between_threads_counts_only_its_own_cpu() {
        fn spin(s: f64) {
            let c0 = thread_cpu();
            while thread_cpu() - c0 < s {
                std::hint::black_box(0u64);
            }
        }
        let b = Scoreboard::new([1, 0, 1, 0, 0]);
        // The finishing thread has run long before the job: under one clock read at each end, that would be counted.
        let (send, take) = std::sync::mpsc::channel::<Ticket>();
        let finisher = std::thread::spawn(move || {
            spin(0.2);
            add("before", 0.2, 0.2);
            let mut t = take.recv().unwrap();
            t.resume();
            spin(0.01);
            add("read", 0.01, 0.01);
            t.pause();
            t
        });
        let mut t = b.queue(Lane::Gpu, vec![Item { form: "probe", units: 1.0, estimate: 0.01, host: 0.0 }]);
        b.start(&mut t);
        spin(0.01);
        add("encode", 0.01, 0.01);
        t.pause();
        send.send(t).unwrap();
        let t = finisher.join().unwrap();
        b.finish(t, true);
        let s = b.score();
        let cpu = s.units[Lane::Gpu.index()].cpu;
        assert!((0.019..0.1).contains(&cpu), "the job's CPU was {cpu} s");
        let names: Vec<_> = s.phases.keys().filter(|k| k.0 == Lane::Gpu).map(|k| k.1).collect();
        assert!(names.contains(&"encode") && names.contains(&"read") && !names.contains(&"before"), "{names:?}");
        // Resumed on one thread and ended on another: the other's clock is not read.
        let mut t = b.queue(Lane::Gpu, vec![Item { form: "probe", units: 1.0, estimate: 0.01, host: 0.0 }]);
        b.start(&mut t);
        let t = std::thread::spawn(move || {
            spin(0.2);
            t
        })
        .join()
        .unwrap();
        b.finish(t, true);
        let more = b.score().units[Lane::Gpu.index()].cpu - cpu;
        assert!((0.0..0.1).contains(&more), "the second job's CPU was {more} s");
    }
}
