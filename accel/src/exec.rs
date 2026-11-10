//! The executor: every job of the process's relational work, run on every lane at once, and scored.
//!
//! **Jobs.** A [`Batch`] is a level's launches: launches that do not read each other's outputs (decision 0095 §5).
//! The executor runs them as jobs on the lanes' workers:
//!
//! - **The cores** have a pool of [`threads`] workers (`RERAIL_THREADS`, 4 by default), one pool for the process,
//!   shared by every executor. Each launch placed on the cores is a job of its own, so a batch's launches on the
//!   cores run on all the pool's threads at once, beside every other batch's. A kernel that splits its work (the
//!   reference's parallel loops) splits it over the same threads, not over the machine's.
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
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

/// The cores' workers: `RERAIL_THREADS`, a whole number of at least 1; 4 when unset.
///
/// The variable is the engine's thread setting (the most threads the process uses). The executor's pool takes its
/// number, so the cores' jobs never use more threads than it allows.
pub fn threads() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| std::env::var("RERAIL_THREADS").ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|&n| n >= 1).unwrap_or(4))
}

/// The cores' pool: [`threads`] workers, made once for the process.
pub fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads())
            .thread_name(|i| format!("rerail-cpu {i}"))
            .build()
            .expect("the executor's thread pool")
    })
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

/// A lane's implementation.
pub trait Backend: Send + Sync {
    fn lane(&self) -> Lane;
    /// Whether this lane can run `k`, its shape's limits included (the Neural Engine's exact range, a constant it
    /// needs fixed, a threadgroup's memory).
    fn can(&self, k: &Kernel) -> bool;
    /// Runs `ks` as one submission. The outputs are in `ks`' order, each equal to [`crate::cpu::run`]'s.
    fn submit(&self, ks: &[&Kernel]) -> Result<Vec<Output>, String>;
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
    /// The process's CPU time (user + sys) over the batch.
    pub cpu: f64,
}

/// A job's outputs, and when it started and ended.
type Reply = (Result<Vec<Output>, String>, Instant, Instant);

/// A device lane's job: its share of a batch, its ticket, and where its outputs go.
type Task = (Vec<Kernel>, Ticket, SyncSender<Reply>);

/// The lanes this process has, their prices, their workers and their scoreboard.
pub struct Executor {
    backends: Vec<Arc<dyn Backend>>,
    /// Per backend: its worker thread's queue, for a device lane; `None` for the cores (their jobs go to the pool).
    workers: Vec<Option<std::sync::mpsc::Sender<Task>>>,
    board: Arc<Scoreboard>,
    pub prices: Prices,
}

/// Runs `ks` on `backend` as one job: started and finished on the scoreboard, a panic made an error.
fn run_job(backend: &dyn Backend, board: &Scoreboard, ks: &[Kernel], mut ticket: Ticket) -> Reply {
    board.start(&mut ticket);
    let start = Instant::now();
    let refs: Vec<&Kernel> = ks.iter().collect();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| backend.submit(&refs))).unwrap_or_else(|_| Err("its submission panicked".into()));
    board.finish(ticket, r.is_ok());
    (r, start, Instant::now())
}

/// The worker of a device lane: runs each job it is sent, in order, until the executor drops.
fn worker(backend: Arc<dyn Backend>, board: Arc<Scoreboard>) -> std::sync::mpsc::Sender<Task> {
    let (send, take) = std::sync::mpsc::channel::<Task>();
    std::thread::Builder::new()
        .name(format!("rerail-accel {}", backend.lane().name()))
        .spawn(move || {
            for (ks, ticket, reply) in take {
                let _ = reply.send(run_job(&*backend, &board, &ks, ticket));
            }
        })
        .expect("a lane's worker thread");
    send
}

/// A batch the lanes are running ([`Executor::begin`]).
pub struct Running {
    placed: Vec<(Lane, f64, f64)>,
    /// Each launch's host CPU estimate.
    hosts: Vec<f64>,
    tag: &'static str,
    begun: Instant,
    board: Arc<Scoreboard>,
    /// Per job: its lane, its launches (indices into the batch), and its reply.
    replies: Vec<(Lane, Vec<usize>, Receiver<Reply>)>,
    report: Report,
    cpu0: f64,
}

/// A host job ([`Executor::spawn`]) the pool is running.
pub struct Job<R> {
    name: &'static str,
    take: Receiver<std::thread::Result<R>>,
}

impl<R> Job<R> {
    /// Waits for the job: its result, or an error when it panicked. On one of the pool's own threads, the wait runs
    /// the pool's other jobs meanwhile, so jobs that wait for jobs cannot take every thread and stall.
    pub fn wait(self) -> Result<R, String> {
        let got = if pool().current_thread_index().is_some() {
            loop {
                match self.take.try_recv() {
                    Ok(r) => break Ok(r),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break Err(()),
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        if rayon::yield_now() != Some(rayon::Yield::Executed) {
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
        match got {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(_)) => Err(format!("host job {}: it panicked", self.name)),
            Err(()) => Err(format!("host job {}: its worker ended", self.name)),
        }
    }
}

impl Executor {
    /// Every lane this machine has, with the default prices.
    pub fn new() -> Self {
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
            let (lane, cost, fastest) = prices.choose_host(&l.kernel, &able, &load).ok_or_else(|| {
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

    /// Places a batch of kind `tag` and queues its jobs, without waiting.
    pub fn begin_as(&self, tag: &'static str, batch: Batch) -> Result<Running, String> {
        let cpu0 = process_cpu();
        let begun = Instant::now();
        let (costs, load) = self.place(tag, &batch)?;
        self.board.placed(tag, &costs, load.slack);
        let placed: Vec<(Lane, f64, f64)> = costs.iter().map(|(l, c, f)| (*l, c.wall, f.wall)).collect();
        let mut report = Report::default();
        let mut per_lane: [Vec<usize>; 4] = Default::default();
        for (i, &(lane, est, fastest)) in placed.iter().enumerate() {
            per_lane[lane.index()].push(i);
            report.launches[lane.index()] += 1;
            report.lost += (est - fastest).max(0.0);
            if lane != Lane::Cpu {
                if let Some(c) = self.prices.wall(&batch.0[i].kernel, Lane::Cpu) {
                    report.cpu_freed += c * self.prices.cores;
                }
            }
        }
        // The engine's launch budget, checked on the submitting thread (decision 0100 §2).
        #[cfg(all(feature = "gpu", target_os = "macos"))]
        if !per_lane[Lane::Gpu.index()].is_empty() {
            crate::gpu::shared::guard().map_err(|e| format!("gpu lane: {e}"))?;
        }
        let item = |i: usize, k: &Kernel| Item { form: k.form().name(), units: k.units(), estimate: placed[i].1, host: costs[i].1.host };
        let mut kernels: Vec<Option<Kernel>> = batch.0.into_iter().map(|l| Some(l.kernel)).collect();
        let mut replies = Vec::new();
        for lane in Lane::ALL {
            let ids = &per_lane[lane.index()];
            if ids.is_empty() {
                continue;
            }
            let at = self.backends.iter().position(|b| b.lane() == lane).expect("placed on a present lane");
            match &self.workers[at] {
                // The cores: a job a launch, on the pool.
                None => {
                    for &i in ids {
                        let k = kernels[i].take().expect("a launch placed once");
                        let ticket = self.board.queue(lane, vec![item(i, &k)]);
                        let (reply, take) = std::sync::mpsc::sync_channel(1);
                        let (backend, board) = (Arc::clone(&self.backends[at]), Arc::clone(&self.board));
                        pool().spawn_fifo(move || {
                            let _ = reply.send(run_job(&*backend, &board, std::slice::from_ref(&k), ticket));
                        });
                        replies.push((lane, vec![i], take));
                    }
                }
                // A device lane: its share as one job, one submission.
                Some(w) => {
                    let ks: Vec<Kernel> = ids.iter().map(|&i| kernels[i].take().expect("a launch placed once")).collect();
                    let ticket = self.board.queue(lane, ids.iter().zip(&ks).map(|(&i, k)| item(i, k)).collect());
                    let (reply, take) = std::sync::mpsc::sync_channel(1);
                    w.send((ks, ticket, reply)).map_err(|_| format!("{} lane: its worker ended", lane.name()))?;
                    replies.push((lane, ids.clone(), take));
                }
            }
        }
        Ok(Running { placed, hosts: costs.iter().map(|c| c.1.host).collect(), replies, report, cpu0, tag, begun, board: Arc::clone(&self.board) })
    }

    /// Runs `f` as a job on the cores' pool, scored as host work named `name` (`estimate`: the caller's guess of
    /// its seconds, for the placement of what follows; 0 when unknown).
    pub fn spawn<R: Send + 'static>(&self, name: &'static str, estimate: f64, f: impl FnOnce() -> R + Send + 'static) -> Job<R> {
        let mut ticket = self.board.queue(Lane::Cpu, vec![Item { form: name, units: 0.0, estimate, host: estimate }]);
        let (reply, take) = std::sync::mpsc::sync_channel(1);
        let board = Arc::clone(&self.board);
        pool().spawn_fifo(move || {
            board.start(&mut ticket);
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            board.finish(ticket, r.is_ok());
            let _ = reply.send(r);
        });
        Job { name, take }
    }
}

impl Running {
    /// Waits for every job: each launch's output, in the batch's order, and the batch's report.
    pub fn wait(self) -> Result<(Vec<Done>, Report), String> {
        let Running { placed, hosts, replies, mut report, cpu0, tag, begun, board } = self;
        let asked = Instant::now();
        let mut outs: Vec<Option<Output>> = vec![None; placed.len()];
        let mut failed: Option<String> = None;
        let mut span: [Option<(Instant, Instant)>; 4] = [None; 4];
        let mut ran = [0.0f64; 4];
        for (lane, ids, take) in replies {
            let (r, start, end) = take.recv().map_err(|_| format!("{} lane: its worker ended", lane.name()))?;
            let s = &mut span[lane.index()];
            *s = Some(s.map_or((start, end), |(a, b)| (a.min(start), b.max(end))));
            ran[lane.index()] += end.duration_since(start).as_secs_f64();
            let r = match r {
                Ok(r) => r,
                Err(e) => {
                    failed.get_or_insert(format!("{} lane: {e}", lane.name()));
                    continue;
                }
            };
            if r.len() != ids.len() {
                failed.get_or_insert(format!("{} lane: {} outputs for {} launches", lane.name(), r.len(), ids.len()));
                continue;
            }
            for (&i, o) in ids.iter().zip(r) {
                outs[i] = Some(o);
            }
        }
        if let Some(e) = failed {
            return Err(e);
        }
        for lane in Lane::ALL {
            if let Some((a, b)) = span[lane.index()] {
                report.wall[lane.index()] = b.duration_since(a).as_secs_f64();
            }
            report.joules[lane.index()] = WATTS[lane.index()] * ran[lane.index()];
        }
        report.cpu = process_cpu() - cpu0;
        let done = outs
            .into_iter()
            .zip(placed)
            .zip(hosts)
            .map(|((o, (lane, estimate, fastest)), host)| Done { out: o.expect("every launch ran"), lane, estimate, fastest, host })
            .collect();
        board.collected(tag, asked.duration_since(begun).as_secs_f64(), asked.elapsed().as_secs_f64());
        Ok((done, report))
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
            _ => true,
        }
    }
    fn submit(&self, ks: &[&Kernel]) -> Result<Vec<Output>, String> {
        use rayon::prelude::*;
        ks.par_iter().map(|k| self.join(k)).collect()
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
