//! The gpu lane (decision 0098 §5, 0095 §3): the four kernels in Metal compute, a batch as **one command buffer**.
//!
//! A submission plans each launch as a list of **stages**: dispatches that read only the launch's inputs and what
//! its earlier stages wrote. All launches' stage 0 dispatches are encoded first, then one memory barrier over the
//! buffers they wrote, then every launch's stage 1, and so on, in one concurrent compute encoder. So independent
//! launches overlap on the device, a barrier stands only where a later dispatch reads an earlier one's output, and
//! the host waits once.
//!
//! The kernels (`*.metal`, compiled once per process):
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
//! counts, so it is page-aligned only when the caller made it so). Outputs are counted, then scattered: deterministic, and equal to [`crate::cpu::run`]'s. A pairs output is
//! written into a buffer of `min(bound, PAIRS_CAP)` pairs; when the device's total is larger, its scatter runs again
//! in a second command buffer into a buffer of the exact size (the counts and offsets stay on the device).

use crate::exec::Backend;
use crate::kernel::{Bits, Kernel, Output};
use crate::lanes::Lane;
use metal::{Buffer, CommandQueue, CompileOptions, ComputePipelineState, Device, MTLDispatchType, MTLResourceOptions, MTLSize};
use objc::{msg_send, sel, sel_impl};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

pub mod shared;

const SOURCE: &str = concat!(include_str!("common.metal"), include_str!("bits.metal"), include_str!("table.metal"), include_str!("closure.metal"));

/// Threads per threadgroup of every kernel (the bit product's as 16 × 16).
const TG: u64 = 256;
/// Elements per threadgroup of the device-wide scan (common.metal's `SCAN_BLOCK`).
const SCAN_BLOCK: u64 = 1024;
/// Pairs a first pass writes at most before the exact rerun (2 MB). A larger first buffer is not free: with 32 MB,
/// a submission's wall time grew by about 50 ms (the pages' preparation), more than a second command buffer's
/// round trip (0.2 to 1.5 ms).
const PAIRS_CAP: u64 = 1 << 18;

#[derive(Clone, Copy, Debug)]
enum Pipe {
    ScanBlock,
    ScanAdd,
    Zero,
    BpTile,
    BpScatter,
    CtCount,
    CtScatter,
    Hist,
    ClInit,
    ClLevel,
    ClGather,
}

const PIPES: [&str; 11] = ["scan_block", "scan_add", "zero_u32", "bp_tile", "bp_scatter", "ct_count", "ct_scatter", "hist", "cl_init", "cl_level", "cl_gather"];

/// The device, its queue and the compiled kernels: once per process.
struct Gpu {
    device: Device,
    queue: CommandQueue,
    pipes: Vec<ComputePipelineState>,
    page: usize,
    max_buffer: u64,
}

impl Gpu {
    fn new() -> Result<Gpu, String> {
        objc::rc::autoreleasepool(|| {
            // The device and queue every relational submission shares (decision 0100 §2).
            let shared = shared::get().ok_or("no Metal device")?;
            let device = shared.device.clone();
            let library = device.new_library_with_source(SOURCE, &CompileOptions::new()).map_err(|e| format!("gpu lane: compiling the kernels: {e}"))?;
            let mut pipes = Vec::with_capacity(PIPES.len());
            for name in PIPES {
                let f = library.get_function(name, None).map_err(|e| format!("gpu lane: {name}: {e}"))?;
                let p = device.new_compute_pipeline_state_with_function(&f).map_err(|e| format!("gpu lane: {name}: {e}"))?;
                if p.max_total_threads_per_threadgroup() < TG {
                    return Err(format!("gpu lane: {name} runs {} threads a threadgroup, not {TG}", p.max_total_threads_per_threadgroup()));
                }
                pipes.push(p);
            }
            Ok(Gpu { device, queue: shared.queue.clone(), pipes, page: shared.page, max_buffer: shared.max_buffer })
        })
    }
}

fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
    GPU.get_or_init(|| Gpu::new().map_err(|e| eprintln!("{e}")).ok()).as_ref()
}

/// This lane's backend, when this machine has a Metal device and the kernels compiled.
pub fn backend() -> Option<Arc<dyn Backend>> {
    gpu().map(|gpu| Arc::new(GpuBackend { gpu }) as Arc<dyn Backend>)
}

/// The gpu lane.
pub struct GpuBackend {
    gpu: &'static Gpu,
}

/// What a submission took on the device.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    /// Device seconds, `GPUEndTime - GPUStartTime`, summed over the command buffers.
    pub device: f64,
    /// Command buffers: 1, or 2 when a pairs output outgrew its first buffer.
    pub command_buffers: usize,
    pub dispatches: usize,
    /// Memory barriers in the encoder.
    pub barriers: usize,
}

/// The gpu lane's backend, as its own type (the timing test reads [`Stats`]).
pub fn gpu_backend() -> Option<GpuBackend> {
    gpu().map(|gpu| GpuBackend { gpu })
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
        }
    }

    fn submit(&self, ks: &[&Kernel]) -> Result<Vec<Output>, String> {
        self.run(ks).map(|(o, _)| o)
    }
}

impl GpuBackend {
    /// Runs `ks` as one submission, with what it took on the device.
    pub fn run(&self, ks: &[&Kernel]) -> Result<(Vec<Output>, Stats), String> {
        objc::rc::autoreleasepool(|| {
            let mut plan = Planner { gpu: self.gpu, inputs: HashMap::new(), leases: Default::default() };
            let jobs: Vec<Job> = ks.iter().map(|k| plan.job(k)).collect::<Result<_, _>>()?;
            let mut stats = Stats::default();
            self.execute(jobs.iter().map(|j| j.stages.as_slice()), &mut stats)?;
            // Pairs that outgrew their first buffer: their scatter again, into the exact size.
            let mut rerun: Vec<Dispatch> = Vec::new();
            let mut exact: HashMap<usize, (Buffer, u64)> = HashMap::new();
            for (i, j) in jobs.iter().enumerate() {
                if let Read::Pairs { total, cap, rescatter, .. } = &j.read {
                    let t = read::<u32>(total, 1)[0] as u64;
                    if t > *cap {
                        if t * 8 > self.gpu.max_buffer {
                            return Err(format!("launch {i}: {t} pairs do not fit a buffer"));
                        }
                        let buf = plan.alloc(t * 8);
                        let (d, bi, ci) = rescatter;
                        let mut d = d.clone();
                        d.set(*bi, Bind::Buf(buf.clone()));
                        d.set(*ci, Bind::U32(t as u32));
                        rerun.push(d);
                        exact.insert(i, (buf, t));
                    }
                }
            }
            if !rerun.is_empty() {
                let stage = [Stage { ds: rerun, writes: Vec::new() }];
                self.execute(std::iter::once(&stage[..]), &mut stats)?;
            }
            let mut outs = Vec::with_capacity(jobs.len());
            for (i, j) in jobs.into_iter().enumerate() {
                if let Some(bad) = &j.bad {
                    if read::<u32>(bad, 1)[0] != 0 {
                        return Err(format!("launch {i} ({}): a code is not below the width", ks[i].form().name()));
                    }
                }
                outs.push(match j.read {
                    Read::Ready(o) => o,
                    Read::Counts { rows, cols, buf } => Output::Counts { rows, cols, data: read::<u32>(&buf, rows * cols) },
                    Read::PerRow { n, buf } => Output::PerRow(read::<u32>(&buf, n)),
                    Read::Bits { rows, words, buf } => Output::Bits(Bits::new(rows, words, read::<u64>(&buf, rows * words))),
                    Read::Pairs { total, buf, .. } => {
                        let t = read::<u32>(&total, 1)[0] as usize;
                        let buf = exact.remove(&i).map_or(buf, |(b, _)| b);
                        Output::Pairs(read::<(u32, u32)>(&buf, t))
                    }
                });
            }
            Ok((outs, stats))
        })
    }

    /// One command buffer: every job's stage `s`, then a barrier over what stage `s` wrote, then stage `s + 1`.
    #[allow(unexpected_cfgs)] // `msg_send!` expands to a `cargo-clippy` cfg check.
    fn execute<'a>(&self, jobs: impl Iterator<Item = &'a [Stage]> + Clone, stats: &mut Stats) -> Result<(), String> {
        let gpu = self.gpu;
        let cb = gpu.queue.new_command_buffer();
        let enc = cb.compute_command_encoder_with_dispatch_type(MTLDispatchType::Concurrent);
        let depth = jobs.clone().map(|j| j.len()).max().unwrap_or(0);
        let mut written: Vec<&Buffer> = Vec::new();
        for s in 0..depth {
            if !written.is_empty() {
                let rs: Vec<&metal::ResourceRef> = written.iter().map(|b| -> &metal::ResourceRef { b }).collect();
                enc.memory_barrier_with_resources(&rs);
                stats.barriers += 1;
                written.clear();
            }
            for stage in jobs.clone().filter_map(|j| j.get(s)) {
                for d in &stage.ds {
                    if d.groups.width * d.groups.height * d.groups.depth == 0 {
                        continue;
                    }
                    enc.set_compute_pipeline_state(&gpu.pipes[d.pipe as usize]);
                    for (i, b) in &d.binds {
                        match b {
                            Bind::Buf(buf) => enc.set_buffer(*i, Some(buf), 0),
                            Bind::U32(v) => enc.set_bytes(*i, 4, (v as *const u32).cast()),
                        }
                    }
                    enc.dispatch_thread_groups(d.groups, d.threads);
                    stats.dispatches += 1;
                }
                written.extend(stage.writes.iter());
            }
        }
        enc.end_encoding();
        cb.commit();
        cb.wait_until_completed();
        if cb.status() != metal::MTLCommandBufferStatus::Completed {
            return Err(format!("the command buffer ended {:?}", cb.status()));
        }
        let (start, end): (f64, f64) = unsafe { (msg_send![cb, GPUStartTime], msg_send![cb, GPUEndTime]) };
        stats.device += (end - start).max(0.0);
        stats.command_buffers += 1;
        Ok(())
    }
}

// The device writes pairs as `uint2`; `(u32, u32)` must have that layout for `read` to copy them as they are.
const _: () = assert!(size_of::<(u32, u32)>() == 8 && std::mem::offset_of!((u32, u32), 0) == 0 && std::mem::offset_of!((u32, u32), 1) == 4);

/// `n` values from the start of a shared buffer.
fn read<T: Copy>(buf: &Buffer, n: usize) -> Vec<T> {
    if n == 0 {
        return Vec::new();
    }
    assert!(buf.length() as usize >= n * size_of::<T>(), "gpu lane: reading past a buffer");
    // SAFETY: a shared buffer the device finished writing (the command buffer completed); `n` values fit it.
    unsafe { std::slice::from_raw_parts(buf.contents() as *const T, n) }.to_vec()
}

#[derive(Clone)]
enum Bind {
    Buf(Buffer),
    U32(u32),
}

#[derive(Clone)]
struct Dispatch {
    pipe: Pipe,
    binds: Vec<(u64, Bind)>,
    groups: MTLSize,
    threads: MTLSize,
}

impl Dispatch {
    fn new(pipe: Pipe, binds: Vec<Bind>, groups: u64) -> Self {
        Dispatch { pipe, binds: binds.into_iter().enumerate().map(|(i, b)| (i as u64, b)).collect(), groups: MTLSize::new(groups, 1, 1), threads: MTLSize::new(TG, 1, 1) }
    }

    /// One thread per element of `n`.
    fn per(pipe: Pipe, binds: Vec<Bind>, n: u64) -> Self {
        Dispatch::new(pipe, binds, n.div_ceil(TG))
    }

    fn set(&mut self, index: u64, b: Bind) {
        let slot = self.binds.iter_mut().find(|(i, _)| *i == index).expect("a bound index");
        slot.1 = b;
    }
}

/// Dispatches that may run together, and the buffers they write (for the barrier before the next stage).
struct Stage {
    ds: Vec<Dispatch>,
    writes: Vec<Buffer>,
}

impl Stage {
    fn one(d: Dispatch, writes: &[&Buffer]) -> Self {
        Stage { ds: vec![d], writes: writes.iter().map(|b| (*b).clone()).collect() }
    }
}

/// How a launch's output is read back after the wait.
enum Read {
    Ready(Output),
    Counts { rows: usize, cols: usize, buf: Buffer },
    PerRow { n: usize, buf: Buffer },
    Bits { rows: usize, words: usize, buf: Buffer },
    /// `total` holds the pairs' count; `buf` the first `cap` of them. `rescatter` is the scatter, with the indexes
    /// of its pairs buffer and its capacity.
    Pairs { total: Buffer, buf: Buffer, cap: u64, rescatter: (Dispatch, u64, u64) },
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
struct Planner {
    gpu: &'static Gpu,
    inputs: HashMap<(usize, usize), Buffer>,
    /// The pooled buffers the submission's jobs use, returned once its command buffers have completed and its
    /// outputs are read (when the planner drops).
    leases: std::cell::RefCell<Vec<shared::Pooled>>,
}

fn u(x: usize) -> u32 {
    x as u32
}

impl Planner {
    /// A zeroed buffer of the pool (the kernels count into some, and read a new buffer as zero).
    fn alloc(&self, bytes: u64) -> Buffer {
        let lease = shared::lease_zeroed(bytes.max(16) as usize).unwrap_or_else(|e| panic!("gpu lane: {e}"));
        let b = lease.buffer().clone();
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
        let b = if bytes != 0 && ptr % self.gpu.page == 0 && bytes % self.gpu.page == 0 {
            self.gpu.device.new_buffer_with_bytes_no_copy(ptr as *const _, bytes as u64, MTLResourceOptions::StorageModeShared, None)
        } else {
            self.upload(data)
        };
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
        // SAFETY: the lease is the planner's alone and at least `bytes` long; no command uses it yet.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr().cast::<u8>(), lease.host(), bytes) };
        let b = lease.buffer().clone();
        self.leases.borrow_mut().push(lease);
        b
    }

    /// The device-wide exclusive scan of `n` u32 (their popcounts when `pop`): stages appended to `st`. Returns the
    /// scanned values and a buffer whose first u32 is the total.
    fn scan(&self, st: &mut Vec<Stage>, input: &Buffer, n: u64, pop: bool) -> (Buffer, Buffer) {
        let blocks = n.div_ceil(SCAN_BLOCK).max(1);
        let (ex, sums) = (self.alloc(n * 4), self.alloc(blocks * 4));
        let d = Dispatch::new(Pipe::ScanBlock, vec![Bind::Buf(input.clone()), Bind::Buf(ex.clone()), Bind::Buf(sums.clone()), Bind::U32(n as u32), Bind::U32(pop as u32)], blocks);
        st.push(Stage::one(d, &[&ex, &sums]));
        if blocks == 1 {
            return (ex, sums);
        }
        let (prefix, total) = self.scan(st, &sums, blocks, false);
        st.push(Stage::one(Dispatch::per(Pipe::ScanAdd, vec![Bind::Buf(ex.clone()), Bind::Buf(prefix), Bind::U32(n as u32)], n), &[&ex]));
        (ex, total)
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
        let mut d = Dispatch::new(Pipe::BpTile, Vec::new(), 0);
        d.groups = MTLSize::new(m.div_ceil(64), n.div_ceil(64), 1);
        d.threads = MTLSize::new(16, 16, 1);
        let dummy = self.alloc(16);
        let mw = m.div_ceil(32);
        let bind = |c: &Buffer, mask: &Buffer, minb: &Buffer, mode: u32| {
            vec![Bind::Buf(ba.clone()), Bind::Buf(bb.clone()), Bind::Buf(c.clone()), Bind::Buf(mask.clone()), Bind::Buf(minb.clone()), Bind::U32(u(a.rows)), Bind::U32(u(b.rows)), Bind::U32(kw as u32), Bind::U32(mode), Bind::U32(mw as u32)]
                .into_iter()
                .enumerate()
                .map(|(i, b)| (i as u64, b))
                .collect()
        };
        let Some(min) = min else {
            let c = self.alloc(n * m * 4);
            d.binds = bind(&c, &dummy, &dummy, 0);
            return Ok(Job { stages: vec![Stage::one(d, &[&c])], read: Read::Counts { rows: a.rows, cols: b.rows, buf: c }, bad: None });
        };
        let nw = n * mw;
        let mask = self.alloc(nw * 4);
        let minb = self.input(min);
        d.binds = bind(&dummy, &mask, &minb, 1);
        let mut st = vec![Stage::one(d, &[&mask])];
        let (ex, total) = self.scan(&mut st, &mask, nw, true);
        let cap = (n * m).min(PAIRS_CAP);
        let pairs = self.alloc(cap * 8);
        let scatter = Dispatch::per(Pipe::BpScatter, vec![Bind::Buf(mask.clone()), Bind::Buf(ex), Bind::Buf(pairs.clone()), Bind::U32(cap as u32), Bind::U32(mw as u32), Bind::U32(nw as u32)], nw);
        st.push(Stage::one(scatter.clone(), &[&pairs]));
        Ok(Job { stages: st, read: Read::Pairs { total, buf: pairs, cap, rescatter: (scatter, 2, 3) }, bad: None })
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
        let count = Dispatch::new(Pipe::CtCount, vec![Bind::Buf(bl.clone()), Bind::Buf(br.clone()), Bind::Buf(blo.clone()), Bind::Buf(bro.clone()), Bind::Buf(counts.clone())], nparts);
        let mut st = vec![Stage::one(count, &[&counts])];
        if !emit {
            return Ok(Job { stages: st, read: Read::PerRow { n: left.len(), buf: counts }, bad: None });
        }
        let (ex, total) = self.scan(&mut st, &counts, nl, false);
        let bound: u64 = loff.windows(2).zip(roff.windows(2)).map(|(l, r)| (l[1] - l[0]) as u64 * (r[1] - r[0]) as u64).sum();
        let cap = bound.min(PAIRS_CAP);
        let pairs = self.alloc(cap * 8);
        let scatter = Dispatch::new(
            Pipe::CtScatter,
            vec![Bind::Buf(bl), Bind::Buf(br), Bind::Buf(blo), Bind::Buf(bro), Bind::Buf(counts), Bind::Buf(ex), Bind::Buf(pairs.clone()), Bind::U32(cap as u32)],
            nparts,
        );
        st.push(Stage::one(scatter.clone(), &[&pairs]));
        Ok(Job { stages: st, read: Read::Pairs { total, buf: pairs, cap, rescatter: (scatter, 6, 7) }, bad: None })
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
        let d = Dispatch::new(Pipe::Hist, vec![Bind::Buf(bc), Bind::Buf(bo), Bind::Buf(h.clone()), Bind::U32(width), Bind::Buf(bad.clone())], nparts as u64);
        Ok(Job { stages: vec![Stage::one(d, &[&h, &bad])], read: Read::Counts { rows: nparts, cols: width as usize, buf: h }, bad: Some(bad) })
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
        let (comp, ncomp) = crate::cpu::components(nu, off, dst);
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
        let w = Bind::U32(words as u32);
        let mut st = vec![
            Stage::one(Dispatch::per(Pipe::Zero, vec![Bind::Buf(r.clone()), Bind::U32(row32 as u32)], row32), &[&r]),
            Stage::one(Dispatch::per(Pipe::ClInit, vec![Bind::Buf(boff), Bind::Buf(bdst), Bind::Buf(bcomp.clone()), Bind::Buf(bcyc), Bind::Buf(r.clone()), Bind::U32(n), w.clone()], n as u64), &[&r]),
        ];
        // Level 0 has no successors: its rows are final after the init.
        for l in 1..levels {
            let (lo, hi) = (lstart[l], lstart[l + 1]);
            let d = Dispatch::new(Pipe::ClLevel, vec![Bind::Buf(r.clone()), Bind::Buf(border.clone()), Bind::Buf(bsoff.clone()), Bind::Buf(bsdst.clone()), Bind::U32(lo), Bind::U32(hi), w.clone()], ((hi - lo) as u64).div_ceil(TG / 32));
            st.push(Stage::one(d, &[&r]));
        }
        let total = (nu * words) as u64;
        st.push(Stage::one(Dispatch::per(Pipe::ClGather, vec![Bind::Buf(r), Bind::Buf(bcomp), Bind::Buf(out.clone()), Bind::U32(n), w], total), &[&out]));
        Ok(Job { stages: st, read: Read::Bits { rows: nu, words, buf: out }, bad: None })
    }
}

#[cfg(test)]
mod tests;
