//! The executor: a batch of launches placed on lanes, each lane's share submitted once, the lanes run together.
//!
//! A [`Batch`] is a wave: launches that do not read each other's outputs. The caller orders waves (a level of the
//! dependency order, decision 0095 §5). Within a batch:
//!
//! 1. **Placement.** Each launch, largest first, goes to the lane [`Prices::choose`] picks: the least energy among
//!    the lanes that would finish within the parity band of the first to finish, counting what the batch already
//!    queued on each lane. A launch may be pinned to a lane.
//! 2. **Submission.** Each lane gets its launches as **one submission** ([`Backend::submit`]): one command buffer on
//!    the GPU, one streaming session per SME worker, batched predictions per model on the Neural Engine, one
//!    parallel pass on the cores. That is what batching launches buys: a lane's fixed cost once per wave, not once
//!    per launch.
//! 3. **Concurrency.** The lanes' submissions run at the same time, each on its lane's own worker thread (one per
//!    lane, kept for the executor's life). [`Executor::begin`] returns as soon as every lane has its share, so the
//!    caller works while the lanes do; [`Running::wait`] collects the outputs (decision 0100 §3).
//!
//! There is no fallback. A launch no present lane can run is an error that names it, and so is a lane's failure.

use crate::kernel::{Kernel, Output};
use crate::lanes::{self, Lane};
use crate::price::Prices;
use std::sync::Arc;
use std::time::Instant;

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

/// Independent launches: one wave.
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
}

/// What a batch did on each lane: for the profile's per-lane report (decision 0098 §2).
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Per lane: launches, submissions, wall seconds of its submission.
    pub launches: [usize; 4],
    pub wall: [f64; 4],
    /// Estimated joules per lane (the price's power times the lane's wall time).
    pub joules: [f64; 4],
    /// Estimated seconds the off-CPU lanes took from the CPU lane: what the CPU would have spent on their launches.
    pub cpu_freed: f64,
    /// Estimated seconds lost against the fastest lane, summed over launches placed elsewhere.
    pub lost: f64,
    /// The process's CPU time (user + sys) over the batch.
    pub cpu: f64,
}

/// The lanes this process has, their prices, and the submission of batches.
pub struct Executor {
    backends: Vec<Arc<dyn Backend>>,
    /// Each backend's worker: the thread its submissions run on, in the order they were sent.
    workers: Vec<std::sync::mpsc::Sender<Task>>,
    pub prices: Prices,
}

/// A lane's share of a batch, and where its outputs go: the outputs and the submission's wall seconds.
type Task = (Vec<Kernel>, std::sync::mpsc::SyncSender<(Result<Vec<Output>, String>, f64)>);

/// The worker of `backend`: runs each task it is sent, in order, until the executor drops.
fn worker(backend: Arc<dyn Backend>) -> std::sync::mpsc::Sender<Task> {
    let (send, take) = std::sync::mpsc::channel::<Task>();
    std::thread::Builder::new()
        .name(format!("rerail-accel {}", backend.lane().name()))
        .spawn(move || {
            for (ks, reply) in take {
                let t = Instant::now();
                let refs: Vec<&Kernel> = ks.iter().collect();
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| backend.submit(&refs))).unwrap_or_else(|_| Err("its submission panicked".into()));
                let _ = reply.send((r, t.elapsed().as_secs_f64()));
            }
        })
        .expect("a lane's worker thread");
    send
}

/// A batch the lanes are running ([`Executor::begin`]).
pub struct Running {
    placed: Vec<(Lane, f64, f64)>,
    per_lane: [Vec<usize>; 4],
    replies: Vec<(Lane, std::sync::mpsc::Receiver<(Result<Vec<Output>, String>, f64)>)>,
    report: Report,
    joules: [f64; 4],
    cpu0: f64,
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
        let workers = backends.iter().map(|b| worker(Arc::clone(b))).collect();
        Executor { backends, workers, prices: Prices::default() }
    }

    pub fn lanes(&self) -> Vec<Lane> {
        self.backends.iter().map(|b| b.lane()).collect()
    }

    fn backend(&self, l: Lane) -> Option<&Arc<dyn Backend>> {
        self.backends.iter().find(|b| b.lane() == l)
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

    /// Places each launch: `(lane, estimate, fastest)` per launch, in the batch's order.
    pub fn place(&self, batch: &Batch) -> Result<Vec<(Lane, f64, f64)>, String> {
        let mut order: Vec<usize> = (0..batch.0.len()).collect();
        order.sort_by(|&a, &b| batch.0[b].kernel.units().total_cmp(&batch.0[a].kernel.units()));
        let mut queued = [0.0f64; 4];
        let mut placed = vec![(Lane::Cpu, 0.0, 0.0); batch.0.len()];
        for i in order {
            let l = &batch.0[i];
            let able: Vec<Lane> = self.backends.iter().filter(|b| l.pin.is_none_or(|p| p == b.lane()) && b.can(&l.kernel)).map(|b| b.lane()).collect();
            let fastest = able.iter().filter_map(|&x| self.prices.wall(&l.kernel, x)).fold(f64::INFINITY, f64::min);
            let (lane, est) = self.prices.choose(&l.kernel, &able, &queued).ok_or_else(|| {
                format!("no lane can run launch {i} ({}, {:.3e} units){}", l.kernel.form().name(), l.kernel.units(), l.pin.map_or(String::new(), |p| format!(" on {}", p.name())))
            })?;
            let q = &mut queued[lane.index()];
            if *q == 0.0 {
                *q += self.prices.fixed[lane.index()].submission;
            }
            *q += est;
            placed[i] = (lane, est, fastest);
        }
        Ok(placed)
    }

    /// Runs a batch: placed, one submission per lane, the lanes together; returns when every lane is done.
    pub fn run(&self, batch: Batch) -> Result<(Vec<Done>, Report), String> {
        self.begin(batch)?.wait()
    }

    /// Places a batch and gives each lane its share, without waiting: the lanes run while the caller works.
    pub fn begin(&self, batch: Batch) -> Result<Running, String> {
        let cpu0 = process_cpu();
        let placed = self.place(&batch)?;
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
        let mut kernels: Vec<Option<Kernel>> = batch.0.into_iter().map(|l| Some(l.kernel)).collect();
        let mut replies = Vec::new();
        for lane in Lane::ALL {
            let ids = &per_lane[lane.index()];
            if ids.is_empty() {
                continue;
            }
            let at = self.backends.iter().position(|b| b.lane() == lane).expect("placed on a present lane");
            let ks: Vec<Kernel> = ids.iter().map(|&i| kernels[i].take().expect("a launch placed once")).collect();
            let (reply, take) = std::sync::mpsc::sync_channel(1);
            self.workers[at].send((ks, reply)).map_err(|_| format!("{} lane: its worker ended", lane.name()))?;
            replies.push((lane, take));
        }
        let mut joules = [0.0; 4];
        for lane in Lane::ALL {
            joules[lane.index()] = self.prices.joules(lane, 1.0);
        }
        Ok(Running { placed, per_lane, replies, report, joules, cpu0 })
    }
}

impl Running {
    /// Waits for every lane's share: each launch's output, in the batch's order, and the batch's report.
    pub fn wait(self) -> Result<(Vec<Done>, Report), String> {
        let Running { placed, per_lane, replies, mut report, joules, cpu0 } = self;
        let mut outs: Vec<Option<Output>> = vec![None; placed.len()];
        let mut failed: Option<String> = None;
        for (lane, take) in replies {
            let (r, wall) = take.recv().map_err(|_| format!("{} lane: its worker ended", lane.name()))?;
            let r = match r {
                Ok(r) => r,
                Err(e) => {
                    failed.get_or_insert(format!("{} lane: {e}", lane.name()));
                    continue;
                }
            };
            let ids = &per_lane[lane.index()];
            if r.len() != ids.len() {
                failed.get_or_insert(format!("{} lane: {} outputs for {} launches", lane.name(), r.len(), ids.len()));
                continue;
            }
            report.wall[lane.index()] = wall;
            report.joules[lane.index()] = joules[lane.index()] * wall;
            for (&i, o) in ids.iter().zip(r) {
                outs[i] = Some(o);
            }
        }
        if let Some(e) = failed {
            return Err(e);
        }
        report.cpu = process_cpu() - cpu0;
        let done = outs
            .into_iter()
            .zip(placed)
            .map(|(o, (lane, estimate, fastest))| Done { out: o.expect("every launch ran"), lane, estimate, fastest })
            .collect();
        Ok((done, report))
    }
}

impl Default for Executor {
    fn default() -> Self {
        Executor::new()
    }
}

/// The CPU lane: the reference kernels on the rayon pool, the batch's launches in one parallel pass. Its space keeps
/// a join's right side sorted by key 0, by the join's id.
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
        let Some(id) = id else { return Ok(Arc::new(crate::rows::Keyed::new(&right[0]))) };
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
        let Kernel::Join { left, right, right_id, .. } = k else { return Ok(crate::cpu::run(k)) };
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
