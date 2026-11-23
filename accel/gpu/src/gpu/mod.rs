//! The gpu lane (decision 0098 §5, 0095 §3): the four kernels in UKL (decision 0440 step C) on the process's
//! device, whatever its backend, a batch as **one submission** on the device's queue.
//!
//! A submission plans each launch as a list of **stages**: launches that read only the launch's inputs and what
//! its earlier stages wrote. All launches' stage 0 launches are enqueued first, then every launch's stage 1, and so
//! on; the queue runs its work in order, and the host waits once.
//!
//! The kernels (`relational.uglykl`, compiled once per process, and the UKL library's scans and histogram):
//!
//! - **bit product**: 64 × 64 output tiles, AND and popcount on the threads (the spikes' W1/W5 bit gram). With
//!   thresholds, each tile writes its pairs as mask bits; a scan of the mask words' popcounts gives each word its
//!   offset, and a scatter writes the pairs in `(i, j)` order.
//! - **count table**: a threadgroup per partition, a 2,048-counter table in threadgroup memory (8 KB), one pass per
//!   2,048-code window up to the partition's largest code. Pairs: the per-row counts, a scan, and a scatter in
//!   which each left row walks its partition's right rows in order (the units price exactly that).
//! - **histogram**: a threadgroup per partition, threadgroup atomics, a pass per 2,048-code window of `width`.
//! - **closure**: components and the condensed DAG's levels on the host (Tarjan's, linear), then the components'
//!   bit rows filled on the device level by level, each level one dispatch.
//!
//! Inputs are wrapped without a copy when page-aligned, and copied otherwise (an `Arc<[T]>`'s data follows its
//! counts, so it is page-aligned only when the caller made it so). Outputs are counted, then scattered: deterministic, and equal to [`rerail_accel::cpu::run`]'s. A pairs output is
//! written into a buffer of `min(bound, PAIRS_CAP)` pairs; when the device's total is larger, its scatter runs again
//! in a second command buffer into a buffer of the exact size (the counts and offsets stay on the device).

use rerail_accel::exec::Backend;
use rerail_accel::kernel::{Bits, JoinBy, Kernel, Output};
use rerail_accel::lanes::Lane;
use rerail_device::device::{Buffer, ComputeFunction, ComputeQueue};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

// The pool, the launch ledger and the batches' counts are rerail-accel's (below every crate that leases or counts).
pub use rerail_accel::gpu::{launch, programs, programs_begun, released, shared};

pub mod join;
pub mod kernels;
pub mod space;
pub mod tile;

/// The lane's own kernels (UKL, decision 0440 step C); the scans, the histogram and the zero fill are the UKL
/// library's (`rerail_gpu::library`, a queue's fill).
const SOURCE: &str = include_str!("relational.uglykl");

/// Threads per threadgroup of every kernel (the bit product's as 16 × 16).
const TG: u64 = 256;
/// Pairs a first pass writes at most before the exact rerun (2 MB). A larger first buffer is not free: with 32 MB,
/// a submission's wall time grew by about 50 ms (the pages' preparation), more than a second command buffer's
/// round trip (0.2 to 1.5 ms).
const PAIRS_CAP: u64 = 1 << 18;

#[derive(Clone, Copy, Debug)]
enum Pipe {
    BpTile,
    BpScatter,
    CtCount,
    CtScatter,
    ClInit,
    ClLevel,
    ClGather,
}

const PIPES: [&str; 7] = ["bp_tile", "bp_scatter", "ct_count", "ct_scatter", "cl_init", "cl_level", "cl_gather"];

/// `f`, a compile that is fatal where it fails (decision 0382), as a result: what compiling ahead reports (its use
/// still stops the run, naming the kernel).
pub fn caught(f: impl FnOnce()) -> Result<(), String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|p| {
        p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "a panic".into())
    })
}

/// The lane's kernels, to compile ahead of their first use (`rerail_gpu::kernels::compile_ahead`): the UKL library,
/// the joins' fixed kernels and each width's sort and gather (as `join::prewarm` makes them), and the relational
/// kernels (`relational.uglykl`). Each is made as its use makes it, so its use then finds it.
pub fn kernels_ahead() -> Vec<rerail_gpu::kernels::Ahead> {
    let mut v = vec![ahead("ukl library", || released(rerail_gpu::library::ready))];
    // The fused sorts and probes only where the device can run them (`Fused::can`), as `join::prewarm`.
    let fused: &[&'static str] = if join::Fused::can() { &["sort_all", "probe_all"] } else { &[] };
    for &name in ["pair_keys", "col_span", "col_base", "col_narrow"].iter().chain(fused) {
        v.push(ahead(format!("join {name}"), move || released(|| join::fixed(name).map(drop))));
    }
    for w in [join::Width::W64, join::Width::W32, join::Width::W16] {
        v.push(ahead(format!("join sort {w:?}"), move || released(|| join::sort_and_gather(w))));
    }
    for name in PIPES {
        v.push(ahead(format!("relational {name}"), move || released(|| caught(|| _ = kernels::function_fixed(SOURCE, name, None)))));
    }
    v
}

/// A kernel to compile ahead (`rerail_gpu::kernels::Ahead`), made in its caller's run (decision 0442): its compile
/// runs on a thread of its own, and what it makes is the caller's machine's, as its use's is.
pub fn ahead(name: impl Into<String>, make: impl Fn() -> Result<(), String> + Send + Sync + 'static) -> rerail_gpu::kernels::Ahead {
    let run = rerail_context::job_run();
    rerail_gpu::kernels::Ahead::new(name, move || {
        let _run = rerail_context::enter(run.clone());
        make()
    })
}

/// The process's device in the device layer and the compiled kernels: once per process, on any backend.
struct Gpu {
    layer: shared::Layer,
    functions: Vec<Arc<ComputeFunction>>,
    page: usize,
    max_buffer: u64,
}

impl Gpu {
    fn new() -> Result<Gpu, String> {
        // The device and queue every relational submission shares (decision 0100 §2).
        let layer = shared::layer().ok_or("no device")?;
        if layer.device.max_threadgroup() < TG as u32 {
            return Err(format!("gpu lane: the device runs {} threads a threadgroup, not {TG}", layer.device.max_threadgroup()));
        }
        // Through the store of compiled kernels (decision 0395); one that does not compile is fatal.
        let functions = PIPES.iter().map(|name| kernels::function_fixed(SOURCE, name, None)).collect();
        let page = rerail_device::os::page_bytes();
        Ok(Gpu { layer, functions, page, max_buffer: layer.max_buffer })
    }
}

/// The machine's GPU lane (decision 0442): its device's queue and pipelines (its batches' counts: `programs`).
#[derive(Default)]
struct GpuLane {
    gpu: OnceLock<Option<Arc<Gpu>>>,
}

fn gpu_lane() -> Arc<GpuLane> {
    rerail_context::machine::<GpuLane>()
}

fn gpu() -> Option<Arc<Gpu>> {
    gpu_lane().gpu.get_or_init(|| Gpu::new().map_err(|e| eprintln!("{e}")).ok().map(Arc::new)).clone()
}

/// This lane's backend, when this machine has a Metal device and the kernels compiled: what accel's executor takes
/// (`rerail_accel::gpu::BACKEND`, registered by `rerail_accel::install`).
pub fn backend() -> Option<Arc<dyn Backend>> {
    rerail_accel::lanes::ask_device_lane();
    gpu().map(|gpu| {
        // The joins' small path compiles in the background, so the first chunk's batch does not wait for it.
        join::prewarm();
        Arc::new(GpuBackend { gpu, space: Default::default() }) as Arc<dyn Backend>
    })
}

/// The gpu lane.
pub struct GpuBackend {
    gpu: Arc<Gpu>,
    /// What the lane keeps between batches: joins' right sides (decision 0100 §4).
    space: std::sync::Mutex<space::Space>,
}

/// What a submission took on the device.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    /// Device seconds, `GPUEndTime - GPUStartTime`, summed over the command buffers.
    pub device: f64,
    /// Host seconds making the jobs' buffers (copies in), and reading the outputs back (copies out).
    pub encode: f64,
    pub decode: f64,
    /// Command buffers: 1, or 2 when a pairs output outgrew its first buffer.
    pub command_buffers: usize,
    pub dispatches: usize,
    /// Memory barriers in the encoder.
    pub barriers: usize,
}

/// The gpu lane's backend, as its own type (the timing test reads [`Stats`]).
pub fn gpu_backend() -> Option<GpuBackend> {
    gpu().map(|gpu| GpuBackend { gpu, space: Default::default() })
}

impl Backend for GpuBackend {
    fn lane(&self) -> Lane {
        Lane::Gpu
    }

    fn can(&self, k: &Kernel) -> bool {
        let max = self.gpu.max_buffer;
        let fits = |bytes: u64| bytes <= max;
        let u32ok = |x: u64| x <= u32::MAX as u64;
        match k {
            Kernel::BitProduct { a, b, min } => {
                let (n, m, kw) = (a.rows as u64, b.rows as u64, a.words as u64);
                let shape = a.words == b.words && fits(n * kw * 8) && fits(m * kw * 8) && u32ok(n) && u32ok(m) && u32ok(kw) && n.div_ceil(64) <= 65535;
                shape
                    && match min {
                        None => fits(n * m * 4),
                        // The pairs' offsets are u32, and the pairs fit one buffer.
                        Some(min) => min.len() as u64 == m && u32ok(n * m) && fits(n * m * 8) && fits(n * m.div_ceil(32) * 4),
                    }
            }
            Kernel::CountTable { loff, left, roff, right, emit } => {
                let shape = !loff.is_empty() && loff.len() == roff.len() && u32ok(left.len() as u64) && u32ok(right.len() as u64);
                shape && (!emit || {
                    let bound: u64 = loff.windows(2).zip(roff.windows(2)).map(|(l, r)| (l[1].saturating_sub(l[0]) as u64) * (r[1].saturating_sub(r[0]) as u64)).sum();
                    u32ok(bound) && fits(bound * 8)
                })
            }
            Kernel::Histogram { off, codes, width } => !off.is_empty() && u32ok(codes.len() as u64) && fits((off.len() as u64 - 1) * *width as u64 * 4),
            Kernel::Closure { n, off, .. } => {
                let (n, words) = (*n as u64, (*n as u64).div_ceil(64).max(1));
                off.len() as u64 == n + 1 && u32ok(n * words * 2) && fits(n * words * 8)
            }
            // A program's kernels are UKL, on the process's device (decision 0440 step C).
            Kernel::Program(p) => p.0.per_unit(rerail_accel::lanes::Lane::Gpu).is_some(),
            Kernel::Join { left, right, by, .. } => {
                let Ok((nl, nr)) = Kernel::join_shape(left, right) else {
                    return false;
                };
                // The pairs' space (two a left row to start) and the columns at 64 bits.
                let shape = fits(nl as u64 * 16) && fits(nr as u64 * 8) && (nl as u64) < (u32::MAX as u64) / 2;
                shape && (*by == JoinBy::Probe || left.len() <= 3)
            }
        }
    }

    fn submit(&self, ks: &[&Kernel]) -> Result<Vec<Output>, String> {
        self.submit_timed(ks).map(|(o, _)| o)
    }

    /// Programs submit their own command buffers on the one queue (`Program::run`), the joins theirs
    /// (`space::run`), the other forms one together (`run`, which times its phases); the programs' and joins' wall
    /// time counts as the device's.
    fn submit_timed(&self, ks: &[&Kernel]) -> Result<(Vec<Output>, rerail_accel::exec::Phases), String> {
        let (programs, rest): (Vec<usize>, Vec<usize>) = (0..ks.len()).partition(|&i| matches!(ks[i], Kernel::Program(_)));
        let (joins, others): (Vec<usize>, Vec<usize>) = rest.into_iter().partition(|&i| matches!(ks[i], Kernel::Join { .. }));
        let mut outs: Vec<Option<Output>> = vec![None; ks.len()];
        let mut phases = rerail_accel::exec::Phases::default();
        if !programs.is_empty() {
            let t = std::time::Instant::now();
            for &i in &programs {
                let Kernel::Program(p) = ks[i] else { unreachable!("a program") };
                let r = released(|| p.0.run(rerail_accel::lanes::Lane::Gpu)).map_err(|e| format!("program {}: {e}", p.0.name()))?;
                outs[i] = Some(Output::Program(rerail_accel::kernel::ProgramResult(r)));
            }
            phases.device = Some(t.elapsed().as_secs_f64());
        }
        if !others.is_empty() {
            let (o, s) = self.run(&others.iter().map(|&i| ks[i]).collect::<Vec<_>>())?;
            phases = rerail_accel::exec::Phases { encode: s.encode, device: Some(phases.device.unwrap_or(0.0) + s.device), decode: s.decode, worker_cpu: 0.0 };
            for (&i, o) in others.iter().zip(o) {
                outs[i] = Some(o);
            }
        }
        if !joins.is_empty() {
            let t = std::time::Instant::now();
            let o = released(|| space::run(&self.space, &joins.iter().map(|&i| ks[i]).collect::<Vec<_>>()))?;
            phases.device = Some(phases.device.unwrap_or(0.0) + t.elapsed().as_secs_f64());
            for (&i, o) in joins.iter().zip(o) {
                outs[i] = Some(o);
            }
        }
        Ok((outs.into_iter().map(|o| o.expect("every launch ran")).collect(), phases))
    }

    /// A batch of joins only: its first command buffer committed, and read by `finish` (the worker begins its next
    /// submission meanwhile). Any other batch runs as it begins.
    fn start<'a>(&'a self, ks: &[&Kernel]) -> Result<Box<dyn rerail_accel::exec::Pending + 'a>, String> {
        if !ks.is_empty() && ks.iter().all(|k| matches!(k, Kernel::Join { .. })) {
            let started = released(|| space::start(&self.space, ks))?;
            return Ok(Box::new(started));
        }
        // A batch of programs: each submits its own command buffers and waits for them, so it runs on the lanes'
        // pool while the worker begins the next submission. One program's host part then runs beside another's on
        // the device; each holds its own device memory (decision 0344: leases, no process-wide lock).
        if overlap() && !ks.is_empty() && ks.iter().all(|k| matches!(k, Kernel::Program(_))) {
            let programs: Vec<_> = ks.iter().map(|k| match k {
                Kernel::Program(p) => Arc::clone(&p.0),
                _ => unreachable!("a program"),
            }).collect();
            let lane = rerail_accel::gpu::programs();
            let beside = lane.running.fetch_add(1, std::sync::atomic::Ordering::AcqRel) > 0;
            lane.begun.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if beside {
                lane.beside.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            let take = rerail_accel::exec::lane_spawn(move || {
                let _running = RunningCount(lane);
                programs
                    .iter()
                    .map(|p| {
                        released(|| p.run(rerail_accel::lanes::Lane::Gpu))
                            .map(|r| Output::Program(rerail_accel::kernel::ProgramResult(r)))
                            .map_err(|e| format!("program {}: {e}", p.name()))
                    })
                    .collect::<Result<Vec<_>, String>>()
            });
            return Ok(Box::new(Running(take)));
        }
        let outs = self.submit(ks);
        Ok(Box::new(Done(outs)))
    }

    fn forget(&self, id: u64) {
        if let Ok(mut s) = self.space.lock() {
            s.forget(id);
        }
    }

    fn clear(&self) {
        if let Ok(mut s) = self.space.lock() {
            s.clear();
        }
    }

    fn space(&self) -> space::Counts {
        self.space.lock().map(|s| s.counts()).unwrap_or_default()
    }
}

impl rerail_accel::exec::Pending for space::Started {
    fn finish(self: Box<Self>) -> Result<Vec<Output>, String> {
        released(|| space::Started::finish(*self))
    }
}

/// Whether a batch of programs runs beside the one before (0344): on unless `RERAIL_GPU_OVERLAP=off`, which runs
/// each as it begins on the lane's worker, to time what the overlap gives.
fn overlap() -> bool {
    // The run's setting (decision 0442).
    rerail_context::setting(|| std::env::var("RERAIL_GPU_OVERLAP").as_deref() != Ok("off"))
}

/// A submission that ran as it began.
struct Done(Result<Vec<Output>, String>);

impl rerail_accel::exec::Pending for Done {
    fn finish(self: Box<Self>) -> Result<Vec<Output>, String> {
        self.0
    }
    fn done(&self) -> bool {
        true
    }
}

/// One running batch of programs, counted while it lives.
struct RunningCount(Arc<rerail_accel::gpu::Programs>);

impl Drop for RunningCount {
    fn drop(&mut self) {
        self.0.running.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// A batch of programs running on the lanes' pool ([`rerail_accel::exec::lane_spawn`]).
struct Running(std::sync::mpsc::Receiver<Ran>);
/// A batch's outputs (or why it failed, or its panic) and the CPU its thread spent.
type Ran = (std::thread::Result<Result<Vec<Output>, String>>, f64);

impl rerail_accel::exec::Pending for Running {
    fn finish(self: Box<Self>) -> Result<Vec<Output>, String> {
        self.finish_counted().0
    }
    fn finish_counted(self: Box<Self>) -> (Result<Vec<Output>, String>, f64) {
        match self.0.recv() {
            Ok((Ok(r), cpu)) => (r, cpu),
            Ok((Err(p), cpu)) => {
                rerail_accel::exec::resume_over_budget(p);
                (Err("its submission panicked".into()), cpu)
            }
            Err(_) => (Err("its submission's thread ended without a result".into()), 0.0),
        }
    }
    fn finish_on_pool(&self) -> bool {
        false
    }
}

impl GpuBackend {
    /// Runs `ks` as one submission, with what it took on the device: an evaluation of the launch budget (`launch`),
    /// or the caller's when it is in one, its pairs' scatter again a rerun's.
    pub fn run(&self, ks: &[&Kernel]) -> Result<(Vec<Output>, Stats), String> {
        let _evaluation = launch::LaunchScope::enter();
        let mut plan = Planner { gpu: &self.gpu, inputs: HashMap::new(), leases: Default::default() };
        let t = std::time::Instant::now();
        let jobs: Vec<Job> = rerail_accel::score::phase("upload", || ks.iter().map(|k| plan.job(k)).collect::<Result<_, _>>())?;
        let mut stats = Stats { encode: t.elapsed().as_secs_f64(), ..Stats::default() };
        self.execute(jobs.iter().map(|j| j.stages.as_slice()), &mut stats)?;
        // Pairs that outgrew their first buffer: their scatter again, into the exact size.
        let mut rerun: Vec<Step> = Vec::new();
        let mut exact: HashMap<usize, (Buffer, u64)> = HashMap::new();
        for (i, j) in jobs.iter().enumerate() {
            if let Read::Pairs { total, cap, rescatter, .. } = &j.read {
                let t = total.read() as u64;
                if t > *cap {
                    if t * 8 > self.gpu.max_buffer {
                        return Err(format!("launch {i}: {t} pairs do not fit a buffer"));
                    }
                    let buf = plan.alloc(t * 8);
                    let mut d = rescatter.dispatch.clone();
                    d.set("pairs", Bind::Buf(buf.clone()));
                    d.set("args", Bind::Bytes((rescatter.args)(t as u32)));
                    rerun.push(Step::Kernel(d));
                    exact.insert(i, (buf, t));
                }
            }
        }
        if !rerun.is_empty() {
            let _rerun = launch::rerun();
            let stage = [Stage { steps: rerun }];
            self.execute(std::iter::once(&stage[..]), &mut stats)?;
        }
        let _read = ReadPhase(rerail_accel::score::thread_cpu(), std::time::Instant::now());
        let t = std::time::Instant::now();
        let mut outs = Vec::with_capacity(jobs.len());
        for (i, j) in jobs.into_iter().enumerate() {
            if let Some(bad) = &j.bad
                && read::<u32>(bad, 1)[0] != 0
            {
                return Err(format!("launch {i} ({}): a code is not below the width", ks[i].form().name()));
            }
            outs.push(match j.read {
                Read::Ready(o) => o,
                Read::Counts { rows, cols, buf } => Output::Counts { rows, cols, data: read::<u32>(&buf, rows * cols) },
                Read::PerRow { n, buf } => Output::PerRow(read::<u32>(&buf, n)),
                Read::Bits { rows, words, buf } => Output::Bits(Bits::new(rows, words, read::<u64>(&buf, rows * words))),
                Read::Pairs { total, buf, .. } => {
                    let t = total.read() as usize;
                    let buf = exact.remove(&i).map_or(buf, |(b, _)| b);
                    Output::Pairs(read::<(u32, u32)>(&buf, t))
                }
            });
        }
        stats.decode = t.elapsed().as_secs_f64();
        Ok((outs, stats))
    }

    /// One submission on the device's queue: every job's stage `s`, then stage `s + 1`. The queue runs its work in
    /// order, so each stage reads what the stages before it wrote (a stage boundary is counted as a barrier).
    fn execute<'a>(&self, jobs: impl Iterator<Item = &'a [Stage]> + Clone, stats: &mut Stats) -> Result<(), String> {
        let gpu = &*self.gpu;
        let (c0, t0) = (rerail_accel::score::thread_cpu(), std::time::Instant::now());
        let q = shared::Queue::new()?;
        // The submission's key in the evaluation's records.
        let key = Box::new(0u8);
        let pointer = std::ptr::from_ref::<u8>(&key) as usize;
        let depth = jobs.clone().map(|j| j.len()).max().unwrap_or(0);
        for s in 0..depth {
            if s > 0 {
                stats.barriers += 1;
            }
            for stage in jobs.clone().filter_map(|j| j.get(s)) {
                for step in &stage.steps {
                    stats.dispatches += step.enqueue(gpu, &q, pointer)?;
                }
            }
        }
        rerail_accel::score::add("encode", rerail_accel::score::thread_cpu() - c0, t0.elapsed().as_secs_f64());
        launch::check_submission();
        launch::mark(pointer, false);
        let event = rerail_accel::score::phase("gpu commit", || q.flush());
        // What the steps read and write is held until the submission completes (decision 0487 step 2).
        rerail_accel::score::phase("gpu wait", || event.await_()).map_err(|e| format!("the submission failed: {e}"))?;
        launch::mark(pointer, true);
        stats.device += event.gpu_seconds();
        stats.command_buffers += 1;
        Ok(())
    }
}

/// The read phase of a run (`rerail_accel::score`), counted when it drops: the outputs copied from their buffers.
struct ReadPhase(f64, std::time::Instant);

impl Drop for ReadPhase {
    fn drop(&mut self) {
        rerail_accel::score::add("read", rerail_accel::score::thread_cpu() - self.0, self.1.elapsed().as_secs_f64());
    }
}

// The device writes pairs as two u32s; `(u32, u32)` must have that layout for `read` to copy them as they are.
const _: () = assert!(size_of::<(u32, u32)>() == 8 && std::mem::offset_of!((u32, u32), 0) == 0 && std::mem::offset_of!((u32, u32), 1) == 4);

/// `n` values from the start of a buffer the host shares.
fn read<T: Copy>(buf: &Buffer, n: usize) -> Vec<T> {
    if n == 0 {
        return Vec::new();
    }
    assert!(buf.length() as usize >= n * size_of::<T>(), "gpu lane: reading past a buffer");
    // copy: not block data (a kernel's output, handed to the caller as its own)
    // SAFETY: a buffer the device finished writing (the submission completed); `n` values fit it.
    unsafe { std::slice::from_raw_parts(buf.contents() as *const T, n) }.to_vec()
}

/// A `#[repr(C)]` value's bytes, as a kernel's `ARGS` row takes them.
fn bytes_of<T: Copy>(v: &T) -> Vec<u8> {
    // SAFETY: the value's bytes; the structs passed have no padding.
    unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), size_of::<T>()) }.to_vec()
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BitProductArgs {
    n: u32,
    m: u32,
    kw: u32,
    mode: u32,
    mw: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct ScatterArgs {
    cap: u32,
    mw: u32,
    nw: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct ClosureArgs {
    n: u32,
    words: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct LevelArgs {
    lo: u32,
    hi: u32,
    words: u32,
}

#[derive(Clone)]
enum Bind {
    Buf(Buffer),
    Bytes(Vec<u8>),
}

/// One launch of the lane's kernels: its rows by name, `groups` threadgroups of `threads`.
#[derive(Clone)]
struct Dispatch {
    pipe: Pipe,
    binds: Vec<(&'static str, Bind)>,
    groups: [u64; 3],
    threads: [u64; 3],
}

impl Dispatch {
    fn new(pipe: Pipe, binds: Vec<(&'static str, Bind)>, groups: u64) -> Self {
        Dispatch { pipe, binds, groups: [groups, 1, 1], threads: [TG, 1, 1] }
    }

    /// One thread per element of `n`.
    fn per(pipe: Pipe, binds: Vec<(&'static str, Bind)>, n: u64) -> Self {
        Dispatch::new(pipe, binds, n.div_ceil(TG))
    }

    fn set(&mut self, name: &str, b: Bind) {
        let slot = self.binds.iter_mut().find(|(n, _)| *n == name).expect("a bound row");
        slot.1 = b;
    }
}

/// One step of a stage: a launch of the lane's kernels, or of the UKL library's.
#[derive(Clone)]
enum Step {
    Kernel(Dispatch),
    /// `output[..=n]`: the exclusive scan of `input[..n]` (of its words' popcounts when `pop`), the total last.
    Scan { input: Buffer, output: Buffer, n: u32, pop: bool },
    /// `H[p * width + c]`: partition `p`'s codes equal to `c`; `bad` set by a code at or above `width`.
    Histogram { codes: Buffer, offsets: Buffer, parts: u32, width: u32, h: Buffer, bad: Buffer },
    /// `words` zero u32s from the buffer's start.
    Zero { buf: Buffer, words: usize },
}

impl Step {
    /// Enqueued on `q`, each launch charged first to the launch budget as the command `pointer`'s (`launch`): the
    /// launches it took. A launch past the budget is the hard failure (decision 0601), before it is enqueued.
    fn enqueue(&self, gpu: &Gpu, q: &ComputeQueue, pointer: usize) -> Result<usize, String> {
        let charge = |kernel: &str, groups: [u64; 3], threads: [u64; 3]| {
            let grid = [groups[0] * threads[0], groups[1] * threads[1], groups[2] * threads[2]];
            launch::attempt(pointer, kernel, grid, threads, false, Vec::new());
            Ok::<(), String>(())
        };
        match self {
            Step::Kernel(d) => {
                if d.groups.iter().product::<u64>() == 0 {
                    return Ok(0);
                }
                charge(&format!("{:?}", d.pipe), d.groups, d.threads)?;
                let mut l = rerail_gpu::kernels::launch(q, &gpu.functions[d.pipe as usize]);
                for (name, b) in &d.binds {
                    l = match b {
                        Bind::Buf(b) => l.buffer(name, b),
                        Bind::Bytes(v) => l.bytes(name, v),
                    };
                }
                l.run(&d.groups, &d.threads).map_err(|e| format!("{:?}: {e}", d.pipe))?;
                Ok(1)
            }
            Step::Scan { input, output, n, pop } => {
                // The library says its launches once it has made them (its levels' count): charged then, past the
                // budget the hard failure.
                let made = rerail_gpu::library::scan_exclusive_u32_in(q, input, output, *n, *pop, &rerail_gpu::library::ScanSpace::new(*n)?)?;
                launch::charge_made(pointer, "ukl_scan_u32", made);
                Ok(made)
            }
            Step::Histogram { codes, offsets, parts, width, h, bad } => {
                // One launch of the library's (`histogram`).
                charge("ukl_histogram", [u64::from(*parts), 1, 1], [1, 1, 1])?;
                rerail_gpu::library::histogram::<u16>(q, codes, offsets, *parts, *width, h, bad)?;
                Ok(1)
            }
            Step::Zero { buf, words } => {
                // A fill, not a compute launch (as a `lane::Command`'s fill).
                q.enqueue_fill_array(buf, &0u32, 0, Some(*words));
                Ok(0)
            }
        }
    }
}

/// Steps that may run together.
struct Stage {
    steps: Vec<Step>,
}

impl Stage {
    fn one(step: Step) -> Self {
        Stage { steps: vec![step] }
    }
}

/// A u32 the device writes at word `at` of `buf` (a scan's total).
struct Word {
    buf: Buffer,
    at: usize,
}

impl Word {
    fn read(&self) -> u32 {
        read::<u32>(&self.buf, self.at + 1)[self.at]
    }
}

/// A pairs output's scatter, to run again into a buffer of the exact size: its dispatch, and its `args` for a
/// capacity.
struct Rescatter {
    dispatch: Dispatch,
    args: Box<dyn Fn(u32) -> Vec<u8>>,
}

/// How a launch's output is read back after the wait.
enum Read {
    Ready(Output),
    Counts { rows: usize, cols: usize, buf: Buffer },
    PerRow { n: usize, buf: Buffer },
    Bits { rows: usize, words: usize, buf: Buffer },
    /// `total` holds the pairs' count; `buf` the first `cap` of them.
    Pairs { total: Word, buf: Buffer, cap: u64, rescatter: Rescatter },
}

struct Job {
    stages: Vec<Stage>,
    read: Read,
    /// A flag the device sets on invalid input.
    bad: Option<Buffer>,
}

impl Job {
    fn ready(o: Output) -> Self {
        Job { stages: Vec::new(), read: Read::Ready(o), bad: None }
    }
}

/// Builds a submission's jobs; inputs shared by several launches are wrapped once.
struct Planner<'g> {
    gpu: &'g Gpu,
    inputs: HashMap<(usize, usize), Buffer>,
    /// The pooled buffers the submission's jobs use, returned once its work has completed and its outputs are
    /// read (when the planner drops).
    leases: std::cell::RefCell<Vec<shared::Pooled>>,
}

fn u(x: usize) -> u32 {
    x as u32
}

impl Planner<'_> {
    /// A zeroed buffer of the pool (the kernels count into some, and read a new buffer as zero).
    fn alloc(&self, bytes: u64) -> Buffer {
        let lease = shared::lease_zeroed(bytes.max(16) as usize).unwrap_or_else(|e| panic!("gpu lane: {e}"));
        let b = lease.device_buffer().clone();
        self.leases.borrow_mut().push(lease);
        b
    }

    /// A kernel's input column: wrapped without a copy when page-aligned, else copied. The kernel outlives the
    /// wait, so the memory stays valid while the device reads it, and a column several launches share (the same
    /// address and length) is wrapped once.
    fn input<T: Copy>(&mut self, data: &[T]) -> Buffer {
        let (ptr, bytes) = (data.as_ptr() as usize, size_of_val(data));
        if let Some(b) = self.inputs.get(&(ptr, bytes)) {
            return b.clone();
        }
        let wrapped = (bytes != 0 && ptr % self.gpu.page == 0 && bytes % self.gpu.page == 0)
            // SAFETY: the caller's column, which outlives the submission (the planner's work is awaited before it
            // returns), read and never written by the kernels.
            .then(|| unsafe { rerail_device::device::wrap_host_memory(self.gpu.layer.device, ptr as *mut u8, bytes) }.ok())
            .flatten();
        let b = wrapped.unwrap_or_else(|| self.upload(data));
        self.inputs.insert((ptr, bytes), b.clone());
        b
    }

    /// A copy of host data the planner made (never cached: its address is reused once it is dropped).
    fn upload<T: Copy>(&self, data: &[T]) -> Buffer {
        let bytes = size_of_val(data);
        if bytes == 0 {
            return self.alloc(16);
        }
        let lease = shared::lease(bytes).unwrap_or_else(|e| panic!("gpu lane: {e}"));
        // copy: not block data (a caller's column, not page-aligned, which the device cannot wrap in place)
        // SAFETY: the lease is the planner's alone and at least `bytes` long; no command uses it yet.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr().cast::<u8>(), lease.host(), bytes) };
        let b = lease.device_buffer().clone();
        self.leases.borrow_mut().push(lease);
        b
    }

    /// The exclusive scan of `n` u32 (their popcounts when `pop`), the UKL library's: a stage appended to `st`.
    /// Returns the scanned values, the total at word `n`.
    fn scan(&self, st: &mut Vec<Stage>, input: &Buffer, n: u64, pop: bool) -> (Buffer, Word) {
        let ex = self.alloc((n + 1) * 4);
        st.push(Stage::one(Step::Scan { input: input.clone(), output: ex.clone(), n: n as u32, pop }));
        (ex.clone(), Word { buf: ex, at: n as usize })
    }

    fn offsets(off: &[u32], len: usize, what: &str) -> Result<(), String> {
        let ok = off.first() == Some(&0) && off.last().map(|&l| l as usize) == Some(len) && off.windows(2).all(|w| w[0] <= w[1]);
        if ok { Ok(()) } else { Err(format!("{what}: offsets are not ascending from 0 to the column's length")) }
    }

    fn job(&mut self, k: &Kernel) -> Result<Job, String> {
        match k {
            Kernel::BitProduct { a, b, min } => self.bit_product(a, b, min.as_deref()),
            Kernel::CountTable { loff, left, roff, right, emit } => self.count_table(loff, left, roff, right, *emit),
            Kernel::Histogram { off, codes, width } => self.histogram(off, codes, *width),
            Kernel::Closure { n, off, dst } => self.closure(*n, off, dst),
            Kernel::Join { .. } => Err("a join runs in the space's own command buffers".into()),
            Kernel::Program(_) => Err("a program submits its own command buffers".into()),
        }
    }

    fn bit_product(&mut self, a: &Bits, b: &Bits, min: Option<&[u32]>) -> Result<Job, String> {
        if a.words != b.words || min.is_some_and(|m| m.len() != b.rows) {
            return Err("bit product: rows of different widths, or not one threshold per row of b".into());
        }
        let (n, m, kw) = (a.rows as u64, b.rows as u64, a.words as u64);
        if n == 0 || m == 0 {
            return Ok(Job::ready(match min {
                None => Output::Counts { rows: a.rows, cols: b.rows, data: Vec::new() },
                Some(_) => Output::Pairs(Vec::new()),
            }));
        }
        let (ba, bb) = (self.input(&a.data), self.input(&b.data));
        let dummy = self.alloc(16);
        let mw = m.div_ceil(32);
        let tile = |c: &Buffer, mask: &Buffer, minb: &Buffer, mode: u32| {
            let args = BitProductArgs { n: u(a.rows), m: u(b.rows), kw: kw as u32, mode, mw: mw as u32 };
            let binds = vec![("A", Bind::Buf(ba.clone())), ("B", Bind::Buf(bb.clone())), ("C", Bind::Buf(c.clone())), ("mask", Bind::Buf(mask.clone())), ("minv", Bind::Buf(minb.clone())), ("args", Bind::Bytes(bytes_of(&args)))];
            Step::Kernel(Dispatch { pipe: Pipe::BpTile, binds, groups: [m.div_ceil(64), n.div_ceil(64), 1], threads: [16, 16, 1] })
        };
        let Some(min) = min else {
            let c = self.alloc(n * m * 4);
            return Ok(Job { stages: vec![Stage::one(tile(&c, &dummy, &dummy, 0))], read: Read::Counts { rows: a.rows, cols: b.rows, buf: c }, bad: None });
        };
        let nw = n * mw;
        let mask = self.alloc(nw * 4);
        let minb = self.input(min);
        let mut st = vec![Stage::one(tile(&dummy, &mask, &minb, 1))];
        let (ex, total) = self.scan(&mut st, &mask, nw, true);
        let cap = (n * m).min(PAIRS_CAP);
        let pairs = self.alloc(cap * 8);
        let args = move |cap: u32| bytes_of(&ScatterArgs { cap, mw: mw as u32, nw: nw as u32 });
        let scatter = Dispatch::per(Pipe::BpScatter, vec![("mask", Bind::Buf(mask.clone())), ("ex", Bind::Buf(ex)), ("pairs", Bind::Buf(pairs.clone())), ("args", Bind::Bytes(args(cap as u32)))], nw);
        st.push(Stage::one(Step::Kernel(scatter.clone())));
        Ok(Job { stages: st, read: Read::Pairs { total, buf: pairs, cap, rescatter: Rescatter { dispatch: scatter, args: Box::new(args) } }, bad: None })
    }

    fn count_table(&mut self, loff: &[u32], left: &[u16], roff: &[u32], right: &[u16], emit: bool) -> Result<Job, String> {
        Self::offsets(loff, left.len(), "count table, left")?;
        Self::offsets(roff, right.len(), "count table, right")?;
        if loff.len() != roff.len() {
            return Err("count table: the sides have different partitions".into());
        }
        let nparts = (loff.len() - 1) as u64;
        if left.is_empty() {
            return Ok(Job::ready(if emit { Output::Pairs(Vec::new()) } else { Output::PerRow(Vec::new()) }));
        }
        let nl = left.len() as u64;
        let (bl, br, blo, bro) = (self.input(left), self.input(right), self.input(loff), self.input(roff));
        let counts = self.alloc(nl * 4);
        let count = Dispatch::new(Pipe::CtCount, vec![("left", Bind::Buf(bl.clone())), ("right", Bind::Buf(br.clone())), ("loff", Bind::Buf(blo.clone())), ("roff", Bind::Buf(bro.clone())), ("counts", Bind::Buf(counts.clone()))], nparts);
        let mut st = vec![Stage::one(Step::Kernel(count))];
        if !emit {
            return Ok(Job { stages: st, read: Read::PerRow { n: left.len(), buf: counts }, bad: None });
        }
        let (ex, total) = self.scan(&mut st, &counts, nl, false);
        let bound: u64 = loff.windows(2).zip(roff.windows(2)).map(|(l, r)| (l[1] - l[0]) as u64 * (r[1] - r[0]) as u64).sum();
        let cap = bound.min(PAIRS_CAP);
        let pairs = self.alloc(cap * 8);
        let args = |cap: u32| cap.to_ne_bytes().to_vec();
        let scatter = Dispatch::new(
            Pipe::CtScatter,
            vec![("left", Bind::Buf(bl)), ("right", Bind::Buf(br)), ("loff", Bind::Buf(blo)), ("roff", Bind::Buf(bro)), ("counts", Bind::Buf(counts)), ("ex", Bind::Buf(ex)), ("pairs", Bind::Buf(pairs.clone())), ("args", Bind::Bytes(args(cap as u32)))],
            nparts,
        );
        st.push(Stage::one(Step::Kernel(scatter.clone())));
        Ok(Job { stages: st, read: Read::Pairs { total, buf: pairs, cap, rescatter: Rescatter { dispatch: scatter, args: Box::new(args) } }, bad: None })
    }

    fn histogram(&mut self, off: &[u32], codes: &[u16], width: u32) -> Result<Job, String> {
        Self::offsets(off, codes.len(), "histogram")?;
        let nparts = off.len() - 1;
        // No counts to make: the reference reads no codes either.
        if nparts == 0 || width == 0 {
            return Ok(Job::ready(Output::Counts { rows: nparts, cols: width as usize, data: vec![0; nparts * width as usize] }));
        }
        let (bc, bo) = (self.input(codes), self.input(off));
        let h = self.alloc(nparts as u64 * width as u64 * 4);
        let bad = self.upload(&[0u32; 4]);
        let step = Step::Histogram { codes: bc, offsets: bo, parts: nparts as u32, width, h: h.clone(), bad: bad.clone() };
        Ok(Job { stages: vec![Stage::one(step)], read: Read::Counts { rows: nparts, cols: width as usize, buf: h }, bad: Some(bad) })
    }

    fn closure(&mut self, n: u32, off: &[u32], dst: &[u32]) -> Result<Job, String> {
        let nu = n as usize;
        let words = nu.div_ceil(64).max(1);
        Self::offsets(off, dst.len(), "closure")?;
        if off.len() != nu + 1 || dst.iter().any(|&w| w >= n) {
            return Err(format!("closure: not a CSR of {n} nodes"));
        }
        if nu == 0 {
            return Ok(Job::ready(Output::Bits(Bits::new(0, words, Vec::<u64>::new()))));
        }
        // The condensed DAG on the host: components, their successor components (deduplicated), cycles, levels.
        let (comp, ncomp) = rerail_accel::cpu::components(nu, off, dst);
        let mut start = vec![0u32; ncomp + 1];
        for &c in &comp {
            start[c as usize + 1] += 1;
        }
        for c in 0..ncomp {
            start[c + 1] += start[c];
        }
        let mut members = vec![0u32; nu];
        let mut fill = start.clone();
        for (v, &c) in comp.iter().enumerate() {
            members[fill[c as usize] as usize] = v as u32;
            fill[c as usize] += 1;
        }
        let (mut soff, mut sdst) = (Vec::with_capacity(ncomp + 1), Vec::new());
        let mut cyclic = vec![0u32; ncomp];
        let mut level = vec![0u32; ncomp];
        let mut seen = vec![u32::MAX; ncomp];
        for c in 0..ncomp {
            soff.push(sdst.len() as u32);
            let ms = &members[start[c] as usize..start[c + 1] as usize];
            cyclic[c] = (ms.len() > 1) as u32;
            for &v in ms {
                for &w in &dst[off[v as usize] as usize..off[v as usize + 1] as usize] {
                    let d = comp[w as usize];
                    if w == v {
                        cyclic[c] = 1;
                    }
                    if d as usize != c && seen[d as usize] != c as u32 {
                        seen[d as usize] = c as u32;
                        sdst.push(d);
                        // Tarjan's numbering puts successors first: their levels are known.
                        level[c] = level[c].max(level[d as usize] + 1);
                    }
                }
            }
        }
        soff.push(sdst.len() as u32);
        let levels = level.iter().copied().max().unwrap_or(0) as usize + 1;
        let mut lstart = vec![0u32; levels + 1];
        for &l in &level {
            lstart[l as usize + 1] += 1;
        }
        for l in 0..levels {
            lstart[l + 1] += lstart[l];
        }
        let mut order = vec![0u32; ncomp];
        let mut lfill = lstart.clone();
        for (c, &l) in level.iter().enumerate() {
            order[lfill[l as usize] as usize] = c as u32;
            lfill[l as usize] += 1;
        }
        let (boff, bdst) = (self.input(off), self.input(dst));
        let (bcomp, bcyc, border, bsoff, bsdst) = (self.upload(&comp), self.upload(&cyclic), self.upload(&order), self.upload(&soff), self.upload(&sdst));
        let row32 = (ncomp * words * 2) as u64;
        let r = self.alloc(row32 * 4);
        let out = self.alloc((nu * words * 8) as u64);
        let args = Bind::Bytes(bytes_of(&ClosureArgs { n, words: words as u32 }));
        let mut st = vec![
            Stage::one(Step::Zero { buf: r.clone(), words: row32 as usize }),
            Stage::one(Step::Kernel(Dispatch::per(Pipe::ClInit, vec![("off", Bind::Buf(boff)), ("dst", Bind::Buf(bdst)), ("comp", Bind::Buf(bcomp.clone())), ("cyclic", Bind::Buf(bcyc)), ("R", Bind::Buf(r.clone())), ("args", args.clone())], n as u64))),
        ];
        // Level 0 has no successors: its rows are final after the init.
        for l in 1..levels {
            let (lo, hi) = (lstart[l], lstart[l + 1]);
            let level = Bind::Bytes(bytes_of(&LevelArgs { lo, hi, words: words as u32 }));
            let d = Dispatch::new(Pipe::ClLevel, vec![("R", Bind::Buf(r.clone())), ("order", Bind::Buf(border.clone())), ("soff", Bind::Buf(bsoff.clone())), ("sdst", Bind::Buf(bsdst.clone())), ("args", level)], ((hi - lo) as u64).div_ceil(TG / 32));
            st.push(Stage::one(Step::Kernel(d)));
        }
        let total = (nu * words) as u64;
        st.push(Stage::one(Step::Kernel(Dispatch::per(Pipe::ClGather, vec![("R", Bind::Buf(r)), ("comp", Bind::Buf(bcomp)), ("out", Bind::Buf(out.clone())), ("args", args)], total))));
        Ok(Job { stages: st, read: Read::Bits { rows: nu, words, buf: out }, bad: None })
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod bench;
