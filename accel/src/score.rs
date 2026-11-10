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
//! - the price's estimate against the time taken, summed, per lane and per form.
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
    /// The estimate of the jobs queued or running now.
    pub outstanding: f64,
    /// When the lane's first job started and its last ended (seconds from the epoch).
    pub first: Option<f64>,
    pub last: f64,
}

impl Unit {
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
    pub units: [Unit; 4],
    /// By (lane, form).
    pub forms: BTreeMap<(Lane, &'static str), FormRow>,
    /// By (lane, phase).
    pub phases: BTreeMap<(Lane, &'static str), PhaseRow>,
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
        writeln!(f, "lane  form                  launches  units       estimate s  actual s")?;
        for ((l, form), r) in &self.forms {
            writeln!(f, "{:<4}  {:<20}  {:>8}  {:>10.3e}  {:>10.4}  {:>8.4}", l.name(), form, r.launches, r.units, r.estimated, r.actual)?;
        }
        if !self.phases.is_empty() {
            writeln!(f, "lane  phase                 count     cpu ms   wall ms  cpu us each")?;
            for ((l, name), p) in &self.phases {
                writeln!(f, "{:<4}  {:<20}  {:>6}  {:>9.3}  {:>8.3}  {:>11.1}", l.name(), name, p.count, p.cpu * 1e3, p.wall * 1e3, p.cpu * 1e6 / p.count.max(1) as f64)?;
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

/// A launch of a job: its form's name, units and estimate.
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub form: &'static str,
    pub units: f64,
    pub estimate: f64,
}

/// A job between its queueing and its end.
pub struct Ticket {
    id: u64,
    lane: Lane,
    items: Vec<Item>,
    queued: Instant,
    started: Option<Instant>,
    cpu0: f64,
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

/// The trace's file, opened once, its header written (`RERAIL_SCOREBOARD`).
fn trace() -> Option<&'static Mutex<std::fs::File>> {
    static TRACE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    TRACE
        .get_or_init(|| {
            let path = std::env::var("RERAIL_SCOREBOARD").ok().filter(|p| !p.is_empty())?;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
            let _ = writeln!(f, "job\tlane\tthread\tlaunches\tforms\tunits\testimate\tqueued\tstart\tend\tok\tcpu\tphases");
            Some(Mutex::new(f))
        })
        .as_ref()
}

fn since(t: Instant) -> f64 {
    t.saturating_duration_since(epoch()).as_secs_f64()
}

impl Scoreboard {
    /// A scoreboard for lanes with `slots` workers each (0: the lane is absent).
    pub fn new(slots: [usize; 4]) -> Self {
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

    /// A job queued on `lane`.
    pub fn queue(&self, lane: Lane, items: Vec<Item>) -> Ticket {
        let est: f64 = items.iter().map(|i| i.estimate).sum();
        self.with(|s| {
            let u = &mut s.units[lane.index()];
            u.queued += 1;
            u.outstanding += est;
        });
        Ticket { id: NEXT.fetch_add(1, Ordering::Relaxed), lane, items, queued: Instant::now(), started: None, cpu0: 0.0 }
    }

    /// A worker took the job.
    pub fn start(&self, t: &mut Ticket) {
        take_phases();
        let now = Instant::now();
        t.started = Some(now);
        t.cpu0 = thread_cpu();
        self.with(|s| {
            let u = &mut s.units[t.lane.index()];
            u.queued -= 1;
            u.running += 1;
            u.peak = u.peak.max(u.running);
            let at = since(now);
            u.first = Some(u.first.map_or(at, |f| f.min(at)));
        });
    }

    /// The job ended, well or not: its seconds run.
    pub fn finish(&self, t: Ticket, ok: bool) -> f64 {
        let end = Instant::now();
        let cpu = thread_cpu() - t.cpu0;
        let phases = take_phases();
        let start = t.started.unwrap_or(end);
        let ran = end.duration_since(start).as_secs_f64();
        let waited = start.saturating_duration_since(t.queued).as_secs_f64();
        let est: f64 = t.items.iter().map(|i| i.estimate).sum();
        self.with(|s| {
            let u = &mut s.units[t.lane.index()];
            u.running -= 1;
            u.outstanding = (u.outstanding - est).max(0.0);
            u.jobs += 1;
            u.launches += t.items.len() as u64;
            u.failed += u64::from(!ok);
            u.busy += ran;
            u.cpu += cpu;
            u.waited += waited;
            u.estimated += est;
            u.last = u.last.max(since(end));
            for i in &t.items {
                let r = s.forms.entry((t.lane, i.form)).or_default();
                r.launches += 1;
                r.units += i.units;
                r.estimated += i.estimate;
                r.actual += if est > 0.0 { ran * i.estimate / est } else { ran / t.items.len().max(1) as f64 };
            }
            for &(name, n, c, w) in &phases {
                let p = s.phases.entry((t.lane, name)).or_default();
                p.count += n;
                p.cpu += c;
                p.wall += w;
            }
        });
        if let Some(f) = trace() {
            let mut forms: Vec<&str> = t.items.iter().map(|i| i.form).collect();
            forms.dedup();
            let units: f64 = t.items.iter().map(|i| i.units).sum();
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
        ran
    }

    /// The estimated seconds not yet done, per lane.
    pub fn outstanding(&self) -> [f64; 4] {
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
        let b = Scoreboard::new([2, 0, 1, 0]);
        let mut t = b.queue(Lane::Gpu, vec![Item { form: "probe", units: 10.0, estimate: 0.5 }, Item { form: "tile", units: 4.0, estimate: 1.5 }]);
        assert_eq!(b.outstanding(), [0.0, 0.0, 2.0, 0.0]);
        assert_eq!(b.score().units[Lane::Gpu.index()].queued, 1);
        b.start(&mut t);
        assert_eq!((b.score().units[Lane::Gpu.index()].running, b.score().units[Lane::Gpu.index()].peak), (1, 1));
        let ran = b.finish(t, true);
        let s = b.score();
        let u = &s.units[Lane::Gpu.index()];
        assert_eq!((u.queued, u.running, u.jobs, u.launches, u.failed), (0, 0, 1, 2, 0));
        assert_eq!(b.outstanding(), [0.0; 4]);
        assert!((u.estimated - 2.0).abs() < 1e-12);
        // The submission's time shared by the estimates: a quarter to the probe.
        assert!((s.forms[&(Lane::Gpu, "probe")].actual - ran / 4.0).abs() < 1e-12);
        assert!(s.to_string().contains("gpu"));
    }
}
