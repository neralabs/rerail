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
//! 3. **Concurrency.** The lanes' submissions run at the same time, each on its own thread; the batch ends when every
//!    lane is done.
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

/// What a batch did on each lane: for the profile's per-lane report (decision 0096 §2).
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
    pub prices: Prices,
}

impl Executor {
    /// Every lane this machine has, with the default prices.
    pub fn new() -> Self {
        let present = lanes::present();
        let mut backends: Vec<Arc<dyn Backend>> = vec![Arc::new(CpuBackend)];
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
        Executor { backends, prices: Prices::default() }
    }

    /// Only the given lanes' backends (tests; a binding that allows only some lanes).
    pub fn with(backends: Vec<Arc<dyn Backend>>) -> Self {
        Executor { backends, prices: Prices::default() }
    }

    pub fn lanes(&self) -> Vec<Lane> {
        self.backends.iter().map(|b| b.lane()).collect()
    }

    fn backend(&self, l: Lane) -> Option<&Arc<dyn Backend>> {
        self.backends.iter().find(|b| b.lane() == l)
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

    /// Runs a batch: placed, one submission per lane, the lanes together.
    pub fn run(&self, batch: Batch) -> Result<(Vec<Done>, Report), String> {
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
        let results: Vec<(Lane, Result<Vec<Output>, String>, f64)> = std::thread::scope(|s| {
            let handles: Vec<_> = Lane::ALL
                .iter()
                .filter(|l| !per_lane[l.index()].is_empty())
                .map(|&lane| {
                    let ks: Vec<&Kernel> = per_lane[lane.index()].iter().map(|&i| &batch.0[i].kernel).collect();
                    let backend = self.backend(lane).expect("placed on a present lane");
                    s.spawn(move || {
                        let t = Instant::now();
                        let r = backend.submit(&ks);
                        (lane, r, t.elapsed().as_secs_f64())
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("a lane's thread panicked")).collect()
        });
        let mut outs: Vec<Option<Output>> = vec![None; batch.0.len()];
        for (lane, r, wall) in results {
            let r = r.map_err(|e| format!("{} lane: {e}", lane.name()))?;
            let ids = &per_lane[lane.index()];
            if r.len() != ids.len() {
                return Err(format!("{} lane: {} outputs for {} launches", lane.name(), r.len(), ids.len()));
            }
            report.wall[lane.index()] = wall;
            report.joules[lane.index()] = self.prices.joules(lane, wall);
            for (&i, o) in ids.iter().zip(r) {
                outs[i] = Some(o);
            }
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

/// The CPU lane: the reference kernels on the rayon pool, the batch's launches in one parallel pass.
pub struct CpuBackend;

impl Backend for CpuBackend {
    fn lane(&self) -> Lane {
        Lane::Cpu
    }
    fn can(&self, _: &Kernel) -> bool {
        true
    }
    fn submit(&self, ks: &[&Kernel]) -> Result<Vec<Output>, String> {
        use rayon::prelude::*;
        Ok(ks.par_iter().map(|k| crate::cpu::run(k)).collect())
    }
}

/// The process's user + system CPU seconds.
pub fn process_cpu() -> f64 {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    let s = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 * 1e-6;
    s(u.ru_utime) + s(u.ru_stime)
}
