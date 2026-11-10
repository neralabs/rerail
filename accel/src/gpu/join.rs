//! Relations sorted by a key on the device, and the equi-joins that probe them (decision 0095 §4, §5): the GPU
//! lane's `join` kernel, moved into the executor from the engine (decision 0100 §1).
//!
//! **A column on the device** (`Column`) is each value's offset from the column's least value, in 16, 32 or 64
//! bits: as narrow as the column's span lets it be, exactly. The host's part is one pass for the span and one that
//! writes the offsets straight into the shared buffer (`rows_sort::span`); nothing is sorted or copied twice.
//!
//! **A relation sorted by its key** (`Keyed`), all of it on the device:
//! - `sort_init`, then per 8-bit digit of the key's span `sort_hist`, a scan, and `sort_scatter`: a stable LSD radix
//!   sort of 1,024-row tiles. A tile's digit histogram is counted in threadgroup memory; a row's place is its digit's
//!   offset for the tile (the scan of every tile's histogram, digit-major), the rows of its digit in the tile's
//!   earlier rounds and simdgroups, and its rank among its simdgroup's earlier lanes of the digit (a bounded loop of
//!   32 shuffles). No atomic decides an order.
//! - `run_heads`, a scan, `run_compact`: the key's runs, each its first place and key, and their count.
//! - `gather`: every other column in the sorted order, so a run's rows are adjacent in every column.
//!
//! A `Keyed` stays on the device for as long as the host holds it: every join that probes the relation by that key
//! (in a stratum, and in the next strata of a chunk while the relation does not change) uses one sort.
//!
//! **The probe join** (`probe`): a thread per left row, unsorted, finds its key's run by a bounded binary search of
//! the runs' keys, and loops over the run's rows comparing the other keys. Count, scan, scatter: the pairs in left
//! row order, sized exactly. The work is the left rows' searches and the pairs of key 0.
//!
//! Every scan is the same multi-level exclusive scan (`Scan`): 1,024 values a threadgroup, the threadgroups' sums
//! scanned again, added back. Threadgroup memory: 10 kB at most (the scatter's counts). Every loop is bounded.

use super::shared;
use crate::rows::{bits, span};
use metal::{Buffer, BufferRef, CommandBufferRef, CommandQueue, ComputeCommandEncoderRef, ComputePipelineState, MTLSize};
use std::cell::{Cell, RefCell};

/// Rows a threadgroup takes in the sort and the scans: 256 threads, four rows each.
const TILE: usize = 1024;

/// A column's width on the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    W16,
    W32,
    W64,
}

impl Width {
    fn of(bits: u32) -> Width {
        match bits {
            0..=16 => Width::W16,
            17..=32 => Width::W32,
            _ => Width::W64,
        }
    }
    fn ty(self) -> &'static str {
        match self {
            Width::W16 => "ushort",
            Width::W32 => "uint",
            Width::W64 => "ulong",
        }
    }
    pub fn bytes(self) -> usize {
        match self {
            Width::W16 => 2,
            Width::W32 => 4,
            Width::W64 => 8,
        }
    }
    /// The word a sort's key takes: 32 bits, or 64.
    fn key(self) -> Width {
        if self == Width::W64 { Width::W64 } else { Width::W32 }
    }
}

/// A column on the device: each row's offset from `base`, in `width`.
pub struct Column {
    buffer: Held,
    pub width: Width,
    pub base: i64,
    /// The bits of the column's span.
    pub bits: u32,
    pub rows: usize,
}

pub fn device() -> Result<&'static metal::Device, String> {
    shared::get().map(|s| &s.device).ok_or_else(|| "no Metal device".to_owned())
}

/// The one queue of relational work (decision 0100 §2), shared with the executor's GPU lane.
pub fn queue() -> Result<CommandQueue, String> {
    Ok(shared::get().ok_or("no Metal device")?.queue.clone())
}

/// A completed command buffer's time on the GPU, in seconds (`GPUEndTime - GPUStartTime`).
#[allow(unexpected_cfgs)]
pub fn gpu_seconds(command: &CommandBufferRef) -> f64 {
    use objc::{msg_send, sel, sel_impl};
    let (start, end): (f64, f64) = unsafe { (msg_send![command, GPUStartTime], msg_send![command, GPUEndTime]) };
    (end - start).max(0.0)
}

/// Compiles `name` from `source` on the shared device.
pub fn compile(source: &str, name: &str) -> Result<ComputePipelineState, String> {
    let device = device()?;
    objc::rc::autoreleasepool(|| {
        let library = device.new_library_with_source(source, &metal::CompileOptions::new()).map_err(|e| format!("compiling {name}: {e}"))?;
        let f = library.get_function(name, None).map_err(|e| format!("{name}: {e}"))?;
        device.new_compute_pipeline_state_with_function(&f).map_err(|e| format!("{name}: {e}"))
    })
}

thread_local! {
    /// This thread's scratch buffers, by slot: a sort's or probe's intermediates, kept for the next (each call
    /// completes its command buffers before it returns, so no two calls share one in flight).
    static SCRATCH: RefCell<Vec<Option<shared::Pooled>>> = const { RefCell::new(Vec::new()) };
}

/// The pipeline `name` of the variant `variant`, its source made by `source` the first time: kept by the one
/// manager (decision 0100 §2), so every thread finds it compiled.
pub fn pipeline(variant: &str, name: &str, source: impl FnOnce() -> String) -> Result<ComputePipelineState, String> {
    shared::pipeline(&format!("join {variant} {name}"), || compile(&source(), name))
}

/// How a call waits for its command buffer: Metal's `waitUntilCompleted`, or a completion handler the thread
/// blocks on (it sleeps until the GPU is done).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    Metal,
    Handler,
}

thread_local! {
    static WAIT: Cell<Wait> = const { Cell::new(Wait::Handler) };
}

/// Sets how this thread's calls wait; the previous way.
pub fn set_wait(w: Wait) -> Wait {
    WAIT.with(|c| c.replace(w))
}

/// A committed command buffer, to wait for, and the pooled buffers it reads that nothing else holds: a pooled
/// buffer is written by the host when handed out again, and Metal orders no host write after a GPU read, so they
/// return to the pool only once it completes.
pub struct Submitted {
    command: metal::CommandBuffer,
    done: Option<std::sync::mpsc::Receiver<()>>,
    reads: Vec<Column>,
}

impl Submitted {
    /// Commits `command` (checked against the launch budget), with a completion handler when this thread waits on one.
    pub fn commit(command: &CommandBufferRef) -> Result<Submitted, String> {
        let done = (WAIT.with(Cell::get) == Wait::Handler).then(|| {
            let (tx, rx) = std::sync::mpsc::sync_channel::<()>(1);
            let block = block::ConcreteBlock::new(move |_: &CommandBufferRef| {
                let _ = tx.try_send(());
            })
            .copy();
            command.add_completed_handler(&block);
            rx
        });
        shared::guard()?;
        command.commit();
        Ok(Submitted { command: command.to_owned(), done, reads: Vec::new() })
    }

    /// Waits for it; its GPU seconds. The buffers it read return to the pool after.
    pub fn wait(self) -> f64 {
        match self.done {
            Some(rx) if self.command.status() != metal::MTLCommandBufferStatus::Completed => {
                let _ = rx.recv();
                // The handler runs as the buffer completes: its status may lag a moment.
                self.command.wait_until_completed();
            }
            _ => self.command.wait_until_completed(),
        }
        gpu_seconds(&self.command)
    }
}

/// Scratch slots.
const KEYS_A: usize = 0;
const KEYS_B: usize = 1;
const IDX: usize = 2;
const HIST: usize = 3;
const HEADS: usize = 4;
/// A scan's levels: slots `SCAN..`.
const SCAN: usize = 5;
/// The first slot free for other modules' scratch (the engine's device stages): past the scan's levels.
pub const FREE: usize = 16;

/// Scratch slot `slot`, at least `bytes` long (grown to a power of two when it is shorter).
pub fn scratch(slot: usize, bytes: usize) -> Result<Buffer, String> {
    SCRATCH.with_borrow_mut(|s| {
        if s.len() <= slot {
            s.resize_with(slot + 1, || None);
        }
        if s[slot].as_ref().is_none_or(|b| (b.length() as usize) < bytes) {
            // A command buffer not yet completed may read the shorter one: it goes to Metal, which holds it until
            // then, not to the pool, whose next lease may be written by the host at once.
            if let Some(old) = s[slot].take() {
                old.release();
            }
            s[slot] = Some(shared::lease_zeroed(bytes.max(64))?);
        }
        Ok(s[slot].as_ref().expect("a scratch buffer").buffer().clone())
    })
}

/// A device buffer of the one manager's pool (decision 0100 §2): a free one of its size class when there is one,
/// returned when dropped. A buffer Metal has seen is resident already: a new one costs its creation and its mapping
/// into the GPU's space at its first commit, a pooled one neither. Its holder drops it only once every command
/// buffer that reads it has completed (`Submitted::reads`, `Keyed`'s drop): the next lease's host writes are not
/// ordered after a GPU read.
pub struct Held {
    _lease: shared::Pooled,
    pub buffer: Buffer,
}

impl Held {
    /// A buffer of at least `bytes` (its contents are a previous user's: every reader writes first).
    pub fn new(bytes: usize) -> Result<Held, String> {
        device()?;
        let lease = shared::lease(bytes)?;
        let buffer = lease.buffer().clone();
        Ok(Held { _lease: lease, buffer })
    }

    /// A buffer holding `values`, written by the host.
    pub fn of<T: Copy>(values: impl ExactSizeIterator<Item = T>) -> Result<Held, String> {
        let n = values.len();
        let held = Held::new(n * std::mem::size_of::<T>())?;
        // SAFETY: the buffer holds at least `n` values of `T`, and nothing else refers to it now.
        let out = unsafe { std::slice::from_raw_parts_mut(held.buffer.contents().cast::<T>(), n) };
        for (o, v) in out.iter_mut().zip(values) {
            *o = v;
        }
        Ok(held)
    }
}

/// One compute encoder over a command buffer: its dispatches run in order (a serial encoder), each seeing the last's
/// writes.
pub struct Pass<'a> {
    encoder: &'a ComputeCommandEncoderRef,
    /// The dispatches encoded so far (a command buffer's launch budget counts them).
    dispatches: Cell<usize>,
}

impl<'a> Pass<'a> {
    pub fn new(command: &'a CommandBufferRef) -> Pass<'a> {
        let encoder = command.new_compute_command_encoder();
        // Named: the launch budget's record reads the encoder's label, and an unnamed one's is nil.
        encoder.set_label("rows");
        Pass { encoder, dispatches: Cell::new(0) }
    }

    /// The dispatches encoded so far.
    pub fn dispatches(&self) -> usize {
        self.dispatches.get()
    }

    /// `groups` threadgroups of 256 threads over `buffers`, then `bytes` (a parameter block, as `constant T&`) after
    /// them.
    pub fn run_bytes(&self, pipeline: &ComputePipelineState, buffers: &[&BufferRef], bytes: &[u8], groups: usize) {
        self.encoder.set_compute_pipeline_state(pipeline);
        for (i, b) in buffers.iter().enumerate() {
            self.encoder.set_buffer(i as u64, Some(b), 0);
        }
        self.encoder.set_bytes(buffers.len() as u64, bytes.len().max(4) as u64, if bytes.is_empty() { [0u32].as_ptr().cast() } else { bytes.as_ptr().cast() });
        self.encoder.dispatch_thread_groups(MTLSize::new(groups.max(1) as u64, 1, 1), MTLSize::new(256, 1, 1));
        self.dispatches.set(self.dispatches.get() + 1);
    }

    /// `groups` threadgroups of 256 threads over `buffers`, then `params` as `constant uint*` and `longs` as
    /// `constant long*` after them.
    pub fn run(&self, pipeline: &ComputePipelineState, buffers: &[&BufferRef], params: &[u32], longs: &[i64], groups: usize) {
        self.encoder.set_compute_pipeline_state(pipeline);
        for (i, b) in buffers.iter().enumerate() {
            self.encoder.set_buffer(i as u64, Some(b), 0);
        }
        let at = buffers.len() as u64;
        self.encoder.set_bytes(at, std::mem::size_of_val(params).max(4) as u64, if params.is_empty() { [0u32].as_ptr().cast() } else { params.as_ptr().cast() });
        if !longs.is_empty() {
            self.encoder.set_bytes(at + 1, std::mem::size_of_val(longs) as u64, longs.as_ptr().cast());
        }
        self.encoder.dispatch_thread_groups(MTLSize::new(groups.max(1) as u64, 1, 1), MTLSize::new(256, 1, 1));
        self.dispatches.set(self.dispatches.get() + 1);
    }

    pub fn end(self) {
        self.encoder.end_encoding();
    }
}

/// `values` as offsets from their least in `width`, written into `out` (as many as `values`).
fn pack(values: &[i64], base: i64, width: Width, out: *mut u8) {
    let n = values.len();
    // SAFETY: `out` holds `n` values of `width` (the caller's buffer), and nothing else refers to it yet.
    unsafe {
        match width {
            Width::W16 => {
                let out = std::slice::from_raw_parts_mut(out.cast::<u16>(), n);
                for (o, &v) in out.iter_mut().zip(values) {
                    *o = v.wrapping_sub(base) as u16;
                }
            }
            Width::W32 => {
                let out = std::slice::from_raw_parts_mut(out.cast::<u32>(), n);
                for (o, &v) in out.iter_mut().zip(values) {
                    *o = v.wrapping_sub(base) as u32;
                }
            }
            Width::W64 => {
                let out = std::slice::from_raw_parts_mut(out.cast::<u64>(), n);
                for (o, &v) in out.iter_mut().zip(values) {
                    *o = v.wrapping_sub(base) as u64;
                }
            }
        }
    }
}

impl Column {
    /// `values` on the device, as narrow as their span lets them be.
    pub fn upload(values: &[i64]) -> Result<Column, String> {
        let (min, max) = span(values).unwrap_or((0, 0));
        let bits = bits(min, max);
        let width = Width::of(bits);
        let buffer = Held::new(values.len() * width.bytes())?;
        pack(values, min, width, buffer.buffer.contents().cast::<u8>());
        Ok(Column { buffer, width, base: min, bits, rows: values.len() })
    }

    /// The bytes the column takes on the device.
    pub fn bytes(&self) -> usize {
        self.rows * self.width.bytes()
    }
}

/// A multi-level exclusive scan of `count` `uint`s in place, its levels' sums in scratch; the total after it in
/// `total()`.
pub struct Scan {
    count: usize,
    /// Each level's threadgroups and their sums.
    levels: Vec<(usize, Buffer)>,
    /// The levels' buffers when they are the scan's own (`owned`), not this thread's scratch.
    _owned: Vec<Held>,
}

const SCAN_SOURCE: &str = r#"#include <metal_stdlib>
using namespace metal;

// 1,024 values a threadgroup, four a thread: each value its exclusive prefix in the block, the block's sum out.
kernel void scan_block(device uint* data [[buffer(0)]], device uint* sums [[buffer(1)]], constant uint* p [[buffer(2)]],
                       uint tg [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]],
                       uint lane [[thread_index_in_simdgroup]], uint sg [[simdgroup_index_in_threadgroup]]) {
    uint n = p[0];
    uint at = tg * 1024 + t * 4;
    uint v[4];
    uint mine = 0;
    for (uint i = 0; i < 4; i++) { v[i] = at + i < n ? data[at + i] : 0; mine += v[i]; }
    uint inc = simd_prefix_inclusive_sum(mine);
    threadgroup uint part[8];
    if (lane == 31) part[sg] = inc;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint off = 0;
    for (uint k = 0; k < sg; k++) off += part[k];
    uint run = off + inc - mine;
    for (uint i = 0; i < 4; i++) { if (at + i < n) data[at + i] = run; run += v[i]; }
    if (t == 255) sums[tg] = off + inc;
}

// Each value of a block plus its block's prefix.
kernel void scan_add(device uint* data [[buffer(0)]], device const uint* sums [[buffer(1)]], constant uint* p [[buffer(2)]],
                     uint tg [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]]) {
    uint n = p[0];
    uint at = tg * 1024 + t * 4;
    uint s = sums[tg];
    for (uint i = 0; i < 4; i++) { if (at + i < n) data[at + i] += s; }
}
"#;

impl Scan {
    pub fn new(count: usize) -> Result<Scan, String> {
        let mut levels = Vec::new();
        let mut size = count;
        loop {
            let groups = size.div_ceil(TILE).max(1);
            levels.push((groups, scratch(SCAN + levels.len(), 4 * groups)?));
            if groups == 1 {
                break;
            }
            size = groups;
        }
        Ok(Scan { count, levels, _owned: Vec::new() })
    }

    /// A scan whose levels are its own: several in one command buffer, each total read after it completes.
    fn owned(count: usize) -> Result<Scan, String> {
        let (mut levels, mut owned) = (Vec::new(), Vec::new());
        let mut size = count;
        loop {
            let groups = size.div_ceil(TILE).max(1);
            let h = Held::new(4 * groups)?;
            levels.push((groups, h.buffer.clone()));
            owned.push(h);
            if groups == 1 {
                break;
            }
            size = groups;
        }
        Ok(Scan { count, levels, _owned: owned })
    }

    pub fn encode(&self, pass: &Pass<'_>, data: &BufferRef) -> Result<(), String> {
        let block = pipeline("scan", "scan_block", || SCAN_SOURCE.to_owned())?;
        let add = pipeline("scan", "scan_add", || SCAN_SOURCE.to_owned())?;
        let mut inputs: Vec<(&BufferRef, usize)> = vec![(data, self.count)];
        for (k, (groups, sums)) in self.levels.iter().enumerate() {
            let (buffer, n) = inputs[k];
            pass.run(&block, &[buffer, sums], &[n as u32], &[], *groups);
            inputs.push((sums, *groups));
        }
        // Back down: each level's blocks plus the scanned sums above them.
        for k in (0..self.levels.len().saturating_sub(1)).rev() {
            let (buffer, n) = inputs[k];
            pass.run(&add, &[buffer, &self.levels[k].1], &[n as u32], &[], self.levels[k].0);
        }
        Ok(())
    }

    /// The scan's total, after its command buffer completed.
    fn total(&self) -> u32 {
        // SAFETY: the top level holds one sum, written by its block.
        unsafe { *self.levels.last().expect("a level").1.contents().cast::<u32>() }
    }
}

/// The sort's, runs' and gathers' kernels for a key column of `col` read into sort words `key`.
fn sort_source(col: Width, key: Width) -> String {
    let (t, k) = (col.ty(), key.ty());
    format!(
        r#"#include <metal_stdlib>
using namespace metal;

kernel void sort_init(device const {t}* col [[buffer(0)]], device {k}* keys [[buffer(1)]], device uint* idx [[buffer(2)]], constant uint* p [[buffer(3)]],
                      uint t [[thread_position_in_grid]]) {{
    if (t < p[0]) {{ keys[t] = {k}(col[t]); idx[t] = t; }}
}}

// A tile's digit histogram, digit-major across the tiles: hist[d * tiles + tile].
kernel void sort_hist(device const {k}* keys [[buffer(0)]], device uint* hist [[buffer(1)]], constant uint* p [[buffer(2)]],
                      uint tg [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]], uint tiles [[threadgroups_per_grid]]) {{
    threadgroup atomic_uint h[256];
    atomic_store_explicit(&h[t], 0, memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint i = 0; i < 4; i++) {{
        uint at = tg * 1024 + i * 256 + t;
        if (at < p[0]) atomic_fetch_add_explicit(&h[uint(keys[at] >> p[1]) & 255], 1, memory_order_relaxed);
    }}
    threadgroup_barrier(mem_flags::mem_threadgroup);
    hist[t * tiles + tg] = atomic_load_explicit(&h[t], memory_order_relaxed);
}}

// Each row at its digit's offset for the tile, after the tile's earlier rows of the digit: stable.
kernel void sort_scatter(device const {k}* keys [[buffer(0)]], device const uint* idx [[buffer(1)]], device const uint* offs [[buffer(2)]],
                         device {k}* okeys [[buffer(3)]], device uint* oidx [[buffer(4)]], constant uint* p [[buffer(5)]],
                         uint tg [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]], uint tiles [[threadgroups_per_grid]],
                         uint lane [[thread_index_in_simdgroup]], uint sg [[simdgroup_index_in_threadgroup]]) {{
    threadgroup uint counts[8][256];
    threadgroup uint before[256];
    threadgroup uint base[256];
    base[t] = offs[t * tiles + tg];
    before[t] = 0;
    for (uint i = 0; i < 4; i++) {{
        uint at = tg * 1024 + i * 256 + t;
        bool live = at < p[0];
        {k} key = live ? keys[at] : {k}(0);
        uint d = live ? (uint(key >> p[1]) & 255) : 256;
        uint rank = 0;
        bool last = true;
        for (ushort j = 0; j < 32; j++) {{
            uint dj = simd_shuffle(d, j);
            if (dj == d && j < lane) rank++;
            if (dj == d && j > lane) last = false;
        }}
        for (uint s = 0; s < 8; s++) counts[s][t] = 0;
        threadgroup_barrier(mem_flags::mem_threadgroup);
        if (live && last) counts[sg][d] = rank + 1;
        threadgroup_barrier(mem_flags::mem_threadgroup);
        uint acc = 0;
        for (uint s = 0; s < 8; s++) {{ uint c = counts[s][t]; counts[s][t] = acc; acc += c; }}
        threadgroup_barrier(mem_flags::mem_threadgroup);
        if (live) {{
            uint at2 = base[d] + before[d] + counts[sg][d] + rank;
            okeys[at2] = key;
            oidx[at2] = idx[at];
        }}
        threadgroup_barrier(mem_flags::mem_threadgroup);
        before[t] += acc;
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }}
}}

kernel void run_heads(device const {k}* keys [[buffer(0)]], device uint* heads [[buffer(1)]], constant uint* p [[buffer(2)]],
                      uint t [[thread_position_in_grid]]) {{
    if (t < p[0]) heads[t] = uint(t == 0 || keys[t] != keys[t - 1]);
}}

// Each run's first place and key; the runs' count, and the end of the last.
kernel void run_compact(device const {k}* keys [[buffer(0)]], device const uint* rid [[buffer(1)]], device uint* starts [[buffer(2)]],
                        device {k}* rkeys [[buffer(3)]], device uint* nruns [[buffer(4)]], constant uint* p [[buffer(5)]],
                        uint t [[thread_position_in_grid]]) {{
    uint n = p[0];
    if (t >= n) return;
    bool head = t == 0 || keys[t] != keys[t - 1];
    if (head) {{ starts[rid[t]] = t; rkeys[rid[t]] = keys[t]; }}
    if (t == n - 1) {{ uint r = rid[t] + uint(head); starts[r] = n; nruns[0] = r; }}
}}
"#
    )
}

/// A column gathered into a sorted order.
fn gather_source(col: Width) -> String {
    let t = col.ty();
    format!(
        r#"#include <metal_stdlib>
using namespace metal;
kernel void gather(device const {t}* col [[buffer(0)]], device const uint* perm [[buffer(1)]], device {t}* out [[buffer(2)]], constant uint* p [[buffer(3)]],
                   uint t [[thread_position_in_grid]]) {{
    if (t < p[0]) out[t] = col[perm[t]];
}}
"#
    )
}

/// A relation sorted by its column 0 on the device: its runs, and its other columns in the sorted order.
pub struct Keyed {
    pub rows: usize,
    key: Width,
    base: i64,
    /// Each sorted place's host row.
    perm: Held,
    starts: Held,
    rkeys: Held,
    nruns: Held,
    /// Columns 1.. in the sorted order (column 0 is the runs' keys).
    others: Vec<Column>,
    /// The sort's command buffer while it may still run (`settle`), then its runs and GPU time (as bits).
    pending: std::sync::Mutex<Option<Submitted>>,
    runs: std::sync::atomic::AtomicUsize,
    gpu: std::sync::atomic::AtomicU64,
    pub bytes: usize,
}

pub fn groups(n: usize) -> usize {
    n.div_ceil(256).max(1)
}

impl Drop for Keyed {
    /// A sort still running holds its buffers: they return to the pool once it completes.
    fn drop(&mut self) {
        self.settle();
    }
}

impl Keyed {
    /// Waits for the sort, once (a sort encoded into a caller's command buffer is the caller's to wait for).
    pub fn settle(&self) {
        use std::sync::atomic::Ordering::Relaxed;
        let mut pending = self.pending.lock().expect("a sort's pending command buffer");
        if let Some(s) = pending.take() {
            self.gpu.store(s.wait().to_bits(), Relaxed);
        }
        if self.runs.load(Relaxed) == usize::MAX {
            // SAFETY: one word, written by `run_compact` (none when there are no rows), its command buffer completed.
            self.runs.store(if self.rows > 0 { (unsafe { *self.nruns.buffer.contents().cast::<u32>() }) as usize } else { 0 }, Relaxed);
        }
    }

    /// The key's runs (after the sort completes).
    pub fn runs(&self) -> usize {
        self.settle();
        self.runs.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The sort's GPU seconds (after it completes).
    pub fn gpu_seconds(&self) -> f64 {
        self.settle();
        f64::from_bits(self.gpu.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// `columns` sorted by column 0 on the device, in one command buffer, committed and not waited for: a probe
    /// of it on this thread's queue runs after it. The intermediates are in scratch, the sort's order, runs and
    /// gathered columns in buffers of their own. (Metal holds every buffer a command buffer uses until it completes,
    /// so the columns may be dropped at once.)
    pub fn sort(columns: Vec<Column>) -> Result<Keyed, String> {
        let queue = queue()?;
        let command = queue.new_command_buffer();
        let pass = Pass::new(command);
        let (mut keyed, reads) = Keyed::encode(&pass, columns)?;
        pass.end();
        let mut pending = Submitted::commit(command)?;
        pending.reads = reads;
        keyed.pending = std::sync::Mutex::new(Some(pending));
        Ok(keyed)
    }

    /// `columns` sorted by column 0, encoded into `pass` (a serial encoder: what follows in it reads the sort). The
    /// columns it reads come back with it: they must outlive the command buffer. The sort is settled by whoever
    /// waits for the command buffer (`settled`).
    pub fn encode(pass: &Pass<'_>, columns: Vec<Column>) -> Result<(Keyed, Vec<Column>), String> {
        device()?;
        let mut columns = columns.into_iter();
        let lead = columns.next().ok_or("a sort of no columns")?;
        let n = lead.rows;
        let parts = encode_sort(pass, &lead.buffer.buffer, lead.width, lead.bits, n)?;
        let mut others = Vec::new();
        let mut reads = Vec::new();
        for c in columns {
            let w = c.width;
            let gather = pipeline(&format!("gather {w:?}"), "gather", || gather_source(w))?;
            let out = Held::new(n * w.bytes())?;
            if n > 0 {
                pass.run(&gather, &[&c.buffer.buffer, &parts.perm.buffer, &out.buffer], &[n as u32], &[], groups(n));
            }
            others.push(Column { buffer: out, width: w, base: c.base, bits: c.bits, rows: c.rows });
            reads.push(c);
        }
        let base = lead.base;
        reads.push(lead);
        let kb = parts.key.bytes();
        let bytes = 4 * n + (n + 1) * (4 + kb) + others.iter().map(Column::bytes).sum::<usize>();
        let SortParts { perm, starts, rkeys, nruns, key } = parts;
        let keyed = Keyed { rows: n, key, base, perm, starts, rkeys, nruns, others, pending: std::sync::Mutex::new(None), runs: std::sync::atomic::AtomicUsize::new(usize::MAX), gpu: std::sync::atomic::AtomicU64::new(0), bytes };
        Ok((keyed, reads))
    }
}

/// A sort's results on the device: each sorted place's row (`perm`), the key's runs (each one's first place in
/// `starts`, with the end after the last; its key in `rkeys`, in the sort's word `key`) and their count (`nruns`).
pub struct SortParts {
    pub perm: Held,
    pub starts: Held,
    pub rkeys: Held,
    pub nruns: Held,
    pub key: Width,
}

/// Encodes into `pass` a stable LSD radix sort of the `n` values of `lead` (each `width` wide, their span `bits`
/// bits) and the runs of equal values: the passes only its span needs. Intermediates are in scratch; the results are
/// buffers of their own. `nruns` is not written when `n` is 0.
pub fn encode_sort(pass: &Pass<'_>, lead: &BufferRef, width: Width, bits: u32, n: usize) -> Result<SortParts, String> {
    let key = width.key();
    let variant = format!("sort {width:?} {key:?}");
    let pipe = |name: &str| pipeline(&variant, name, || sort_source(width, key));
    let (init, hist, scatter, heads, compact) = (pipe("sort_init")?, pipe("sort_hist")?, pipe("sort_scatter")?, pipe("run_heads")?, pipe("run_compact")?);
    let kb = key.bytes();
    let (keys_a, keys_b) = (scratch(KEYS_A, kb * n)?, scratch(KEYS_B, kb * n)?);
    // The order ends in `idx_a` after an even number of passes: that one is the relation's own, the other scratch.
    let passes = bits.div_ceil(8);
    let perm = Held::new(4 * n)?;
    let other = scratch(IDX, 4 * n)?;
    let (idx_a, idx_b): (&BufferRef, &BufferRef) = if passes % 2 == 0 { (&perm.buffer, &other) } else { (&other, &perm.buffer) };
    let tiles = n.div_ceil(TILE).max(1);
    let starts = Held::new(4 * (n + 1))?;
    let rkeys = Held::new(kb * n)?;
    let nruns = Held::new(4)?;
    if n > 0 {
        let histogram = scratch(HIST, 4 * 256 * tiles)?;
        let hscan = Scan::new(256 * tiles)?;
        let heads_buf = scratch(HEADS, 4 * n)?;
        let nn = n as u32;
        pass.run(&init, &[lead, &keys_a, idx_a], &[nn], &[], groups(n));
        for p in 0..passes {
            let (ki, ii, ko, io) = if p % 2 == 0 { (&keys_a, idx_a, &keys_b, idx_b) } else { (&keys_b, idx_b, &keys_a, idx_a) };
            pass.run(&hist, &[ki, &histogram], &[nn, 8 * p], &[], tiles);
            hscan.encode(pass, &histogram)?;
            pass.run(&scatter, &[ki, ii, &histogram, ko, io], &[nn, 8 * p], &[], tiles);
        }
        let keys = if passes % 2 == 0 { &keys_a } else { &keys_b };
        // The heads' scan after the histograms': the two share scratch, one after the other.
        let rscan = Scan::new(n)?;
        pass.run(&heads, &[keys, &heads_buf], &[nn], &[], groups(n));
        rscan.encode(pass, &heads_buf)?;
        pass.run(&compact, &[keys, &heads_buf, &starts.buffer, &rkeys.buffer, &nruns.buffer], &[nn], &[], groups(n));
    }
    Ok(SortParts { perm, starts, rkeys, nruns, key })
}

/// A probe join's pairs, (left row, right host row), in left row order, and its GPU time.
pub struct Probed {
    pub pairs: Vec<(u32, u32)>,
    pub gpu_seconds: f64,
    /// Command buffers: one, or two when the pairs overflowed the space expected for them.
    pub submissions: u32,
}

/// The probe join's kernels for the left keys' widths, the right's sort word and other keys' widths. Buffers: the
/// runs' keys, starts and count, the right's order, the left keys, the right's other keys, then the counts (or the
/// offsets and the pairs); after them the row count, and the keys' bases (the left's, then the right's, key 0 first).
fn probe_source(left: &[Width], key: Width, right: &[Width]) -> String {
    let k = left.len();
    let kt = key.ty();
    let lcols: String = left.iter().enumerate().map(|(i, w)| format!(", device const {}* l{i} [[buffer({})]]", w.ty(), 4 + i)).collect();
    let rcols: String = right.iter().enumerate().map(|(i, w)| format!(", device const {}* r{} [[buffer({})]]", w.ty(), i + 1, 4 + k + i)).collect();
    let n = 4 + k + right.len();
    let mut test = String::from("true");
    for i in 1..k {
        test.push_str(&format!(" && ulong(l{i}[t]) + ulong(base[{i}]) == ulong(r{i}[q]) + ulong(base[{}])", k + i));
    }
    let args = format!("device const {kt}* rkeys [[buffer(0)]], device const uint* starts [[buffer(1)]], device const uint* nruns [[buffer(2)]], device const uint* perm [[buffer(3)]]{lcols}{rcols}");
    format!(
        r#"#include <metal_stdlib>
using namespace metal;

// The run of a left row's key 0 (`lo`, and whether there is one): a bounded binary search of the runs' keys.
static inline bool find(ulong key, device const {kt}* rkeys, uint runs, long rbase, thread uint& lo) {{
    lo = 0;
    uint hi = runs;
    for (uint s = 0; s < 34 && lo < hi; s++) {{
        uint mid = (lo + hi) / 2;
        long v = long(ulong(rkeys[mid]) + ulong(rbase));
        if (v < long(key)) lo = mid + 1; else hi = mid;
    }}
    return lo < runs && long(ulong(rkeys[lo]) + ulong(rbase)) == long(key);
}}

kernel void probe_count({args}, device uint* counts [[buffer({n})]], constant uint* p [[buffer({n1})]], constant long* base [[buffer({n2})]],
                        uint t [[thread_position_in_grid]]) {{
    if (t >= p[0]) return;
    uint run;
    uint c = 0;
    if (find(ulong(l0[t]) + ulong(base[0]), rkeys, nruns[0], base[{k}], run)) {{
        for (uint q = starts[run]; q < starts[run + 1]; q++) c += uint({test});
    }}
    counts[t] = c;
}}

kernel void probe_scatter({args}, device const uint* offsets [[buffer({n})]], device uint* out [[buffer({n1})]], constant uint* p [[buffer({n2})]],
                          constant long* base [[buffer({n3})]], uint t [[thread_position_in_grid]]) {{
    if (t >= p[0]) return;
    uint run;
    if (!find(ulong(l0[t]) + ulong(base[0]), rkeys, nruns[0], base[{k}], run)) return;
    uint at = offsets[t];
    for (uint q = starts[run]; q < starts[run + 1]; q++) {{
        if ({test}) {{
            if (at < p[1]) {{ out[2 * at] = t; out[2 * at + 1] = perm[q]; }}
            at++;
        }}
    }}
}}
"#,
        n1 = n + 1,
        n2 = n + 2,
        n3 = n + 3
    )
}

/// The equi-join of `left`'s columns with `right` (sorted by its key 0), key `i` each one's column `i`. One command
/// buffer counts, scans and scatters into space for `expect` pairs (at least two a left row); when there are more,
/// a second scatters them all into space of the exact size.
pub fn probe(left: &[Column], right: &Keyed, expect: usize) -> Result<Probed, String> {
    let queue = queue()?;
    let command = queue.new_command_buffer();
    let pass = Pass::new(command);
    let job = ProbeJob::encode(&pass, left, right, expect)?;
    pass.end();
    let mut gpu = Submitted::commit(command)?.wait();
    let mut submissions = 1;
    if job.overflowed() {
        let command = queue.new_command_buffer();
        let pass = Pass::new(command);
        let job = job.rescatter(&pass, left, right)?;
        pass.end();
        gpu += Submitted::commit(command)?.wait();
        submissions += 1;
        return Ok(Probed { pairs: job.pairs(), gpu_seconds: gpu, submissions });
    }
    if job.empty {
        submissions = 0;
    }
    Ok(Probed { pairs: job.pairs(), gpu_seconds: gpu, submissions })
}

/// A probe encoded into a caller's pass (several in one command buffer, decision 0100 §5): its counts, scan and
/// pairs in buffers of its own. After the command buffer completes, its pairs, or (`overflowed`) a second scatter
/// into the exact space.
pub struct ProbeJob {
    nl: usize,
    cap: usize,
    counts: Option<Held>,
    scan: Option<Scan>,
    space: Option<Held>,
    bases: Vec<i64>,
    scatter: Option<ComputePipelineState>,
    /// No pairs can be: no left or right rows (nothing was encoded).
    pub empty: bool,
}

impl ProbeJob {
    fn inputs<'a>(left: &'a [Column], right: &'a Keyed) -> Vec<&'a BufferRef> {
        let mut inputs: Vec<&BufferRef> = vec![&right.rkeys.buffer, &right.starts.buffer, &right.nruns.buffer, &right.perm.buffer];
        inputs.extend(left.iter().map(|c| &*c.buffer.buffer));
        inputs.extend(right.others.iter().map(|c| &*c.buffer.buffer));
        inputs
    }

    /// The probe of `right` by `left`, encoded after whatever `pass` holds (a sort of `right` in it is read in order).
    pub fn encode(pass: &Pass<'_>, left: &[Column], right: &Keyed, expect: usize) -> Result<ProbeJob, String> {
        device()?;
        let k = left.len();
        if k == 0 || k != 1 + right.others.len() {
            return Err(format!("a probe of {k} keys against a sort of {}", 1 + right.others.len()));
        }
        let nl = left[0].rows;
        if nl == 0 || right.rows == 0 {
            return Ok(ProbeJob { nl, cap: 0, counts: None, scan: None, space: None, bases: Vec::new(), scatter: None, empty: true });
        }
        let (lw, rw): (Vec<Width>, Vec<Width>) = (left.iter().map(|c| c.width).collect(), right.others.iter().map(|c| c.width).collect());
        let variant = format!("probe {lw:?} {:?} {rw:?}", right.key);
        let count = pipeline(&variant, "probe_count", || probe_source(&lw, right.key, &rw))?;
        let scatter = pipeline(&variant, "probe_scatter", || probe_source(&lw, right.key, &rw))?;
        let bases: Vec<i64> = left.iter().map(|c| c.base).chain([right.base]).chain(right.others.iter().map(|c| c.base)).collect();
        let counts = Held::new(4 * nl)?;
        let scan = Scan::owned(nl)?;
        let cap = expect.max(2 * nl).min(u32::MAX as usize / 2);
        let space = Held::new(8 * cap)?;
        let inputs = Self::inputs(left, right);
        let mut with = inputs.clone();
        with.push(&counts.buffer);
        pass.run(&count, &with, &[nl as u32], &bases, groups(nl));
        scan.encode(pass, &counts.buffer)?;
        let mut with = inputs;
        with.extend([&*counts.buffer, &*space.buffer]);
        pass.run(&scatter, &with, &[nl as u32, cap as u32], &bases, groups(nl));
        Ok(ProbeJob { nl, cap, counts: Some(counts), scan: Some(scan), space: Some(space), bases, scatter: Some(scatter), empty: false })
    }

    /// The pairs, after the command buffer completed.
    pub fn total(&self) -> usize {
        self.scan.as_ref().map_or(0, |s| s.total() as usize)
    }

    /// Whether the pairs outgrew their space (after the command buffer completed).
    pub fn overflowed(&self) -> bool {
        !self.empty && self.total() > self.cap
    }

    /// The scatter again into space of the exact size, encoded into `pass` (the same `left` and `right`).
    pub fn rescatter(mut self, pass: &Pass<'_>, left: &[Column], right: &Keyed) -> Result<ProbeJob, String> {
        let total = self.total();
        let out = Held::new(8 * total)?;
        let (Some(counts), Some(scatter)) = (&self.counts, &self.scatter) else { return Ok(self) };
        let mut with = Self::inputs(left, right);
        with.extend([&*counts.buffer, &*out.buffer]);
        pass.run(scatter, &with, &[self.nl as u32, total as u32], &self.bases, groups(self.nl));
        self.space = Some(out);
        self.cap = total;
        Ok(self)
    }

    /// The pairs, (left row, right host row), in left row order: after the command buffer completed.
    pub fn pairs(&self) -> Vec<(u32, u32)> {
        let total = self.total();
        let Some(space) = &self.space else { return Vec::new() };
        // SAFETY: `total` pairs, written by a completed scatter into space for at least that many.
        let flat = unsafe { std::slice::from_raw_parts(space.buffer.contents().cast::<[u32; 2]>(), total) };
        flat.iter().map(|p| (p[0], p[1])).collect()
    }
}

/// Commits `command` (checked against the launch budget) and waits for it: its GPU seconds.
pub fn commit_wait(command: &CommandBufferRef) -> Result<f64, String> {
    Ok(Submitted::commit(command)?.wait())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(n: usize, seed: u64, modulus: i64, high: i64) -> Vec<i64> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                high + ((x >> 33) as i64).rem_euclid(modulus)
            })
            .collect()
    }

    /// Reads a device buffer of `n` `u32`s.
    fn read(b: &Held, n: usize) -> Vec<u32> {
        // SAFETY: the buffer holds at least `n` words, written by a completed command buffer.
        unsafe { std::slice::from_raw_parts(b.buffer.contents().cast::<u32>(), n) }.to_vec()
    }

    #[test]
    fn the_device_sort_is_the_hosts_stable_sort_and_the_probe_join_its_join() {
        if device().is_err() {
            return;
        }
        // 16, 32 and 64-bit spans; one value; several tiles and a part tile; negative values.
        for (n, modulus, high) in [(5000, 300, 0), (70_000, 1 << 30, 1 << 40), (3000, 1 << 50, -(1 << 55)), (1500, 1, 9), (1, 1, 0), (2100, 7, -3)] {
            let keys = rows(n, 11, modulus, high);
            let sorted = Keyed::sort(vec![Column::upload(&keys).unwrap()]).unwrap();
            sorted.settle();
            let want = crate::rows::sort(&keys);
            assert_eq!(read(&sorted.perm, n), want.perm, "n {n} modulus {modulus}");
            assert_eq!(sorted.runs(), crate::rows::runs(&want.keys).len());
        }
        // Two keys, the second of another span on each side; a left key no right row has.
        let (a0, a1) = (rows(9000, 1, 2000, 1 << 32), rows(9000, 2, 3, 0));
        let (b0, b1) = (rows(7000, 3, 2000, 1 << 32), rows(7000, 4, 5, -1));
        let right = Keyed::sort(vec![Column::upload(&b0).unwrap(), Column::upload(&b1).unwrap()]).unwrap();
        let got = probe(&[Column::upload(&a0).unwrap(), Column::upload(&a1).unwrap()], &right, 0).unwrap();
        assert_eq!(got.submissions, 1);
        // On key 0 alone, about 3.5 pairs a left row: past the space a probe expects unless told more, so a second
        // scatter; told, one submission. The same pairs.
        let alone = Keyed::sort(vec![Column::upload(&b0).unwrap()]).unwrap();
        let over = probe(&[Column::upload(&a0).unwrap()], &alone, 0).unwrap();
        let told = probe(&[Column::upload(&a0).unwrap()], &alone, 1 << 16).unwrap();
        assert_eq!((over.submissions, told.submissions), (2, 1));
        assert_eq!(over.pairs, told.pairs);
        let mut want0 = crate::rows::join(&[&a0], &[&b0]);
        want0.sort_unstable();
        let mut over = over.pairs;
        over.sort_unstable();
        assert_eq!(over, want0);
        let mut got = got.pairs;
        got.sort_unstable();
        let mut want = crate::rows::join(&[&a0, &a1], &[&b0, &b1]);
        want.sort_unstable();
        assert!(!want.is_empty());
        assert_eq!(got, want);
    }
}

/// The host's part of the device's work, each piece alone: what a sort and a probe cost the CPU.
#[cfg(test)]
mod host_costs {
    use super::*;

    /// The process's CPU, user and system: Metal's own threads' too.
    fn cpu() -> f64 {
        let mut u: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: a valid rusage out.
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
        let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1e6;
        t(u.ru_utime) + t(u.ru_stime)
    }

    /// Microseconds of this thread's CPU per run of `f`, over `n` runs.
    fn per(n: usize, mut f: impl FnMut()) -> f64 {
        let c0 = cpu();
        for _ in 0..n {
            f();
        }
        (cpu() - c0) / n as f64 * 1e6
    }

    #[test]
    #[ignore = "a measurement, not a check"]
    fn host_costs() {
        if device().is_err() {
            return;
        }
        let device = device().unwrap();
        let alloc = per(500, || drop(device.new_buffer(16384, metal::MTLResourceOptions::StorageModeShared)));
        let pooled = per(500, || drop(Held::new(16384).unwrap()));
        let queue = queue().unwrap();
        let empty = per(300, || {
            let c = queue.new_command_buffer();
            c.commit();
            c.wait_until_completed();
        });
        let src = SCAN_SOURCE;
        let block = pipeline("scan", "scan_block", || src.to_owned()).unwrap();
        let data = shared::lease_zeroed(4096).unwrap();
        let sums = shared::lease_zeroed(4).unwrap();
        let dispatch = per(100, || {
            let c = queue.new_command_buffer();
            let pass = Pass::new(c);
            for _ in 0..50 {
                pass.run(&block, &[&data, &sums], &[1024], &[], 1);
            }
            pass.end();
            c.commit();
            c.wait_until_completed();
        });
        let dispatch = (dispatch - empty) / 50.0;
        println!("host per buffer allocated {alloc:.1} us, pooled {pooled:.2} us; per empty submission {empty:.1} us; per dispatch encoded {dispatch:.2} us");
        for (wait, rows) in [(Wait::Metal, 1000usize), (Wait::Handler, 1000), (Wait::Metal, 20_000), (Wait::Handler, 20_000), (Wait::Metal, 200_000), (Wait::Handler, 200_000)] {
            set_wait(wait);
            let mut x = 7u64;
            let mut next = |m: i64| {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ((x >> 33) as i64).rem_euclid(m)
            };
            let keys: Vec<i64> = (0..rows).map(|_| (1 << 40) + next(rows as i64 * 4)).collect();
            let probe_keys: Vec<i64> = (0..rows).map(|_| (1 << 40) + next(rows as i64 * 4)).collect();
            let runs = (2_000_000 / rows).clamp(3, 200);
            let upload = per(runs, || drop(Column::upload(&keys).unwrap()));
            let mut sort_gpu = 0.0;
            let sort = per(runs, || sort_gpu += Keyed::sort(vec![Column::upload(&keys).unwrap()]).unwrap().gpu_seconds()) - upload;
            let right = Keyed::sort(vec![Column::upload(&keys).unwrap()]).unwrap();
            right.settle();
            let mut probe_gpu = 0.0;
            let mut pairs = 0;
            let probe_cpu = per(runs, || {
                let p = probe(&[Column::upload(&probe_keys).unwrap()], &right, 0).unwrap();
                probe_gpu += p.gpu_seconds;
                pairs = p.pairs.len();
            }) - upload;
            let host = per(runs, || drop(crate::rows::Keyed::new(&keys)));
            let host_join = per(runs, || drop(crate::rows::join_keyed(&[&probe_keys], &crate::rows::Keyed::new(&keys), &[&keys]))) - host;
            println!(
                "{wait:?} {rows} rows ({pairs} pairs): upload {upload:.0} us; device sort host {sort:.0} us, GPU {:.0} us; probe host {probe_cpu:.0} us, GPU {:.0} us; host radix sort {host:.0} us, host sort join (left sorted, runs merged) {host_join:.0} us",
                sort_gpu / runs as f64 * 1e6,
                probe_gpu / runs as f64 * 1e6
            );
        }
    }
}
