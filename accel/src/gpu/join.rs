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

/// Why a failed command buffer failed: its error's description and code, once a run for each description (a wave's
/// strata are refused to the CPU with no more than `BackendFailure` otherwise).
#[allow(unexpected_cfgs)]
pub fn say_failure(command: &CommandBufferRef) {
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};
    /// The descriptions said: the run's own (decision 0442).
    #[derive(Default)]
    struct Said(std::sync::Mutex<Vec<String>>);
    // SAFETY: `error` is an NSError or nil; its description an NSString, read as UTF-8 while the error lives.
    let why = unsafe {
        let error: *mut Object = msg_send![command, error];
        if error.is_null() {
            "no error given".to_owned()
        } else {
            let code: isize = msg_send![error, code];
            let text: *mut Object = msg_send![error, localizedDescription];
            let utf8: *const std::ffi::c_char = msg_send![text, UTF8String];
            let text = if utf8.is_null() { String::new() } else { std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned() };
            format!("{text} (code {code})")
        }
    };
    rerail_context::with_local::<Said, _>(|said| {
        let mut said = said.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !said.contains(&why) {
            eprintln!("rerail: a command buffer the GPU failed: {why}");
            said.push(why);
        }
    });
}

/// A Metal library, function or pipeline that does not compile is fatal (decision 0382): the run stops, naming the
/// kernel and the compiler's message, and no caller runs another path (the CPU's) on it.
#[track_caller]
pub fn compile_failed(name: &str, what: &str, error: impl std::fmt::Display) -> ! {
    panic!("Metal: the {what} of `{name}` does not compile (decision 0382: fatal): {error}")
}

/// The threads of a join kernel's group: every dispatch's (`Pass::dispatch*`).
pub const GROUP: u64 = 256;

/// `name` of `source` on the shared device: through the store of compiled kernels (decision 0395).
pub fn compile(source: &str, name: &str) -> Result<ComputePipelineState, String> {
    device()?;
    Ok(super::kernels::pipeline_fixed(source, name, None))
}

thread_local! {
    /// This thread's scratch buffers, two sets by slot: a sort's or probe's intermediates, kept for the next. A
    /// submission uses one set (`scratch_set`); the lane has at most two in flight, each on its own set, and starts
    /// a third only once the first completed, so no two in flight share one.
    static SCRATCH: RefCell<[Vec<Option<shared::Pooled>>; 2]> = const { RefCell::new([Vec::new(), Vec::new()]) };
    static SET: Cell<usize> = const { Cell::new(0) };
}

/// Makes `set` (0 or 1) this thread's scratch set; the previous one.
pub fn scratch_set(set: usize) -> usize {
    SET.with(|c| c.replace(set & 1))
}

/// The pipeline `name` of the variant `variant`, its source made by `source` the first time: kept by the one
/// manager (decision 0100 §2), so every thread finds it compiled, and in the store of compiled kernels (decision
/// 0395). A variant's source is compiled once, into one library, and each of its kernels made from that library.
pub fn pipeline(variant: &str, name: &str, source: impl FnOnce() -> String) -> Result<ComputePipelineState, String> {
    shared::pipeline(&format!("join {variant} {name}"), || {
        device()?;
        let pipe = super::kernels::pipeline_fixed(&source(), name, None);
        // Every dispatch of a join kernel is of groups of 256 threads: a kernel whose registers allow fewer cannot run
        // them (a dispatch past its most is undefined). Such a pipeline does not compile, and the run stops (decision
        // 0382): no caller runs another path on it.
        let most = pipe.max_total_threads_per_threadgroup();
        if most < GROUP {
            compile_failed(name, "pipeline", format!("at most {most} threads a group, its dispatches' {GROUP}"));
        }
        Ok(pipe)
    })
}

/// The radix sort's kernels of a lead column of `width`, and the gather of a column of `width`.
fn sort_pipes(width: Width) -> Result<[ComputePipelineState; 5], String> {
    let key = width.key();
    let variant = format!("sort {width:?} {key:?}");
    let pipe = |name: &str| pipeline(&variant, name, || sort_source(width, key));
    Ok([pipe("sort_init")?, pipe("sort_hist")?, pipe("sort_scatter")?, pipe("run_heads")?, pipe("run_compact")?])
}

fn gather_pipe(width: Width) -> Result<ComputePipelineState, String> {
    pipeline(&format!("gather {width:?}"), "gather", || gather_source(width))
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
    /// The host clock at its commit (`host_clock`); NaN for a second watcher of a command buffer another waits
    /// for (`watch`), which adds no timeline.
    committed: f64,
}

/// The host clock Metal's command buffer times are on (`mach_absolute_time`, in seconds).
pub fn host_clock() -> f64 {
    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }
    unsafe extern "C" {
        fn mach_timebase_info(info: *mut Timebase) -> i32;
        fn mach_absolute_time() -> u64;
    }
    static RATIO: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    let ratio = *RATIO.get_or_init(|| {
        let mut info = Timebase { numer: 0, denom: 0 };
        // SAFETY: fills the struct.
        unsafe { mach_timebase_info(&mut info) };
        f64::from(info.numer) / f64::from(info.denom.max(1)) / 1e9
    });
    // SAFETY: no arguments.
    (unsafe { mach_absolute_time() }) as f64 * ratio
}

/// A completed command buffer's timeline, as wall-only phases of the job (`crate::score`): commit to the system's
/// scheduling (`kernelStartTime`), to the device's start, the device's run, and its end to the host's wait returning.
#[allow(unexpected_cfgs)] // `msg_send!` expands to a `cargo-clippy` cfg check.
pub fn add_timeline(command: &CommandBufferRef, committed: f64, returned: f64) {
    use objc::{msg_send, sel, sel_impl};
    let (k, s, e): (f64, f64, f64) = unsafe { (msg_send![command, kernelStartTime], msg_send![command, GPUStartTime], msg_send![command, GPUEndTime]) };
    if k <= 0.0 || s <= 0.0 || e <= 0.0 {
        return;
    }
    crate::score::add("gpu submit", 0.0, (k - committed).max(0.0));
    crate::score::add("gpu schedule", 0.0, (s - k).max(0.0));
    crate::score::add("gpu run", 0.0, (e - s).max(0.0));
    crate::score::add("gpu wake", 0.0, (returned - e).max(0.0));
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
        crate::score::phase("gpu commit", || -> Result<(), String> {
            shared::guard()?;
            command.commit();
            Ok(())
        })?;
        Ok(Submitted { command: command.to_owned(), done, reads: Vec::new(), committed: host_clock() })
    }

    /// Another wait for a committed command buffer: what a sort encoded into it waits for (`Keyed::settle`).
    pub(super) fn watch(&self) -> Submitted {
        Submitted { command: self.command.clone(), done: None, reads: Vec::new(), committed: f64::NAN }
    }

    /// Waits for it; its GPU seconds. The buffers it read return to the pool after.
    pub fn wait(self) -> f64 {
        crate::score::phase("gpu wait", || match &self.done {
            Some(rx) if self.command.status() != metal::MTLCommandBufferStatus::Completed => {
                let _ = rx.recv();
                // The handler runs as the buffer completes: its status may lag a moment.
                self.command.wait_until_completed();
            }
            _ => self.command.wait_until_completed(),
        });
        if !self.committed.is_nan() {
            add_timeline(&self.command, self.committed, host_clock());
        }
        gpu_seconds(&self.command)
    }

    /// `wait`, and whether the GPU ran it to the end: a command buffer the GPU failed (out of memory, a fault)
    /// completes too, its outputs partly written. A caller that keeps what it wrote must check.
    pub fn wait_ok(self) -> (f64, bool) {
        let command = self.command.clone();
        let seconds = self.wait();
        let ok = command.status() != metal::MTLCommandBufferStatus::Error;
        if !ok {
            say_failure(&command);
        }
        (seconds, ok)
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
    let set = SET.with(Cell::get);
    SCRATCH.with_borrow_mut(|sets| {
        let s = &mut sets[set];
        if s.len() <= slot {
            s.resize_with(slot + 1, || None);
        }
        if s[slot].as_ref().is_none_or(|b| (b.length() as usize) < bytes) {
            // A command buffer not yet completed may read the shorter one: it goes to Metal, which holds it until
            // then, not to the pool, whose next lease may be written by the host at once.
            if let Some(old) = s[slot].take() {
                old.release();
            }
            s[slot] = Some(shared::lease_for_join(bytes.max(64), true)?);
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
    _lease: Lease,
    pub buffer: Buffer,
}

/// What keeps a held buffer's memory: a lease of the pool, a staged column whose shared buffer the device reads
/// in place (`crate::col`), or what holds a block the device reads in place (its owner's handle: decision 0325).
enum Lease {
    Pooled(#[allow(dead_code)] shared::Pooled),
    Col(#[allow(dead_code)] crate::col::Col),
    Kept(#[allow(dead_code)] Box<dyn std::any::Any + Send + Sync>),
}

impl Held {
    /// A block's own device buffer (`rerail_blocks::Block::gpu_buffer`), kept bound while `keep` (what holds the
    /// block) lives: the device reads the block where it is, nothing copied (decision 0325).
    pub fn kept(buffer: Buffer, keep: Box<dyn std::any::Any + Send + Sync>) -> Held {
        Held { _lease: Lease::Kept(keep), buffer }
    }

    /// A buffer of at least `bytes` (its contents are a previous user's: every reader writes first).
    pub fn new(bytes: usize) -> Result<Held, String> {
        device()?;
        let lease = shared::lease_for_join(bytes, false)?;
        let buffer = lease.buffer().clone();
        Ok(Held { _lease: Lease::Pooled(lease), buffer })
    }

    /// A buffer of at least `bytes`, its first `bytes` zero.
    pub fn zeroed(bytes: usize) -> Result<Held, String> {
        device()?;
        let lease = shared::lease_for_join(bytes, true)?;
        let buffer = lease.buffer().clone();
        Ok(Held { _lease: Lease::Pooled(lease), buffer })
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
    /// Whether `end` ended the encoder; dropped without it (an error between its dispatches: a refused lease), the
    /// drop ends it, as Metal requires of an encoder it releases.
    ended: bool,
}

impl<'a> Pass<'a> {
    pub fn new(command: &'a CommandBufferRef) -> Pass<'a> {
        let encoder = command.new_compute_command_encoder();
        // Named: the launch budget's record reads the encoder's label, and an unnamed one's is nil.
        encoder.set_label("rows");
        Pass { encoder, dispatches: Cell::new(0), ended: false }
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
        self.encoder.dispatch_thread_groups(MTLSize::new(groups.max(1) as u64, 1, 1), MTLSize::new(GROUP, 1, 1));
        self.dispatches.set(self.dispatches.get() + 1);
    }

    /// `run_bytes`, its threadgroups the three words at `offset` of `grid` (written by the device before it).
    pub fn run_bytes_indirect(&self, pipeline: &ComputePipelineState, buffers: &[&BufferRef], bytes: &[u8], grid: &BufferRef, offset: u64) {
        self.encoder.set_compute_pipeline_state(pipeline);
        for (i, b) in buffers.iter().enumerate() {
            self.encoder.set_buffer(i as u64, Some(b), 0);
        }
        self.encoder.set_bytes(buffers.len() as u64, bytes.len().max(4) as u64, if bytes.is_empty() { [0u32].as_ptr().cast() } else { bytes.as_ptr().cast() });
        self.encoder.dispatch_thread_groups_indirect(grid, offset, MTLSize::new(GROUP, 1, 1));
        self.dispatches.set(self.dispatches.get() + 1);
    }

    /// `run` of `params`, its threadgroups the three words at `offset` of `grid`.
    pub fn run_indirect(&self, pipeline: &ComputePipelineState, buffers: &[&BufferRef], params: &[u32], grid: &BufferRef, offset: u64) {
        self.encoder.set_compute_pipeline_state(pipeline);
        for (i, b) in buffers.iter().enumerate() {
            self.encoder.set_buffer(i as u64, Some(b), 0);
        }
        self.encoder.set_bytes(buffers.len() as u64, std::mem::size_of_val(params).max(4) as u64, if params.is_empty() { [0u32].as_ptr().cast() } else { params.as_ptr().cast() });
        self.encoder.dispatch_thread_groups_indirect(grid, offset, MTLSize::new(GROUP, 1, 1));
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
        self.encoder.dispatch_thread_groups(MTLSize::new(groups.max(1) as u64, 1, 1), MTLSize::new(GROUP, 1, 1));
        self.dispatches.set(self.dispatches.get() + 1);
    }

    /// Makes `buffers` resident for the dispatches after it, to be read: buffers a kernel reaches by address (a
    /// view's directory and tapes, decision 0353 step 4d), bound by none.
    pub fn use_read(&self, buffers: &[&BufferRef]) {
        for b in buffers {
            self.encoder.use_resource(b, metal::MTLResourceUsage::Read);
        }
    }

    /// Makes `buffer` resident for the dispatches after it, to be read and written: a cache a kernel's reads keep
    /// (a 16z batch's blocks, decision 0353 step 4b), reached by address.
    pub fn use_read_write(&self, buffer: &BufferRef) {
        self.encoder.use_resource(buffer, metal::MTLResourceUsage::Read | metal::MTLResourceUsage::Write);
    }

    pub fn end(mut self) {
        self.encoder.end_encoding();
        self.ended = true;
    }
}

impl Drop for Pass<'_> {
    fn drop(&mut self) {
        if !self.ended {
            self.encoder.end_encoding();
        }
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

// The grids of a scan sized on the device (`Scan::size`): of min(totals[p[0]], p[1]) + 1 values, and of a pass of
// 256 threads a group over them. Per level k: its count at a[k], its threadgroups at a[8 + 3k]; the pass's at a[32].
kernel void scan_size(device const uint* totals [[buffer(0)]], device uint* a [[buffer(1)]], constant uint* p [[buffer(2)]],
                      uint t [[thread_position_in_grid]]) {
    if (t != 0) return;
    uint n = min(totals[p[0]], p[1]) + 1;
    a[32] = (n + 255) / 256; a[33] = 1; a[34] = 1;
    for (uint k = 0; k < p[2] && k < 8; k++) {
        uint g = max((n + 1023) / 1024, 1u);
        a[k] = n; a[8 + 3 * k] = g; a[9 + 3 * k] = 1; a[10 + 3 * k] = 1;
        n = g;
    }
}

// scan_block and scan_add, their count a[p[0]] (`scan_size`'s).
kernel void scan_block_sized(device uint* data [[buffer(0)]], device uint* sums [[buffer(1)]], device const uint* a [[buffer(2)]],
                             constant uint* p [[buffer(3)]], uint tg [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]],
                             uint lane [[thread_index_in_simdgroup]], uint sg [[simdgroup_index_in_threadgroup]]) {
    uint n = a[p[0]];
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

kernel void scan_add_sized(device uint* data [[buffer(0)]], device const uint* sums [[buffer(1)]], device const uint* a [[buffer(2)]],
                           constant uint* p [[buffer(3)]], uint tg [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]]) {
    uint n = a[p[0]];
    uint at = tg * 1024 + t * 4;
    uint s = sums[tg];
    for (uint i = 0; i < 4; i++) { if (at + i < n) data[at + i] += s; }
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
    pub fn owned(count: usize) -> Result<Scan, String> {
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

    /// An owned scan's buffers, to keep until its command buffer completes.
    fn into_held(self) -> Vec<Held> {
        self._owned
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

    /// Encodes the grids of a scan sized on the device: of `min(totals[slot], cap) + 1` values (a stage's rows and
    /// one), from the binding table's count the work before wrote, not the host's guess of its space. The scan's
    /// levels are those of `count` values (its most). Returns the grids' words, for `encode_sized` and
    /// `Pass::run_bytes_sized`.
    pub fn size(&self, pass: &Pass<'_>, totals: &BufferRef, slot: usize, cap: usize) -> Result<Held, String> {
        assert!(self.levels.len() <= 8 && cap < self.count);
        let size = pipeline("scan", "scan_size", || SCAN_SOURCE.to_owned())?;
        let a = Held::new(4 * 40)?;
        pass.run(&size, &[totals, &a.buffer], &[slot as u32, cap as u32, self.levels.len() as u32], &[], 1);
        Ok(a)
    }

    /// `encode`, each level's grid `size`'s.
    pub fn encode_sized(&self, pass: &Pass<'_>, data: &BufferRef, sizes: &Held) -> Result<(), String> {
        let block = pipeline("scan", "scan_block_sized", || SCAN_SOURCE.to_owned())?;
        let add = pipeline("scan", "scan_add_sized", || SCAN_SOURCE.to_owned())?;
        let a: &BufferRef = &sizes.buffer;
        let mut inputs: Vec<&BufferRef> = vec![data];
        for (k, (_, sums)) in self.levels.iter().enumerate() {
            pass.run_indirect(&block, &[inputs[k], sums, a], &[k as u32], a, 4 * (8 + 3 * k as u64));
            inputs.push(sums);
        }
        for k in (0..self.levels.len().saturating_sub(1)).rev() {
            pass.run_indirect(&add, &[inputs[k], &self.levels[k].1, a], &[k as u32], a, 4 * (8 + 3 * k as u64));
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
    /// The command buffer this sort was encoded into, committed and not waited for: `settle` waits for it.
    pub(super) fn pending_on(&self, s: &Submitted) {
        *self.pending.lock().expect("a sort's pending command buffer") = Some(s.watch());
    }

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

    /// The runs' keys in order: the key's distinct values, ascending (after the sort completes).
    pub fn run_keys(&self) -> Vec<i64> {
        let n = self.runs();
        let p = self.rkeys.buffer.contents();
        // SAFETY: `run_compact` wrote `n` keys of the sort's word, its command buffer completed (`runs` settled it).
        (0..n).map(|i| unsafe {
            let k = match self.key {
                Width::W16 => *p.cast::<u16>().add(i) as u64,
                Width::W32 => *p.cast::<u32>().add(i) as u64,
                Width::W64 => *p.cast::<u64>().add(i),
            };
            (k as i64).wrapping_add(self.base)
        }).collect()
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
            let gather = gather_pipe(w)?;
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

/// `n` pairs of 32-bit words (`pairs`, as `[first, second]`) sorted by (first, second), encoded into `pass` (a
/// serial encoder): `out` gets them in that order. The second word takes `second_bits` bits and the first
/// `first_bits`, so the sort's key is `first << second_bits | second`, in 32 bits when they fit. The radix sort's
/// passes alone, without runs: a tile join's pairs come out in the kernel's order (left row, then right row) with
/// no host sort (decision 0100 §9). Its intermediates are kept in `keep` until the command buffer completes.
pub fn encode_sorted_pairs(pass: &Pass<'_>, pairs: &Held, n: usize, first_bits: u32, second_bits: u32, out: &Held, keep: &mut Vec<Held>) -> Result<(), String> {
    let (words, bits) = (pairs, first_bits + second_bits);
    if n == 0 {
        return Ok(());
    }
    let width = if bits <= 32 { Width::W32 } else { Width::W64 };
    let [init, hist, scatter, ..] = sort_pipes(width)?;
    let narrow = pipeline("pair keys", "pair_keys", || PAIR_KEYS_SOURCE.to_owned())?;
    let gather = gather_pipe(Width::W64)?;
    let kb = width.bytes();
    let lead = Held::new(kb * n)?;
    let (keys_a, keys_b) = (Held::new(kb * n)?, Held::new(kb * n)?);
    let passes = bits.div_ceil(8);
    let (idx_a, idx_b) = (Held::new(4 * n)?, Held::new(4 * n)?);
    let tiles = n.div_ceil(TILE).max(1);
    let histogram = Held::new(4 * 256 * tiles)?;
    let hscan = Scan::owned(256 * tiles)?;
    let nn = n as u32;
    // The words as sort keys of the sort's width (a 32-bit key when the bits fit).
    pass.run(&narrow, &[&words.buffer, &lead.buffer], &[nn, u32::from(width == Width::W64), second_bits], &[], groups(n));
    pass.run(&init, &[&lead.buffer, &keys_a.buffer, &idx_a.buffer], &[nn], &[], groups(n));
    for p in 0..passes {
        let (ki, ii, ko, io) = if p % 2 == 0 { (&keys_a, &idx_a, &keys_b, &idx_b) } else { (&keys_b, &idx_b, &keys_a, &idx_a) };
        pass.run(&hist, &[&ki.buffer, &histogram.buffer], &[nn, 8 * p], &[], tiles);
        hscan.encode(pass, &histogram.buffer)?;
        pass.run(&scatter, &[&ki.buffer, &ii.buffer, &histogram.buffer, &ko.buffer, &io.buffer], &[nn, 8 * p], &[], tiles);
    }
    let order = if passes % 2 == 0 { &idx_a } else { &idx_b };
    pass.run(&gather, &[&words.buffer, &order.buffer, &out.buffer], &[nn], &[], groups(n));
    keep.extend([lead, keys_a, keys_b, idx_a, idx_b, histogram]);
    keep.extend(hscan.into_held());
    Ok(())
}

/// The pairs' sort's pipelines, compiled (`prewarm`).
pub fn prewarm_pair_sort() -> Result<(), String> {
    pipeline("pair keys", "pair_keys", || PAIR_KEYS_SOURCE.to_owned())?;
    Ok(())
}

/// Pairs (two 32-bit words, the first in the low half) as sort keys: the first word above the second's `p[2]` bits,
/// written as 64 bits when `p[1]`, else 32.
const PAIR_KEYS_SOURCE: &str = r#"#include <metal_stdlib>
using namespace metal;

kernel void pair_keys(device const ulong* words [[buffer(0)]], device uint* out [[buffer(1)]], constant uint* p [[buffer(2)]],
                      uint t [[thread_position_in_grid]]) {
    if (t >= p[0]) return;
    ulong w = words[t];
    ulong key = (ulong(uint(w)) << p[2]) | (w >> 32);
    if (p[1] != 0) { out[2 * t] = uint(key); out[2 * t + 1] = uint(key >> 32); }
    else out[t] = uint(key);
}
"#;

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
///
/// Every allocation that can fail is made before the first dispatch: a sort refused after encoding would drop
/// buffers (`perm`, `starts`) that work already in the command buffer writes, back to the pool at once.
pub fn encode_sort(pass: &Pass<'_>, lead: &BufferRef, width: Width, bits: u32, n: usize) -> Result<SortParts, String> {
    let key = width.key();
    let [init, hist, scatter, heads, compact] = sort_pipes(width)?;
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
        // The heads' scan after the histograms': the two share scratch levels, one after the other in the command
        // buffer. Made here, before any dispatch, so its levels are at their size for both.
        let rscan = Scan::new(n)?;
        pipeline("scan", "scan_block", || SCAN_SOURCE.to_owned())?;
        pipeline("scan", "scan_add", || SCAN_SOURCE.to_owned())?;
        let nn = n as u32;
        pass.run(&init, &[lead, &keys_a, idx_a], &[nn], &[], groups(n));
        for p in 0..passes {
            let (ki, ii, ko, io) = if p % 2 == 0 { (&keys_a, idx_a, &keys_b, idx_b) } else { (&keys_b, idx_b, &keys_a, idx_a) };
            pass.run(&hist, &[ki, &histogram], &[nn, 8 * p], &[], tiles);
            hscan.encode(pass, &histogram)?;
            pass.run(&scatter, &[ki, ii, &histogram, ko, io], &[nn, 8 * p], &[], tiles);
        }
        let keys = if passes % 2 == 0 { &keys_a } else { &keys_b };
        pass.run(&heads, &[keys, &heads_buf], &[nn], &[], groups(n));
        rscan.encode(pass, &heads_buf)?;
        pass.run(&compact, &[keys, &heads_buf, &starts.buffer, &rkeys.buffer, &nruns.buffer], &[nn], &[], groups(n));
    }
    Ok(SortParts { perm, starts, rkeys, nruns, key })
}

/// A column's statistics made on the device (decision 0209 §6): its rows, distinct values, the sums of its values'
/// counts squared and cubed, and its `k` most common values with their counts, most common first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColStats {
    pub rows: u64,
    pub ndv: u64,
    pub sq: u64,
    pub cube: u64,
    pub top: Vec<(i64, u64)>,
}

/// The kernels of `column_stats` for a sort word `key`.
fn col_stats_source(key: Width) -> String {
    let k = key.ty();
    format!(
        r#"#include <metal_stdlib>
using namespace metal;

// Each run's length (0 past the runs), 1,024 a threadgroup, four a thread; the group's sums of lengths squared and
// cubed into `part` (two words a group).
kernel void col_stats(device const uint* starts [[buffer(0)]], device const uint* nruns [[buffer(1)]], device uint* lens [[buffer(2)]],
                      device ulong* part [[buffer(3)]], constant uint* p [[buffer(4)]],
                      uint g [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]]) {{
    uint n = p[0];
    uint runs = n > 0 ? nruns[0] : 0;
    ulong sq = 0, cube = 0;
    for (uint i = 0; i < 4; i++) {{
        uint r = g * 1024 + i * 256 + t;
        if (r >= n) break;
        uint len = r < runs ? starts[r + 1] - starts[r] : 0;
        lens[r] = len;
        ulong l = len;
        sq += l * l;
        cube += l * l * l;
    }}
    threadgroup ulong S[256];
    threadgroup ulong C[256];
    S[t] = sq; C[t] = cube;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint s = 128; s > 0; s >>= 1) {{
        if (t < s) {{ S[t] += S[t + s]; C[t] += C[t + s]; }}
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }}
    if (t == 0) {{ part[2 * g] = S[0]; part[2 * g + 1] = C[0]; }}
}}

// One threadgroup: the groups' sums (`p[1]` of them) and the runs' count into `out`.
kernel void col_sum(device const ulong* part [[buffer(0)]], device const uint* nruns [[buffer(1)]], device ulong* out [[buffer(2)]],
                    constant uint* p [[buffer(3)]], uint t [[thread_index_in_threadgroup]]) {{
    uint n = p[0], groups = p[1];
    ulong sq = 0, cube = 0;
    for (uint g = t; g < groups; g += 256) {{ sq += part[2 * g]; cube += part[2 * g + 1]; }}
    threadgroup ulong S[256];
    threadgroup ulong C[256];
    S[t] = sq; C[t] = cube;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint s = 128; s > 0; s >>= 1) {{
        if (t < s) {{ S[t] += S[t + s]; C[t] += C[t + s]; }}
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }}
    if (t == 0) {{ out[0] = n > 0 ? nruns[0] : 0; out[1] = S[0]; out[2] = C[0]; }}
}}

// The k longest runs, longest first, from the lengths' sort (ascending, stable): each one's key and length.
kernel void col_top(device const uint* perm [[buffer(0)]], device const uint* lens [[buffer(1)]], device const {k}* rkeys [[buffer(2)]],
                    device ulong* out [[buffer(3)]], constant uint* p [[buffer(4)]], uint t [[thread_position_in_grid]]) {{
    uint n = p[0], k = p[1];
    if (t >= k) return;
    ulong key = 0, len = 0;
    if (t < n) {{ uint r = perm[n - 1 - t]; len = lens[r]; key = ulong(rkeys[r]); }}
    out[3 + t] = key;
    out[3 + k + t] = len;
}}
"#
    )
}

/// A column for `column_stats_of`: values on the host, uploaded as narrow as their span lets them be; or 64-bit values
/// the device already holds (a resident relation's column, `rows` longs from long `at` of `buffer`), read in place.
pub enum StatCol<'a> {
    Host(&'a [i64]),
    /// Values the device holds: `rows` of them at value `at` of `buffer`, or, with `col`, column `col` of the
    /// `rerail_blocks` relation whose descriptor is `at` bytes into the block `buffer` binds (decision 0325).
    Device { buffer: &'a BufferRef, at: usize, rows: usize, col: Option<u32> },
}

impl StatCol<'_> {
    fn rows(&self) -> usize {
        match self {
            StatCol::Host(v) => v.len(),
            StatCol::Device { rows, .. } => *rows,
        }
    }
}

/// A device column's span and its keys (`col_span`, `col_base`, `col_narrow`): each tile's least and greatest into
/// `part`, then the least (the base) and whether the span passes 32 bits into `span`, then each value less the base
/// as a sort key, 32 bits wide, or 64 when `p[3]` says the span is wide (the batch is encoded again for those).
/// The column span kernels' source: `COL_SPAN_SOURCE` after the relation's reads written once
/// (`rerail_blocks::RELATION_READS`), which a relation's column is read in its block with (decision 0325).
fn col_span_source() -> String {
    COL_SPAN_SOURCE.replacen("using namespace metal;\n", &format!("using namespace metal;\n{}", rerail_blocks::RELATION_READS), 1)
}

const COL_SPAN_SOURCE: &str = r#"#include <metal_stdlib>
using namespace metal;

// Value `r` of a column: at `at` of `src`, or (`rel_col` its number + 1) column `rel_col - 1` of the relation whose
// descriptor is `at` bytes into the block `src` binds.
static inline long col_value(device const long* src, ulong at, uint rel_col, uint r) {
    return rel_col == 0u ? src[at + r] : long(clif_rel_get((device const uchar*)src, at, r, rel_col - 1u));
}

constant long LEAST = long(0x8000000000000000UL);
constant long MOST = long(0x7fffffffffffffffUL);

kernel void col_span(device const long* src [[buffer(0)]], device long* part [[buffer(1)]], constant uint* p [[buffer(2)]],
                     uint g [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]]) {
    uint n = p[0];
    ulong at = ulong(p[1]) | (ulong(p[2]) << 32);
    long lo = MOST, hi = LEAST;
    for (uint i = 0; i < 4; i++) {
        uint r = g * 1024 + i * 256 + t;
        if (r >= n) break;
        long v = col_value(src, at, p[3], r);
        lo = min(lo, v);
        hi = max(hi, v);
    }
    threadgroup long L[256];
    threadgroup long H[256];
    L[t] = lo; H[t] = hi;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint s = 128; s > 0; s >>= 1) {
        if (t < s) { L[t] = min(L[t], L[t + s]); H[t] = max(H[t], H[t + s]); }
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }
    if (t == 0) { part[2 * g] = L[0]; part[2 * g + 1] = H[0]; }
}

// One threadgroup: the tiles' (`p[0]` of them) least and greatest; the base and whether the span is wide.
kernel void col_base(device const long* part [[buffer(0)]], device long* span [[buffer(1)]], constant uint* p [[buffer(2)]],
                     uint t [[thread_index_in_threadgroup]]) {
    uint groups = p[0];
    long lo = MOST, hi = LEAST;
    for (uint g = t; g < groups; g += 256) { lo = min(lo, part[2 * g]); hi = max(hi, part[2 * g + 1]); }
    threadgroup long L[256];
    threadgroup long H[256];
    L[t] = lo; H[t] = hi;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint s = 128; s > 0; s >>= 1) {
        if (t < s) { L[t] = min(L[t], L[t + s]); H[t] = max(H[t], H[t + s]); }
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }
    if (t == 0) { span[0] = L[0]; span[1] = (ulong(H[0]) - ulong(L[0])) >> 32 != 0 ? 1 : 0; }
}

kernel void col_narrow(device const long* src [[buffer(0)]], device const long* span [[buffer(1)]], device uchar* keys [[buffer(2)]],
                       constant uint* p [[buffer(3)]], uint r [[thread_position_in_grid]]) {
    uint n = p[0];
    if (r >= n) return;
    ulong at = ulong(p[1]) | (ulong(p[2]) << 32);
    ulong d = ulong(col_value(src, at, p[4], r)) - ulong(span[0]);
    if (p[3] == 0) ((device uint*)keys)[r] = uint(d); else ((device ulong*)keys)[r] = d;
}
"#;

/// Decision 0209 §6, the GPU batch over host columns (`column_stats_of`).
pub fn column_stats(cols: &[&[i64]], k: usize) -> Result<Vec<ColStats>, String> {
    let cols: Vec<StatCol<'_>> = cols.iter().map(|c| StatCol::Host(c)).collect();
    column_stats_of(&cols, k)
}

/// Decision 0209 §6, the GPU batch: each column's statistics (`ColStats`) and its `k` most common values, all in one
/// command buffer. Per column: the radix sort with its runs (`encode_sort`), a value's count its run's length;
/// `col_stats` reduces the lengths a tile a threadgroup, `col_sum` the tiles'; a second sort of the lengths gives
/// the longest runs (`col_top`). The host reads one small buffer a column. Exact, as the CPU's batch is.
///
/// A column the device holds (`StatCol::Device`) is not read by the host: its span is found on the device
/// (`COL_SPAN_SOURCE`), and its keys sorted 32 bits wide. A column whose span passes 32 bits is made again in a second
/// command buffer, 64 bits wide.
pub fn column_stats_of(cols: &[StatCol<'_>], k: usize) -> Result<Vec<ColStats>, String> {
    device()?;
    let mut got = stats_round(cols, k, false)?;
    let wide: Vec<usize> = got.iter().enumerate().filter(|(_, g)| g.is_none()).map(|(i, _)| i).collect();
    if !wide.is_empty() {
        let again: Vec<StatCol<'_>> = wide.iter().map(|&i| match &cols[i] {
            StatCol::Device { buffer, at, rows, col } => StatCol::Device { buffer, at: *at, rows: *rows, col: *col },
            StatCol::Host(v) => StatCol::Host(v),
        }).collect();
        for (i, g) in wide.into_iter().zip(stats_round(&again, k, true)?) {
            got[i] = g;
        }
    }
    Ok(got.into_iter().map(|g| g.expect("made 64 bits wide")).collect())
}

/// One command buffer of `column_stats_of`; a device column whose span passes 32 bits is `None` unless `wide`.
fn stats_round(cols: &[StatCol<'_>], k: usize, wide: bool) -> Result<Vec<Option<ColStats>>, String> {
    let queue = queue()?;
    let command = queue.new_command_buffer();
    let pass = Pass::new(command);
    let mut keep: Vec<Held> = Vec::new();
    let mut reads: Vec<Column> = Vec::new();
    // Per column: its statistics' words, and its base (on the host, or the device's span buffer).
    let mut outs: Vec<Option<(Held, Result<i64, Held>)>> = Vec::new();
    for col in cols {
        let n = col.rows();
        if n == 0 {
            outs.push(None);
            continue;
        }
        let nn = n as u32;
        let tiles = n.div_ceil(TILE).max(1);
        let (keys, width, bits, base): (Buffer, Width, u32, Result<i64, Held>) = match col {
            StatCol::Host(values) => {
                let c = Column::upload(values)?;
                let got = (c.buffer.buffer.clone(), c.width, c.bits, Ok(c.base));
                reads.push(c);
                got
            }
            StatCol::Device { buffer, at, rows: _, col } => {
                let spans = pipeline("column span", "col_span", col_span_source)?;
                let based = pipeline("column span", "col_base", col_span_source)?;
                let narrow = pipeline("column span", "col_narrow", col_span_source)?;
                let (lo, hi) = (*at as u32, (*at >> 32) as u32);
                // A relation's column (its number + 1), or 0 for values at `at`.
                let rel_col = col.map_or(0, |c| c + 1);
                let part = Held::new(16 * tiles)?;
                let span = Held::new(16)?;
                let width = if wide { Width::W64 } else { Width::W32 };
                let keys = Held::new(width.bytes() * n)?;
                pass.run(&spans, &[buffer, &part.buffer], &[nn, lo, hi, rel_col], &[], tiles);
                pass.run(&based, &[&part.buffer, &span.buffer], &[tiles as u32], &[], 1);
                pass.run(&narrow, &[buffer, &span.buffer, &keys.buffer], &[nn, lo, hi, u32::from(wide), rel_col], &[], groups(n));
                let buf = keys.buffer.clone();
                keep.extend([part, keys]);
                (buf, width, if wide { 64 } else { 32 }, Err(span))
            }
        };
        let parts = encode_sort(&pass, &keys, width, bits, n)?;
        let variant = format!("column stats {:?}", parts.key.bytes());
        let stats = pipeline(&variant, "col_stats", || col_stats_source(parts.key))?;
        let sum = pipeline(&variant, "col_sum", || col_stats_source(parts.key))?;
        let top = pipeline(&variant, "col_top", || col_stats_source(parts.key))?;
        let lens = Held::new(4 * n)?;
        let out = Held::zeroed(8 * (3 + 2 * k))?;
        // The lengths' sums spread over the column's tiles, then their sums in one threadgroup.
        let part = Held::new(16 * tiles)?;
        pass.run(&stats, &[&parts.starts.buffer, &parts.nruns.buffer, &lens.buffer, &part.buffer], &[nn], &[], tiles);
        pass.run(&sum, &[&part.buffer, &parts.nruns.buffer, &out.buffer], &[nn, tiles as u32], &[], 1);
        keep.push(part);
        let by_len = encode_sort(&pass, &lens.buffer, Width::W32, 32, n)?;
        pass.run(&top, &[&by_len.perm.buffer, &lens.buffer, &parts.rkeys.buffer, &out.buffer], &[nn, k as u32], &[], groups(k));
        outs.push(Some((out, base)));
        keep.extend([lens, parts.perm, parts.starts, parts.rkeys, parts.nruns, by_len.perm, by_len.starts, by_len.rkeys, by_len.nruns]);
    }
    pass.end();
    let mut done = Submitted::commit(command)?;
    done.reads = reads;
    done.wait();
    drop(keep);
    Ok(outs
        .into_iter()
        .zip(cols)
        .map(|(o, col)| {
            let Some((out, base)) = o else { return Some(ColStats::default()) };
            let base = match base {
                Ok(b) => b,
                Err(span) => {
                    // SAFETY: two longs, written by `col_base`; the command buffer completed.
                    let s = unsafe { std::slice::from_raw_parts(span.buffer.contents().cast::<i64>(), 2) };
                    if s[1] != 0 && !wide {
                        return None;
                    }
                    s[0]
                }
            };
            // SAFETY: `3 + 2k` words, written by `col_stats` and `col_top`; the command buffer completed.
            let w = unsafe { std::slice::from_raw_parts(out.buffer.contents().cast::<u64>(), 3 + 2 * k) };
            let ndv = w[0];
            let top = (0..k.min(ndv as usize)).map(|i| ((w[3 + i] as i64).wrapping_add(base), w[3 + k + i])).collect();
            Some(ColStats { rows: col.rows() as u64, ndv, sq: w[1], cube: w[2], top })
        })
        .collect())
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
        let (Some(counts), Some(scatter)) = (&self.counts, &self.scatter) else {
            return Ok(self);
        };
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
        let Some(space) = &self.space else {
            return Vec::new();
        };
        // SAFETY: `total` pairs, written by a completed scatter into space for at least that many.
        let flat = unsafe { std::slice::from_raw_parts(space.buffer.contents().cast::<[u32; 2]>(), total) };
        flat.iter().map(|p| (p[0], p[1])).collect()
    }
}

/// The rows a one-dispatch sort takes at most (`small_sort`): one threadgroup, the keys and their rows in
/// threadgroup memory (20 KB: 2,048 `long` keys and `ushort` rows).
pub const SMALL: usize = 2048;


// ---- The small path (decision 0100 §9): a sort in one dispatch, a probe in one, raw columns, one arena. ----

impl Column {
    /// `col` on the device as raw 64-bit values (base 0): no span is taken and nothing narrowed. A staged column is
    /// its own shared buffer, read in place; a host column is copied once.
    pub fn raw(col: &crate::col::Col) -> Result<Column, String> {
        let rows = col.len();
        if let Some(p) = col.shared() {
            let buffer = p.buffer().clone();
            return Ok(Column { buffer: Held { _lease: Lease::Col(col.clone()), buffer }, width: Width::W64, base: 0, bits: 64, rows });
        }
        let buffer = Held::new((rows * 8).max(8))?;
        // SAFETY: the buffer holds `rows` longs, and nothing else refers to it yet.
        unsafe { std::ptr::copy_nonoverlapping(col.as_ptr(), buffer.buffer.contents().cast::<i64>(), rows) };
        Ok(Column { buffer, width: Width::W64, base: 0, bits: 64, rows })
    }
}


/// Compiles every pipeline a batch's joins use ahead of their first use, as a job on the pool: the small path's,
/// the scans', the radix sort's of each width with the gathers, and the tile pairs' sort. A job that needs one
/// still being compiled waits for it (`pipeline wait`) rather than compiling it again. A panic leaves them to be
/// compiled where they are needed (a job's panic would end the process).
pub fn prewarm() {
    crate::exec::configure(false);
    // In its caller's run (decision 0442): the pipelines are the caller's machine's.
    let run = rerail_context::job_run();
    rayon::spawn(move || {
        // Entered before the job: `as_job` reads the job walls, the machine's.
        let _run = rerail_context::enter(run);
        crate::exec::as_job("the GPU's prewarm", || {
            let _ = std::panic::catch_unwind(|| objc::rc::autoreleasepool(|| {
                let _ = fused_pipe("sort_all");
                let _ = fused_pipe("probe_all");
                let _ = pipeline("scan", "scan_block", || SCAN_SOURCE.to_owned());
                let _ = pipeline("scan", "scan_add", || SCAN_SOURCE.to_owned());
                for w in [Width::W64, Width::W32, Width::W16] {
                    let _ = sort_pipes(w);
                    let _ = gather_pipe(w);
                }
                super::tile::prewarm();
            }));
        })
    });
}

/// A batch's pairs: one buffer the probes reserve their threadgroups' pairs in with one atomic each, so no probe
/// needs a second command buffer to learn its size. `cap` is what the host expects in all; a probe whose pairs do
/// not fit raises the flag, and only it runs again, into space of its exact size.
pub struct Arena {
    /// Shared with the views of its probes' pairs, which keep it.
    pairs: std::sync::Arc<Held>,
    head: Held,
    flags: Held,
    blocks: Held,
    pub cap: usize,
    /// Blocks handed out so far.
    used: usize,
}

impl Arena {
    /// An arena for `cap` pairs, and `blocks` threadgroups in all.
    pub fn new(cap: usize, blocks: usize) -> Result<Arena, String> {
        let cap = cap.clamp(1, u32::MAX as usize / 2);
        Ok(Arena { pairs: std::sync::Arc::new(Held::new(8 * cap)?), head: Held::zeroed(4)?, flags: Held::zeroed(4)?, blocks: Held::new(8 * blocks.max(1))?, cap, used: 0 })
    }

    /// Whether any probe's pairs did not fit (after the command buffer completed).
    pub fn overflowed(&self) -> bool {
        // SAFETY: one word, zeroed by the host and written by the device, its command buffer completed.
        unsafe { *self.flags.buffer.contents().cast::<u32>() != 0 }
    }
}

/// The threadgroups a probe of `nl` left rows takes (1,024 rows each).
pub fn probe_blocks(nl: usize) -> usize {
    nl.div_ceil(TILE).max(1)
}

/// A probe in one dispatch, its pairs in an arena.
pub struct ProbeOne {
    nl: usize,
    first: usize,
    blocks: usize,
}

impl ProbeOne {
    /// Whether this probe's pairs did not all fit, and how many it found (after the command buffer completed).
    pub fn spilled(&self, arena: &Arena) -> (bool, usize) {
        let b = self.read_blocks(arena);
        (b.iter().any(|&(at, n)| at == u32::MAX && n > 0), b.iter().map(|&(_, n)| n as usize).sum())
    }

    fn read_blocks(&self, arena: &Arena) -> Vec<(u32, u32)> {
        // SAFETY: this probe's blocks, written by the device (or the host for an empty probe), the command buffer
        // completed.
        let b = unsafe { std::slice::from_raw_parts(arena.blocks.buffer.contents().cast::<u32>(), 2 * (self.first + self.blocks)) };
        (self.first..self.first + self.blocks).map(|i| (b[2 * i], b[2 * i + 1])).collect()
    }

    /// The pairs read in place, in left row order (block by block), after the command buffer completed: a view of
    /// the arena, which it keeps. `None` when they did not fit.
    pub fn view(&self, arena: &Arena) -> Option<crate::kernel::PairView> {
        let mut segs: Vec<(u32, u32)> = Vec::new();
        for (at, n) in self.read_blocks(arena) {
            if n == 0 {
                continue;
            }
            if at == u32::MAX {
                return None;
            }
            // Blocks the device placed one after the other are one segment.
            match segs.last_mut() {
                Some((a, m)) if *a + *m == at => *m += n,
                _ => segs.push((at, n)),
            }
        }
        let keep: std::sync::Arc<dyn std::any::Any + Send + Sync> = arena.pairs.clone();
        // SAFETY: each segment is a block's pairs, inside the arena (the device checked the capacity), written by the
        // completed command buffer and never written again: the arena is this batch's alone.
        Some(unsafe { crate::kernel::PairView::new(keep, arena.pairs.buffer.contents().cast::<[u32; 2]>(), segs) })
    }

    /// The pairs, in left row order (block by block), after the command buffer completed. `None` when they did not
    /// fit.
    pub fn pairs(&self, arena: &Arena) -> Option<Vec<(u32, u32)>> {
        let blocks = self.read_blocks(arena);
        let total: usize = blocks.iter().map(|&(_, n)| n as usize).sum();
        let mut out = Vec::with_capacity(total);
        for (at, n) in blocks {
            if n == 0 {
                continue;
            }
            if at == u32::MAX {
                return None;
            }
            // SAFETY: `n` pairs at `at`, inside the arena (the device checked the capacity), written by it.
            let flat = unsafe { std::slice::from_raw_parts(arena.pairs.buffer.contents().cast::<[u32; 2]>().add(at as usize), n as usize) };
            out.extend(flat.iter().map(|p| (p[0], p[1])));
        }
        let _ = self.nl;
        Some(out)
    }
}

// ---- One dispatch for a batch's small sorts, one for its probes (decision 0100 §9). ----
//
// Each job's buffers are reached through device pointers in a table the host writes (their `gpu_address`), and
// made resident with one `use_resources` call: a batch of any number of joins costs the host two dispatches.

/// The source of the fused sort and probe: `sort_all` (a threadgroup a small sort) and `probe_all` (a threadgroup
/// a block of 1,024 left rows of any probe).
const FUSED_SOURCE: &str = r#"#include <metal_stdlib>
using namespace metal;

struct SortJob {
    device const long* c[3];
    device uint* perm;
    device uint* starts;
    device ulong* rkeys;
    device uint* nruns;
    device long* o[2];
    uint n, m, k, pad;
};

struct ProbeJob {
    device const uchar* rkeys;
    device const uint* starts;
    device const uint* nruns;
    device const uint* perm;
    device const ulong* l[3];
    device const uchar* r[2];
    long lbase[3];
    long kbase;
    long rbase[2];
    uint nl, k, kw, pad;
    uint rw[2];
    uint pad2[2];
};

static inline ulong ld(device const uchar* p, uint w, uint i) {
    switch (w) {
        case 2: return ((device const ushort*)p)[i];
        case 4: return ((device const uint*)p)[i];
        default: return ((device const ulong*)p)[i];
    }
}

kernel void sort_all(device const SortJob* jobs [[buffer(0)]],
                     uint g [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]],
                     uint lane [[thread_index_in_simdgroup]], uint sg [[simdgroup_index_in_threadgroup]]) {
    threadgroup long K[2048];
    threadgroup ushort I[2048];
    threadgroup uint part[8];
    device const SortJob& j = jobs[g];
    uint n = j.n, m = j.m;
    for (uint i = t; i < m; i += 256) {
        bool live = i < n;
        K[i] = live ? j.c[0][i] : LONG_MAX;
        I[i] = live ? ushort(i) : ushort(0xFFFF);
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint size = 2; size <= m; size <<= 1) {
        for (uint stride = size >> 1; stride > 0; stride >>= 1) {
            for (uint q = t; q < m / 2; q += 256) {
                uint lo = 2 * q - (q & (stride - 1));
                uint hi = lo + stride;
                bool up = (lo & size) == 0;
                long ka = K[lo], kb = K[hi];
                ushort ia = I[lo], ib = I[hi];
                bool gt = ka > kb || (ka == kb && ia > ib);
                if (gt == up) { K[lo] = kb; K[hi] = ka; I[lo] = ib; I[hi] = ia; }
            }
            threadgroup_barrier(mem_flags::mem_threadgroup);
        }
    }
    uint i0 = t * 8;
    uint mine = 0, heads = 0;
    for (uint q = 0; q < 8; q++) {
        uint i = i0 + q;
        bool h = i < n && (i == 0 || K[i] != K[i - 1]);
        heads |= uint(h) << q;
        mine += uint(h);
    }
    uint inc = simd_prefix_inclusive_sum(mine);
    if (lane == 31) part[sg] = inc;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint off = 0;
    for (uint s = 0; s < sg; s++) off += part[s];
    uint run = off + inc - mine;
    for (uint q = 0; q < 8; q++) {
        uint i = i0 + q;
        if (i >= n) break;
        uint pi = I[i];
        j.perm[i] = pi;
        for (uint c = 1; c < j.k; c++) j.o[c - 1][i] = j.c[c][pi];
        if ((heads >> q) & 1) { j.starts[run] = i; j.rkeys[run] = ulong(K[i]) ^ 0x8000000000000000ul; run++; }
    }
    if (t == 255) {
        uint total = 0;
        for (uint s = 0; s < 8; s++) total += part[s];
        j.nruns[0] = total;
        j.starts[total] = n;
    }
}

static inline bool find(ulong key, device const ProbeJob& j, uint runs, thread uint& lo) {
    lo = 0;
    uint hi = runs;
    for (uint s = 0; s < 34 && lo < hi; s++) {
        uint mid = (lo + hi) / 2;
        long v = long(ld(j.rkeys, j.kw, mid) + ulong(j.kbase));
        if (v < long(key)) lo = mid + 1; else hi = mid;
    }
    return lo < runs && long(ld(j.rkeys, j.kw, lo) + ulong(j.kbase)) == long(key);
}

static inline bool same(device const ProbeJob& j, uint r, uint q) {
    for (uint c = 1; c < j.k; c++) {
        if (j.l[c][r] + ulong(j.lbase[c]) != ld(j.r[c - 1], j.rw[c - 1], q) + ulong(j.rbase[c - 1])) return false;
    }
    return true;
}

// The table: per threadgroup, (job, the block's number in the arena's blocks, its number in the job). p: the arena's
// capacity in pairs.
kernel void probe_all(device const ProbeJob* jobs [[buffer(0)]], device const uint4* table [[buffer(1)]],
                      device uint* arena [[buffer(2)]], device atomic_uint* head [[buffer(3)]], device uint* blocks [[buffer(4)]],
                      device atomic_uint* flags [[buffer(5)]], constant uint* p [[buffer(6)]],
                      uint g [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]],
                      uint lane [[thread_index_in_simdgroup]], uint sg [[simdgroup_index_in_threadgroup]]) {
    threadgroup uint part[8];
    threadgroup uint place[2];
    uint4 e = table[g];
    device const ProbeJob& j = jobs[e.x];
    uint nl = j.nl;
    uint runs = j.nruns[0];
    uint at = e.z * 1024 + t * 4;
    uint run[4];
    uint c[4];
    uint mine = 0;
    for (uint i = 0; i < 4; i++) {
        uint r = at + i;
        c[i] = 0;
        run[i] = 0xFFFFFFFFu;
        uint lo;
        if (r < nl && find(j.l[0][r] + ulong(j.lbase[0]), j, runs, lo)) {
            run[i] = lo;
            for (uint q = j.starts[lo]; q < j.starts[lo + 1]; q++) c[i] += uint(same(j, r, q));
        }
        mine += c[i];
    }
    uint inc = simd_prefix_inclusive_sum(mine);
    if (lane == 31) part[sg] = inc;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint off = 0;
    for (uint s = 0; s < sg; s++) off += part[s];
    if (t == 255) {
        uint total = off + inc;
        uint b = total > 0 ? atomic_fetch_add_explicit(head, total, memory_order_relaxed) : 0;
        bool fits = ulong(b) + ulong(total) <= ulong(p[0]);
        if (!fits) atomic_store_explicit(flags, 1, memory_order_relaxed);
        blocks[2 * e.y] = fits ? b : 0xFFFFFFFFu;
        blocks[2 * e.y + 1] = total;
        place[0] = b;
        place[1] = uint(fits);
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (place[1] == 0) return;
    uint o = place[0] + off + inc - mine;
    for (uint i = 0; i < 4; i++) {
        if (run[i] == 0xFFFFFFFFu) continue;
        uint r = at + i;
        for (uint q = j.starts[run[i]]; q < j.starts[run[i] + 1]; q++) {
            if (same(j, r, q)) { arena[2 * o] = r; arena[2 * o + 1] = j.perm[q]; o++; }
        }
    }
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SortDesc {
    c: [u64; 3],
    perm: u64,
    starts: u64,
    rkeys: u64,
    nruns: u64,
    o: [u64; 2],
    n: u32,
    m: u32,
    k: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProbeDesc {
    rkeys: u64,
    starts: u64,
    nruns: u64,
    perm: u64,
    l: [u64; 3],
    r: [u64; 2],
    lbase: [i64; 3],
    kbase: i64,
    rbase: [i64; 2],
    nl: u32,
    k: u32,
    kw: u32,
    pad: u32,
    rw: [u32; 2],
    pad2: [u32; 2],
}

fn fused_pipe(name: &'static str) -> Result<ComputePipelineState, String> {
    pipeline("fused", name, || FUSED_SOURCE.to_owned())
}

/// A batch's small sorts and probes, described as they are encoded and dispatched together at the end
/// ([`Fused::dispatch`]).
#[derive(Default)]
pub struct Fused {
    sorts: Vec<SortDesc>,
    probes: Vec<ProbeDesc>,
    /// Per threadgroup of the probe dispatch: (probe, arena block, block in the probe, 0).
    table: Vec<[u32; 4]>,
    /// Every buffer the tables point to, made resident with one call.
    used: Vec<Buffer>,
}

impl Fused {
    fn addr(&mut self, b: &Buffer) -> u64 {
        self.used.push(b.clone());
        b.gpu_address()
    }

    /// `columns` (raw 64-bit, at most `SMALL` rows, one to three) to be sorted by column 0 in the batch's sort
    /// dispatch: the sort, ready for probes encoded after it. Its keys are offsets from `long`'s least.
    pub fn sort(&mut self, columns: Vec<Column>) -> Result<(Keyed, Vec<Column>), String> {
        let k = columns.len();
        let n = columns.first().ok_or("a sort of no columns")?.rows;
        if !(1..=3).contains(&k) || n > SMALL || columns.iter().any(|c| c.width != Width::W64 || c.base != 0 || c.rows != n) {
            return Err(format!("a small sort of {k} columns of {n} rows"));
        }
        let perm = Held::new(4 * n.max(1))?;
        let starts = Held::new(4 * (n + 1))?;
        let rkeys = Held::new(8 * n.max(1))?;
        let nruns = Held::new(4)?;
        let others: Vec<Column> = (1..k).map(|_| Held::new(8 * n.max(1)).map(|b| Column { buffer: b, width: Width::W64, base: 0, bits: 64, rows: n })).collect::<Result<_, _>>()?;
        let mut d = SortDesc { n: n as u32, m: n.next_power_of_two().max(2) as u32, k: k as u32, ..Default::default() };
        for (i, c) in columns.iter().enumerate() {
            d.c[i] = self.addr(&c.buffer.buffer);
        }
        d.perm = self.addr(&perm.buffer);
        d.starts = self.addr(&starts.buffer);
        d.rkeys = self.addr(&rkeys.buffer);
        d.nruns = self.addr(&nruns.buffer);
        for (i, c) in others.iter().enumerate() {
            d.o[i] = self.addr(&c.buffer.buffer);
        }
        self.sorts.push(d);
        let bytes = 4 * n + (n + 1) * 12 + others.iter().map(Column::bytes).sum::<usize>();
        let keyed = Keyed {
            rows: n,
            key: Width::W64,
            base: i64::MIN,
            perm,
            starts,
            rkeys,
            nruns,
            others,
            pending: std::sync::Mutex::new(None),
            runs: std::sync::atomic::AtomicUsize::new(usize::MAX),
            gpu: std::sync::atomic::AtomicU64::new(0),
            bytes,
        };
        Ok((keyed, columns))
    }

    /// The probe of `right` by `left` (raw columns), its pairs to `arena`, in the batch's probe dispatch.
    pub fn probe(&mut self, left: &[Column], right: &Keyed, arena: &mut Arena) -> Result<ProbeOne, String> {
        let k = left.len();
        if k == 0 || k > 3 || k != 1 + right.others.len() || left.iter().any(|c| c.width != Width::W64) {
            return Err(format!("a fused probe of {k} keys against a sort of {}", 1 + right.others.len()));
        }
        let nl = left[0].rows;
        let blocks = probe_blocks(nl);
        let first = arena.used;
        arena.used += blocks;
        if nl == 0 || right.rows == 0 {
            // SAFETY: the blocks' words for this probe, the host's alone before the command buffer is committed.
            let b = unsafe { std::slice::from_raw_parts_mut(arena.blocks.buffer.contents().cast::<u32>(), 2 * arena.used) };
            b[2 * first..2 * (first + blocks)].fill(0);
            return Ok(ProbeOne { nl, first, blocks });
        }
        let mut d = ProbeDesc { nl: nl as u32, k: k as u32, kw: right.key.bytes() as u32, kbase: right.base, ..Default::default() };
        d.rkeys = self.addr(&right.rkeys.buffer);
        d.starts = self.addr(&right.starts.buffer);
        d.nruns = self.addr(&right.nruns.buffer);
        d.perm = self.addr(&right.perm.buffer);
        for (i, c) in left.iter().enumerate() {
            d.l[i] = self.addr(&c.buffer.buffer);
            d.lbase[i] = c.base;
        }
        for (i, c) in right.others.iter().enumerate() {
            d.r[i] = self.addr(&c.buffer.buffer);
            d.rbase[i] = c.base;
            d.rw[i] = c.width.bytes() as u32;
        }
        let at = self.probes.len() as u32;
        self.probes.push(d);
        self.table.extend((0..blocks).map(|b| [at, (first + b) as u32, b as u32, 0]));
        Ok(ProbeOne { nl, first, blocks })
    }

    /// Encodes the batch's sort dispatch, then its probe dispatch (a serial encoder: the probes see the sorts), and
    /// keeps the tables alive in `keep` until the command buffer completes.
    pub fn dispatch(self, pass: &Pass<'_>, arena: &Arena, keep: &mut Vec<Held>) -> Result<(), String> {
        let table = |bytes: &[u8]| -> Result<Held, String> {
            let h = Held::new(bytes.len().max(16))?;
            // SAFETY: the buffer holds at least `bytes`, and nothing else refers to it yet.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), h.buffer.contents().cast::<u8>(), bytes.len()) };
            Ok(h)
        };
        // SAFETY: plain `repr(C)` values, read as their bytes.
        let raw = |p: *const u8, n: usize| unsafe { std::slice::from_raw_parts(p, n) };
        let mut used: Vec<&metal::ResourceRef> = self.used.iter().map(|b| &***b).collect();
        used.extend([&**arena.pairs.buffer, &**arena.head.buffer, &**arena.blocks.buffer, &**arena.flags.buffer]);
        pass.encoder.use_resources(&used, metal::MTLResourceUsage::Read | metal::MTLResourceUsage::Write);
        if !self.sorts.is_empty() {
            let pipe = fused_pipe("sort_all")?;
            let t = table(raw(self.sorts.as_ptr().cast(), std::mem::size_of_val(&self.sorts[..])))?;
            pass.run(&pipe, &[&t.buffer], &[], &[], self.sorts.len());
            keep.push(t);
        }
        if !self.table.is_empty() {
            let pipe = fused_pipe("probe_all")?;
            let jobs = table(raw(self.probes.as_ptr().cast(), std::mem::size_of_val(&self.probes[..])))?;
            let tab = table(raw(self.table.as_ptr().cast(), std::mem::size_of_val(&self.table[..])))?;
            pass.run(
                &pipe,
                &[&jobs.buffer, &tab.buffer, &arena.pairs.buffer, &arena.head.buffer, &arena.blocks.buffer, &arena.flags.buffer],
                &[arena.cap as u32],
                &[],
                self.table.len(),
            );
            keep.push(jobs);
            keep.push(tab);
        }
        Ok(())
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

    /// Pairs sorted on the device come out in (first, second) order: keys of 32 bits and of 64.
    #[test]
    fn pairs_sorted_on_the_device_are_in_row_order() {
        if shared::get().is_none() {
            return;
        }
        for (n, fb, sb) in [(5000usize, 12u32, 13u32), (70_000, 20, 30), (1, 0, 0), (3000, 32, 32)] {
            let firsts = rows(n, 7 + n as u64, 1i64 << fb.min(31), 0);
            let seconds = rows(n, 11 + n as u64, 1i64 << sb.min(31), 0);
            let want_in: Vec<[u32; 2]> = firsts.iter().zip(&seconds).map(|(&a, &b)| [a as u32, b as u32]).collect();
            let pairs = Held::of(want_in.iter().copied()).unwrap();
            let out = Held::new(8 * n).unwrap();
            let queue = queue().unwrap();
            let command = queue.new_command_buffer();
            let pass = Pass::new(command);
            let mut keep = Vec::new();
            encode_sorted_pairs(&pass, &pairs, n, fb, sb, &out, &mut keep).unwrap();
            pass.end();
            commit_wait(command).unwrap();
            let got: Vec<(u32, u32)> = read(&out, 2 * n).chunks_exact(2).map(|p| (p[0], p[1])).collect();
            let mut want: Vec<(u32, u32)> = want_in.iter().map(|p| (p[0], p[1])).collect();
            want.sort_unstable();
            assert_eq!(got, want, "{n} pairs of {fb} and {sb} bits");
        }
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

/// Decision 0440 step 5's speed gate: the UKL library's scan and radix sort (`rerail_gpu::library`), on Metal, against
/// this lane's own kernels on the same input: GPU time of the command buffer, warmed, the median of nine runs taken in
/// turn. A measurement: `--ignored`.
#[cfg(test)]
mod library_speed {
    use super::*;
    use rerail_gpu::device::ComputeQueue;
    use rerail_gpu::kernels::{alloc, buffer_of};
    use rerail_gpu::library::{radix_sort_pairs, scan_exclusive_u32};

    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    }

    /// The lane's time and the library's, in ms, of `lane` and `lib` (each returns its GPU seconds).
    fn race(what: &str, mut lane: impl FnMut() -> f64, mut lib: impl FnMut() -> f64) -> (f64, f64) {
        for _ in 0..3 {
            lane();
            lib();
        }
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for _ in 0..9 {
            a.push(lane());
            b.push(lib());
        }
        let (a, b) = (median(a) * 1e3, median(b) * 1e3);
        eprintln!("{what}: lane {a:.3} ms, UKL library {b:.3} ms ({:.2}x)", b / a);
        (a, b)
    }

    #[test]
    #[ignore = "a measurement"]
    fn the_library_is_no_slower_than_the_lane() {
        let shared = rerail_gpu::shared::get().expect("a Metal device");
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };

        // A scan of 16M values.
        let n = 1usize << 24;
        let vals: Vec<u32> = (0..n).map(|_| (next() % 7) as u32).collect();
        let data = Held::of(vals.iter().copied()).unwrap();
        let lane_scan = Scan::owned(n).unwrap();
        let (inb, out) = (buffer_of(&vals).unwrap(), alloc::<u32>(n + 1).unwrap());
        let (a, b) = race(
            "scan of 16M u32",
            || {
                let command = queue().unwrap().new_command_buffer().to_owned();
                let pass = Pass::new(&command);
                lane_scan.encode(&pass, &data.buffer).unwrap();
                pass.end();
                command.commit();
                command.wait_until_completed();
                gpu_seconds(&command)
            },
            || {
                let mut q = ComputeQueue::new(&shared.queue);
                scan_exclusive_u32(&q, &inb, &out, n as u32).unwrap();
                let e = q.flush();
                e.await_().unwrap();
                e.gpu_seconds()
            },
        );
        assert!(b <= a * 1.1, "the library's scan is slower than the lane's");

        // A radix sort of 4M 32-bit keys with their rows: four passes of histogram, scan and scatter each.
        let n = 1usize << 22;
        let keys: Vec<u32> = (0..n).map(|_| next() as u32).collect();
        let rows: Vec<u32> = (0..n as u32).collect();
        let [_, hist, scatter, ..] = sort_pipes(Width::W32).unwrap();
        let (ka, kb) = (Held::of(keys.iter().copied()).unwrap(), Held::new(4 * n).unwrap());
        let (ia, ib) = (Held::of(rows.iter().copied()).unwrap(), Held::new(4 * n).unwrap());
        let tiles = n.div_ceil(TILE);
        let histogram = Held::new(4 * 256 * tiles).unwrap();
        let hscan = Scan::owned(256 * tiles).unwrap();
        let (lk, lv) = (buffer_of(&keys).unwrap(), buffer_of(&rows).unwrap());
        let (tk, tv) = (alloc::<u32>(n).unwrap(), alloc::<u32>(n).unwrap());
        let (a, b) = race(
            "radix sort of 4M u32 keys and rows",
            || {
                let command = queue().unwrap().new_command_buffer().to_owned();
                let pass = Pass::new(&command);
                for p in 0..4u32 {
                    let (ki, ii, ko, io) = if p % 2 == 0 { (&ka, &ia, &kb, &ib) } else { (&kb, &ib, &ka, &ia) };
                    pass.run(&hist, &[&ki.buffer, &histogram.buffer], &[n as u32, 8 * p], &[], tiles);
                    hscan.encode(&pass, &histogram.buffer).unwrap();
                    pass.run(&scatter, &[&ki.buffer, &ii.buffer, &histogram.buffer, &ko.buffer, &io.buffer], &[n as u32, 8 * p], &[], tiles);
                }
                pass.end();
                command.commit();
                command.wait_until_completed();
                gpu_seconds(&command)
            },
            || {
                let mut q = ComputeQueue::new(&shared.queue);
                radix_sort_pairs::<u32>(&q, &lk, &lv, &tk, &tv, n as u32, 32).unwrap();
                let e = q.flush();
                e.await_().unwrap();
                e.gpu_seconds()
            },
        );
        assert!(b <= a * 1.1, "the library's sort is slower than the lane's");
    }
}
