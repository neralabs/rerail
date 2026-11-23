//! Relations sorted by a key on the device, and the equi-joins that probe them (decision 0095 §4, §5): the GPU
//! lane's `join` kernel, moved into the executor from the engine (decision 0100 §1). The kernels are UKL (decision
//! 0440 step C), on the process's device whatever its backend (`shared::layer`).
//!
//! **A column on the device** (`Column`) is each value's offset from the column's least value, in 16, 32 or 64
//! bits: as narrow as the column's span lets it be, exactly. The host's part is one pass for the span and one that
//! writes the offsets straight into the shared buffer (`rows_sort::span`); nothing is sorted or copied twice.
//!
//! **A relation sorted by its key** (`Keyed`), all of it on the device (`join_sort.uglykl`):
//! - `sort_init`, then the UKL library's stable LSD radix sort (`rerail_gpu::library::radix_sort_pairs_in`, decision
//!   0440 step D), a pass per 8-bit digit of the key's span. No atomic decides an order.
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
//! Every scan is the UKL library's multi-level exclusive scan (`Scan`): 1,024 values a threadgroup, the threadgroups'
//! sums scanned again in place into their offsets. Every loop is bounded.
//!
//! **Launches** (`Pass`): every kernel runs threadgroups of 256 threads (`GROUP`) over its buffers, bound in the
//! order the kernel declares them, and one parameter block, its `ARGS` row `a`: a `JoinP` (`join_common.h`) of up
//! to eight words and eight longs, or a caller's own structure (`Pass::run_bytes`).

use super::launch;
use super::shared;
use crate::rows::{bits, span};
use rerail_device::compute_kernel::GridComputeFunctionArgumentDisposition;
use rerail_device::device::{Buffer, BufferRef, ComputeFunction, ComputeQueue, SharedEvent};
use rerail_gpu::library::{radix_sort_pairs_in, scan_exclusive_u32_in, RadixSpace, ScanSpace};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// What every join kernel's source starts with (`join_common.h`): `JoinP`, the threadgroup's prefix sum.
pub const COMMON: &str = include_str!("join_common.h");

/// The joins' fixed kernels: the scans, the pairs' keys, the column span, the fused sort and probe.
fn fixed_source() -> String {
    format!("{COMMON}{}{}", screen_reads(), include_str!("join.uglykl"))
}

/// The radix sort's, the gather's and the column statistics' kernels for a column of `col`, its sort word
/// `col.key()`.
fn sort_source(col: Width) -> String {
    format!("#define COL_T {}\n#define KEY_T {}\n{COMMON}{}", col.ty(), col.key().ty(), include_str!("join_sort.uglykl"))
}

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
    /// The word's type in UKL.
    pub fn ty(self) -> &'static str {
        match self {
            Width::W16 => "uint16_t",
            Width::W32 => "uint32_t",
            Width::W64 => "uint64_t",
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

/// The process's device in the device layer, whatever its backend.
pub fn device() -> Result<&'static rerail_device::device::Device, String> {
    shared::layer().map(|l| l.device).ok_or_else(|| "no device".to_owned())
}

/// Why a command the device failed failed: once a run for each reason (a wave's strata are refused to the CPU with
/// no more than `BackendFailure` otherwise).
pub fn say_failure(why: &str) {
    /// The reasons said: the run's own (decision 0442).
    #[derive(Default)]
    struct Said(Mutex<Vec<String>>);
    rerail_context::with_local::<Said, _>(|said| {
        let mut said = said.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !said.iter().any(|s| s == why) {
            eprintln!("rerail: a submission the device failed: {why}");
            said.push(why.to_owned());
        }
    });
}

/// The threads of a join kernel's group: every launch's (`Pass::run*`) but a SIMD group's (`Pass::run_bytes_simd`).
pub const GROUP: u64 = 256;
/// The threads of the process's device's SIMD groups (`UKL_SIMD_WIDTH`, as its kernels are compiled).
pub fn simd_group() -> usize {
    device().map_or(32, |d| d.simd_width() as usize)
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

/// A join kernel: its function, and its buffer arguments' names in the order it declares them (`Pass` binds a
/// launch's buffers in that order).
pub struct Pipe {
    function: Arc<ComputeFunction>,
    buffers: Vec<String>,
    /// Whether the kernel may write each buffer, as it declares it (`ROBUFFER` is read-only, decision 0491 step 5):
    /// how a launch pins what it binds.
    writes: Vec<bool>,
    /// Its name, as a pin names its operation.
    op: &'static str,
    /// Whether it takes the parameter block `a`.
    args: bool,
    name: String,
}

/// The join kernels made, by variant and name: the machine's device's (decision 0442: the machine's).
#[derive(Default)]
struct Pipes(Mutex<HashMap<String, Arc<Pipe>>>);

fn pipes() -> Arc<Pipes> {
    rerail_context::machine::<Pipes>()
}

/// The kernel `name` of the variant `variant`, its UKL source made by `source` the first time: kept for the
/// process, so every thread finds it compiled, and in the store of compiled kernels (decision 0395). A variant's
/// source is compiled once, into one library, and each of its kernels made from that library. A kernel that does not
/// compile is fatal (decision 0382): the run stops, naming it, and no caller runs another path on it.
pub fn pipeline(variant: &str, name: &str, source: impl FnOnce() -> String) -> Result<Arc<Pipe>, String> {
    let key = format!("join {variant} {name}");
    if let Some(p) = pipes().0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
        return Ok(Arc::clone(p));
    }
    device()?;
    let function = crate::score::phase("compile", || super::kernels::function_fixed(&source(), name, None));
    let mut args: Vec<_> = function.get_argument_info().to_vec();
    args.sort_by_key(|a| a.compute_function_arg_index);
    let bound: Vec<_> = args.iter().filter(|a| a.disposition == GridComputeFunctionArgumentDisposition::Buffer && a.name != "a").collect();
    let buffers = bound.iter().map(|a| a.name.clone()).collect();
    let writes = bound.iter().map(|a| a.ty.access != rerail_device::compute_kernel::MemoryRegionAccess::Read).collect();
    // One a kernel for the process (the pipes are the machine's, decision 0442): a pin's operation names it.
    let op: &'static str = Box::leak(name.to_owned().into_boxed_str());
    let pipe = Arc::new(Pipe { args: args.iter().any(|a| a.name == "a"), function, buffers, writes, op, name: name.to_owned() });
    Ok(Arc::clone(pipes().0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).entry(key).or_insert(pipe)))
}

/// A sort's kernels of a lead column of `width` around the library's radix sort: its keys and rows, and its runs.
fn sort_pipes(width: Width) -> Result<[Arc<Pipe>; 3], String> {
    let variant = format!("sort {width:?}");
    let pipe = |name: &str| pipeline(&variant, name, || sort_source(width));
    rerail_gpu::library::ready()?;
    Ok([pipe("sort_init")?, pipe("run_heads")?, pipe("run_compact")?])
}

/// The library's radix sort of `pair` (sort words of `key`, rows) by `bits` bits, `tmp` the other pair, encoded into
/// `pass`: the rows end in `pair` after an even number of passes, else in `tmp`.
fn radix_sort(pass: &Pass, key: Width, pair: [&Buffer; 2], tmp: [&Buffer; 2], n: usize, bits: u32, space: &RadixSpace) {
    let (pair, tmp): ([&BufferRef; 2], [&BufferRef; 2]) = ([pair[0], pair[1]], [tmp[0], tmp[1]]);
    pass.library(pair.iter().chain(&tmp).copied().chain(space.buffers()), |q| {
        let sorted = if key == Width::W64 { radix_sort_pairs_in::<u64>(q, pair, tmp, n as u32, bits, space) } else { radix_sort_pairs_in::<u32>(q, pair, tmp, n as u32, bits, space) };
        sorted.map(|(_, made)| made)
    });
}

fn gather_pipe(width: Width) -> Result<Arc<Pipe>, String> {
    pipeline(&format!("sort {width:?}"), "gather", || sort_source(width))
}

/// A width's sort kernels and its gather, made (`kernels_ahead`).
pub fn sort_and_gather(width: Width) -> Result<(), String> {
    sort_pipes(width)?;
    gather_pipe(width).map(drop)
}

/// One of the fixed kernels (`join.uglykl`).
pub fn fixed(name: &str) -> Result<Arc<Pipe>, String> {
    pipeline("fixed", name, fixed_source)
}

/// A committed submission, to wait for, and the pooled buffers it reads that nothing else holds: a pooled buffer is
/// written by the host when handed out again, and no host write is ordered after a device read, so they return to
/// the pool only once it completes.
pub struct Submitted {
    /// Its committed work, shared with its watchers (`watch`). Its fence (decision 0487) holds what the work reaches
    /// until the work completes: at the first wait that sees it complete, or when the last of them is dropped, which
    /// waits for it first.
    event: Arc<shared::Flushed>,
    reads: Vec<Column>,
    /// The host clock at its commit (`host_clock`); NaN for a second watcher of a submission another waits for
    /// (`watch`), which adds no timeline.
    committed: f64,
    /// Whether this one only watches (`watch`): it waits only while the owner has not.
    watcher: bool,
    /// Its pass's key in the evaluation's records (`launch::mark`), the owner's; none for a watcher.
    key: Option<Box<u8>>,
}

impl Submitted {
    /// Its launches' records marked completed (the owner's, once).
    fn completed(&mut self) {
        if let Some(key) = self.key.take() {
            launch::mark(std::ptr::from_ref::<u8>(&key) as usize, true);
        }
    }
}

impl Drop for Submitted {
    /// What it reads and reaches is held until its work completes: the owner let go unawaited waits for it here first,
    /// before its `reads` go back to the pool, whoever else watches it.
    fn drop(&mut self) {
        if !self.watcher && !self.event.fence().is_complete() {
            let _ = self.event.await_();
        }
        self.completed();
    }
}

/// The host clock Metal's command buffer times are on (`mach_absolute_time`, in seconds).
#[cfg(target_os = "macos")]
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

/// The host's monotonic clock in seconds (elsewhere than macOS no device timestamp shares it).
#[cfg(not(target_os = "macos"))]
pub fn host_clock() -> f64 {
    rerail_device::os::monotonic()
}

/// A completed Metal command buffer's timeline, as wall-only phases of the job (`crate::score`): commit to the
/// system's scheduling (`kernelStartTime`), to the device's start, the device's run, and its end to the host's wait
/// returning.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // `msg_send!` expands to a `cargo-clippy` cfg check.
pub fn add_timeline(command: &metal::CommandBufferRef, committed: f64, returned: f64) {
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
    /// Another wait for a committed submission: what a sort encoded into it waits for (`Keyed::settle`).
    pub fn watch(&self) -> Submitted {
        Submitted { event: Arc::clone(&self.event), reads: Vec::new(), committed: f64::NAN, watcher: true, key: None }
    }

    /// Keeps `columns` (what the submission reads) until it completes.
    pub fn reading(&mut self, columns: Vec<Column>) {
        self.reads.extend(columns);
    }

    /// Waits for it; its device seconds. The buffers it read return to the pool after.
    pub fn wait(self) -> f64 {
        self.wait_ok().0
    }

    /// `wait`, and whether the device ran it to the end: a submission the device failed (out of memory, a fault)
    /// completes too, its outputs partly written. A caller that keeps what it wrote must check.
    pub fn wait_ok(self) -> (f64, bool) {
        // A watcher's wait on a submission its owner awaited already would count its device time twice.
        if self.watcher && self.event.is_complete() {
            // Its owner awaited it: whether the device ran it to the end is the event's, read again.
            return (self.event.gpu_seconds(), self.event.event().await_().is_ok());
        }
        let done = crate::score::phase("gpu wait", || self.event.await_());
        if let Err(why) = &done {
            say_failure(why);
        }
        if !self.committed.is_nan() {
            let returned = host_clock();
            // Metal's command buffers give each part of the timeline.
            #[cfg(target_os = "macos")]
            let parts = self.event.event().metal().map(|command| add_timeline(command, self.committed, returned)).is_some();
            #[cfg(not(target_os = "macos"))]
            let parts = false;
            // Another backend's event gives only the device's seconds: the run, and the rest of the span as the wake.
            if !parts && done.is_ok() {
                let run = self.event.gpu_seconds();
                crate::score::add("gpu run", 0.0, run);
                crate::score::add("gpu wake", 0.0, (returned - self.committed - run).max(0.0));
            }
        }
        (self.event.gpu_seconds(), done.is_ok())
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

/// Gives this thread's scratch back (Q203): kept on the thread, the most each thread's sorts ever took stayed leased
/// to the end, 276 MB of a 1 GB budget on the M3's bench. To Metal, not the pool: a command buffer still running may
/// write it (returned to the pool, a sort still running wrote into the block the pool's next lease made of it: Q265's
/// check, `metal_join`), and Metal frees it once every command buffer that holds it completed.
pub fn scratch_return() -> usize {
    let n = SCRATCH.with_borrow_mut(|sets| {
        let mut n = 0;
        for b in sets.iter_mut().flat_map(|s| s.drain(..)).flatten() {
            n += b.length() as usize;
            b.release();
        }
        n
    });
    if n > 0 {
        shared::kept_change(shared::Kept::Scratch, -(n as isize));
    }
    n
}

/// Scratch slot `slot`, at least `bytes` long (grown to a power of two when it is shorter).
pub fn scratch(slot: usize, bytes: usize) -> Result<Buffer, String> {
    let set = SET.with(Cell::get);
    SCRATCH.with_borrow_mut(|sets| {
        let s = &mut sets[set];
        if s.len() <= slot {
            s.resize_with(slot + 1, || None);
        }
        if s[slot].as_ref().is_none_or(|b| (b.length() as usize) < bytes) {
            // The shorter one goes to Metal, not the pool: callers hold its buffer by handle (`scratch` hands out
            // clones) and may bind it again, and Metal keeps it while one does. Its ledger holds it for the queued work.
            if let Some(old) = s[slot].take() {
                shared::kept_change(shared::Kept::Scratch, -(old.length() as isize));
                old.release();
            }
            // Not zeroed: every kernel writes its scratch before reading it (a slot was always reused with the last
            // sort's contents), and a lease a stratum zeroed would cost a write of it all each stratum.
            let new = shared::lease_for_join(bytes.max(64), false)?;
            shared::kept_change(shared::Kept::Scratch, new.length() as isize);
            s[slot] = Some(new);
        }
        Ok(s[slot].as_ref().expect("a scratch buffer").device_buffer().clone())
    })
}

/// A device buffer of the one manager's pool (decision 0100 §2): a free one of its size class when there is one,
/// returned when dropped. A buffer the device has seen is resident already: a new one costs its creation and its
/// mapping at its first use, a pooled one neither. Its lease is held as a block's memory is (decision 0487 step 3, by
/// the lease's own ledger): a pass that binds it holds it, so dropped while queued work reads it, it goes back to the
/// pool only once that work completes, and the next lease's host writes are never ordered before it.
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
    /// A block's own device buffer (`rerail_blocks::Block::device_buffer`), kept bound while `keep` (what holds the
    /// block) lives: the device reads the block where it is, nothing copied (decision 0325).
    pub fn kept(buffer: Buffer, keep: Box<dyn std::any::Any + Send + Sync>) -> Held {
        Held { _lease: Lease::Kept(keep), buffer }
    }

    /// `kept`, holding the block by its own region too (`rerail_blocks::region::whole`, decision 0491 step 3): a
    /// kernel's bind of `buffer` pins that region, read or written as the kernel declares, so the block's frozenness
    /// and versions are its regions'.
    pub fn of_block(buffer: Buffer, region: rerail_blocks::region::Shared<u8>, keep: Box<dyn std::any::Any + Send + Sync>) -> Held {
        Held { _lease: Lease::Kept(Box::new((keep, region))), buffer }
    }

    /// A buffer of at least `bytes` (its contents are a previous user's: every reader writes first).
    pub fn new(bytes: usize) -> Result<Held, String> {
        device()?;
        Ok(Held::pooled(shared::lease_for_join(bytes, false)?))
    }

    /// A buffer of at least `bytes`, its first `bytes` zero.
    pub fn zeroed(bytes: usize) -> Result<Held, String> {
        device()?;
        Ok(Held::pooled(shared::lease_for_join(bytes, true)?))
    }

    /// `lease`, held by its own ledger while it lives (`shared::Pooled`).
    fn pooled(lease: shared::Pooled) -> Held {
        let buffer = lease.device_buffer().clone();
        Held { _lease: Lease::Pooled(lease), buffer }
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

/// A launch's grid: `groups` threadgroups of `threads`, or as many of `GROUP` as three words of a buffer say when it
/// runs (at most `most`).
enum Grid<'a> {
    Groups(usize, u64),
    Indirect { grid: &'a Buffer, offset: u64, most: usize },
}

/// The launches of one submission on the process's queue, in order: each sees the last's writes. Committed by
/// `commit`; dropped without it, nothing it holds runs. Each launch is charged to the evaluation's launch budget
/// (`launch::attempt`) before it is enqueued, and the commit is the budget's check (`launch::check_submission`): a
/// pass is a submission as a `rerail_metal::lane::Command` is, in the same ledger. Past the budget, or outside an
/// evaluation, is the hard failure (decision 0601), never an error to fall back on.
pub struct Pass {
    /// The process's queue (`shared::Queue`): its fence holds every buffer the launches reach (decision 0487).
    queue: shared::Queue,
    /// The launches charged so far (the evaluation's ledger holds the budget; this is the pass's own count, for
    /// its reports).
    dispatches: Cell<usize>,
    /// Buffers the launches after reach by address (`use_read`, `use_read_write`), bound by none, each with whether
    /// the launches write it.
    resident: RefCell<Vec<(Buffer, bool)>>,
    /// The first launch that could not be made: the commit's error.
    error: RefCell<Option<String>>,
    /// This submission's key in the evaluation's records (`launch::attempt`, `launch::mark`).
    key: Box<u8>,
}

impl Pass {
    pub fn new() -> Result<Pass, String> {
        Ok(Pass { queue: shared::Queue::new()?, dispatches: Cell::new(0), resident: RefCell::new(Vec::new()), error: RefCell::new(None), key: Box::new(0) })
    }

    fn pointer(&self) -> usize {
        std::ptr::from_ref::<u8>(&self.key) as usize
    }

    /// The launches made so far.
    pub fn dispatches(&self) -> usize {
        self.dispatches.get()
    }

    fn launch(&self, pipe: &Pipe, buffers: &[&Buffer], bytes: &[u8], grid: Grid<'_>) {
        self.launch_at(pipe, &buffers.iter().map(|b| (*b, 0)).collect::<Vec<_>>(), bytes, grid);
    }

    /// `launch`, each buffer bound at its offset.
    fn launch_at(&self, pipe: &Pipe, buffers: &[(&Buffer, u64)], bytes: &[u8], grid: Grid<'_>) {
        if self.error.borrow().is_some() {
            return;
        }
        for (i, (b, _)) in buffers.iter().enumerate() {
            self.holds(b, pipe.op, i, pipe.writes.get(i).copied().unwrap_or(true));
        }
        // Charged before it is enqueued: past the budget, the hard failure (decision 0601).
        let (shape, indirect) = match &grid {
            Grid::Groups(groups, threads) => ([*groups as u64 * threads, 1, 1], false),
            Grid::Indirect { .. } => ([0; 3], true),
        };
        let group = match &grid {
            Grid::Groups(_, threads) => [*threads, 1, 1],
            Grid::Indirect { .. } => [GROUP, 1, 1],
        };
        let bound = buffers.iter().map(|(b, _)| (b.contents() as usize, b.length())).collect();
        launch::attempt(self.pointer(), &pipe.name, shape, group, indirect, bound);
        let made = (|| {
            if buffers.len() != pipe.buffers.len() {
                return Err(format!("the kernel {} takes {} buffers, given {}", pipe.name, pipe.buffers.len(), buffers.len()));
            }
            let mut l = rerail_gpu::kernels::launch(&self.queue, &pipe.function);
            for (name, (b, at)) in pipe.buffers.iter().zip(buffers) {
                l = l.buffer_at(name, b, *at);
            }
            if pipe.args {
                l = l.bytes("a", bytes);
            }
            for (b, write) in self.resident.borrow().iter() {
                l = if *write { l.uses(b) } else { l.uses_read(b) };
            }
            match grid {
                Grid::Groups(groups, threads) => l.run(&[groups.max(1) as u64, 1, 1], &[threads, 1, 1]),
                Grid::Indirect { grid, offset, most } => l.run_indirect(grid, offset, &[most.max(1) as u64, 1, 1], &[GROUP, 1, 1]),
            }
            .map_err(|e| format!("the kernel {}: {e}", pipe.name))
        })();
        if let Err(e) = made {
            *self.error.borrow_mut() = Some(e);
        }
        self.dispatches.set(self.dispatches.get() + 1);
    }

    /// `groups` threadgroups of 256 threads over `buffers`, `bytes` its parameter block (the kernel's `ARGS`
    /// structure, laid out as the kernel's).
    pub fn run_bytes(&self, pipe: &Pipe, buffers: &[&Buffer], bytes: &[u8], groups: usize) {
        self.launch(pipe, buffers, bytes, Grid::Groups(groups, GROUP));
    }

    /// `run_bytes` in threadgroups of one SIMD group (`simd_group` threads), `groups` of them: a kernel
    /// whose SIMD group does one item's work, as long as the item says. The backends that make SIMD groups' operations
    /// of threadgroup barriers (OpenCL 1.2's) then never wait on another group's lanes.
    pub fn run_bytes_simd(&self, pipe: &Pipe, buffers: &[&Buffer], bytes: &[u8], groups: usize) {
        self.launch(pipe, buffers, bytes, Grid::Groups(groups, simd_group() as u64));
    }

    /// `run_bytes`, each buffer bound at its offset (a structure inside a block: decision 0478 step 2c).
    pub fn run_bytes_at(&self, pipe: &Pipe, buffers: &[(&Buffer, u64)], bytes: &[u8], groups: usize) {
        self.launch_at(pipe, buffers, bytes, Grid::Groups(groups, GROUP));
    }

    /// `run_bytes_simd`, each buffer bound at its offset.
    pub fn run_bytes_simd_at(&self, pipe: &Pipe, buffers: &[(&Buffer, u64)], bytes: &[u8], groups: usize) {
        self.launch_at(pipe, buffers, bytes, Grid::Groups(groups, simd_group() as u64));
    }

    /// `run_bytes`, its threadgroups the three words at `offset` of `grid` (written by the device before it), at
    /// most `most`.
    pub fn run_bytes_indirect(&self, pipe: &Pipe, buffers: &[&Buffer], bytes: &[u8], grid: &Buffer, offset: u64, most: usize) {
        self.launch(pipe, buffers, bytes, Grid::Indirect { grid, offset, most });
    }

    /// `run_bytes_indirect`, each buffer bound at its offset.
    pub fn run_bytes_indirect_at(&self, pipe: &Pipe, buffers: &[(&Buffer, u64)], bytes: &[u8], grid: &Buffer, offset: u64, most: usize) {
        self.launch_at(pipe, buffers, bytes, Grid::Indirect { grid, offset, most });
    }

    /// `run` of `params`, its threadgroups the three words at `offset` of `grid`, at most `most`.
    pub fn run_indirect(&self, pipe: &Pipe, buffers: &[&Buffer], params: &[u32], grid: &Buffer, offset: u64, most: usize) {
        self.launch(pipe, buffers, &join_p(params, &[]), Grid::Indirect { grid, offset, most });
    }

    /// `groups` threadgroups of 256 threads over `buffers`, `params` and `longs` its `JoinP`.
    pub fn run(&self, pipe: &Pipe, buffers: &[&Buffer], params: &[u32], longs: &[i64], groups: usize) {
        self.launch(pipe, buffers, &join_p(params, longs), Grid::Groups(groups, GROUP));
    }

    /// Launches of the UKL library's (`rerail_gpu::library`) on this pass's queue, in order with its own: `call`
    /// returns the launches it made, which count to the submission's budget. The library's kernels read only what
    /// they bind, `binds`: held until the work completes, as a launch's buffers are (decision 0491 step 2). An error
    /// is the commit's, as a launch's is.
    pub fn library<'b>(&self, binds: impl IntoIterator<Item = &'b Buffer>, call: impl FnOnce(&ComputeQueue) -> Result<usize, String>) {
        if self.error.borrow().is_some() {
            return;
        }
        for (i, b) in binds.into_iter().enumerate() {
            // The library's kernels write their outputs and scratch: each held as written.
            self.holds(b, "a UKL library launch", i, true);
        }
        // The library says its launches once it has enqueued them (its levels' count): charged then, past the
        // budget the hard failure.
        match call(&self.queue) {
            Ok(made) => {
                self.dispatches.set(self.dispatches.get() + made);
                launch::charge_made(self.pointer(), "a UKL library launch", made);
            }
            Err(e) => *self.error.borrow_mut() = Some(format!("the UKL library: {e}")),
        }
    }

    /// Makes `buffers` resident for the launches after it: buffers a kernel reaches by address (a view's directory
    /// and tapes, decision 0353 step 4d), bound by none.
    pub fn use_read(&self, buffers: &[&Buffer]) {
        for b in buffers {
            self.holds(b, "a resident read", 0, false);
        }
        self.resident.borrow_mut().extend(buffers.iter().map(|b| ((*b).clone(), false)));
    }

    /// Makes `buffer` resident for the launches after it, to be read and written: a cache a kernel's reads keep (a
    /// 16z batch's blocks, decision 0353 step 4b), reached by address.
    pub fn use_read_write(&self, buffer: &Buffer) {
        self.holds(buffer, "a resident write", 0, true);
        self.resident.borrow_mut().push((buffer.clone(), true));
    }

    /// Holds the launches after it until the host signals `event` with `value` or more (a gate's, `crate::col`).
    pub fn wait_for_event(&self, event: &SharedEvent, value: u64) {
        self.queue.wait_for_event(event, value);
    }

    /// The block or pooled lease `b` lies in (by its contents' address), held by this submission's fence until its
    /// work completes (decision 0491 step 2): what drops it meanwhile keeps it from the pool until then. A block's
    /// own region, while a handle holds it, is pinned as the kernel declares the argument, read or written (step 5). Every mapped
    /// buffer a kernel binds is one: a test build fails on one that is not, which nothing would keep.
    fn holds(&self, b: &Buffer, kernel: &'static str, arg: usize, write: bool) {
        let at = b.contents() as usize;
        let known = at == 0 || rerail_blocks::region::bind_address(self.queue.fence(), at, write, kernel);
        #[cfg(any(test, feature = "test-hooks"))]
        assert!(known, "{kernel}'s buffer {arg} ({} bytes) lies in no block or pooled lease the ledger knows: nothing holds it while the work runs (decision 0491 step 2)", b.length());
        let _ = (known, kernel, arg);
    }

    /// Commits the launches, checked against the evaluation's launch budget (`launch::check_submission`): what to
    /// wait for.
    pub fn commit(self) -> Result<Submitted, String> {
        if let Some(e) = self.error.take() {
            return Err(e);
        }
        launch::check_submission();
        let pointer = self.pointer();
        let event = crate::score::phase("gpu commit", || self.queue.flush());
        launch::mark(pointer, false);
        Ok(Submitted { event: Arc::new(event), reads: Vec::new(), committed: host_clock(), watcher: false, key: Some(self.key) })
    }

    /// Commits the launches and waits for them: their device seconds.
    /// `Err` when the device failed it (its outputs partly written, never read).
    pub fn commit_wait(self) -> Result<f64, String> {
        match self.commit()?.wait_ok() {
            (seconds, true) => Ok(seconds),
            (_, false) => Err("a submission the device failed".into()),
        }
    }
}

/// `params` and `longs` as a `JoinP` (`join_common.h`): eight words, then eight longs, the rest zero.
fn join_p(params: &[u32], longs: &[i64]) -> [u8; 96] {
    assert!(params.len() <= 8 && longs.len() <= 8, "a join kernel takes at most 8 words and 8 longs, given {} and {}", params.len(), longs.len());
    let mut b = [0u8; 96];
    for (i, p) in params.iter().enumerate() {
        b[4 * i..4 * i + 4].copy_from_slice(&p.to_ne_bytes());
    }
    for (i, l) in longs.iter().enumerate() {
        b[32 + 8 * i..40 + 8 * i].copy_from_slice(&l.to_ne_bytes());
    }
    b
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

/// A multi-level exclusive scan of `count` `uint`s, the UKL library's (`rerail_gpu::library`): in place, its buffer
/// holding `count + 1` values, the total after them. Its levels are this thread's scratch (`new`) or its own
/// (`owned`).
pub struct Scan {
    count: usize,
    space: ScanSpace,
    /// The levels' buffers when they are the scan's own (`owned`), not this thread's scratch.
    _owned: Vec<Held>,
}

impl Scan {
    pub fn new(count: usize) -> Result<Scan, String> {
        let space = ScanSpace::new_in(device_count(count)?, |level, words| scratch(SCAN + level, 4 * words))?;
        Ok(Scan { count, space, _owned: Vec::new() })
    }

    /// A scan whose levels are its own: several in one submission.
    pub fn owned(count: usize) -> Result<Scan, String> {
        let mut owned = Vec::new();
        let space = ScanSpace::new_in(device_count(count)?, |_, words| {
            let h = Held::new(4 * words)?;
            let b = h.buffer.clone();
            owned.push(h);
            Ok(b)
        })?;
        Ok(Scan { count, space, _owned: owned })
    }

    /// An owned scan's buffers, to keep until its submission completes.
    pub fn into_held(self) -> Vec<Held> {
        self._owned
    }

    /// The scan of `data`'s `count` values, in place: `count + 1` values, the total last.
    pub fn encode(&self, pass: &Pass, data: &Buffer) -> Result<(), String> {
        self.encode_into(pass, data, data)
    }

    /// The scan of `input`'s `count` values into `output` (`count + 1` values, the total last).
    pub fn encode_into(&self, pass: &Pass, input: &Buffer, output: &Buffer) -> Result<(), String> {
        pass.library([input, output].into_iter().chain(self.space.buffers()), |q| scan_exclusive_u32_in(q, input, output, self.count as u32, false, &self.space));
        Ok(())
    }
}

/// A count of values the device's scans and sorts take: 32 bits.
fn device_count(count: usize) -> Result<u32, String> {
    u32::try_from(count).map_err(|_| format!("{count} values: past the device's scans"))
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
    /// The screen of its keys (decision 0488): a probe tests a left key with a hash and one load before its search
    /// of the runs, and one the screen refuses has no pairs. Made in the sort's submission from the runs where the
    /// sort has `SCREENED_ROWS` rows, or a column's kept filter bound where it lies (`screen_by`).
    screen: Option<Screen>,
    /// The sort's submission while it may still run (`settle`), then its runs and device time (as bits).
    pending: Mutex<Option<Submitted>>,
    /// Its submission failed on the device (`settle`): its rows are partly written, so no probe reads them.
    failed: std::sync::atomic::AtomicBool,
    runs: std::sync::atomic::AtomicUsize,
    gpu: std::sync::atomic::AtomicU64,
    pub bytes: usize,
}

/// A screen (`Keyed::screen`): a blocked Bloom filter, the host's own format and reads (`rerail_blocks::bloom`: the
/// hash, the mask and the test written once in CLIF, `clif_bloom_*` in `<ukl/reads/relation.h>`, read by
/// `join_common.h`'s `screen_may_hold`), its descriptor `at` bytes into `held`'s buffer. The device's own, made from a
/// sort's runs (descriptor at 0, words at 64), or a block's: a column's kept filter, which the host's sketch keeps and
/// the device reads where it lies.
pub struct Screen {
    held: Held,
    at: u32,
}

/// No screen: a probe's `at` of none.
pub const NO_SCREEN: u32 = u32::MAX;

impl Screen {
    /// The device's own of `words` words (a power of two), zeroed, its descriptor written.
    fn own(words: usize) -> Result<Screen, String> {
        let held = Held::zeroed(64 + 8 * words)?;
        // SAFETY: the buffer's first 16 bytes, a `bloom::Bloom`'s span: its words 64 bytes on, `words` of them; the
        // host's alone until the sort's submission is committed.
        unsafe {
            let p = held.buffer.contents().cast::<u64>();
            *p = 64;
            *p.add(1) = words as u64;
        }
        Ok(Screen { held, at: 0 })
    }

    /// Its words' bytes (an own screen's; a kept one is its block's).
    fn bytes(&self) -> usize {
        if self.at == 0 { self.held.buffer.length() as usize } else { 0 }
    }
}

/// The screens' reads: the blocks' (`clif_bloom_*`, the host's own), a probe's test and a builder's add, in UKL:
/// one source for every backend, after `COMMON`.
fn screen_reads() -> String {
    format!(
        r#"{reads}
// Whether the screen (a `bloom::Bloom` at byte `at` of `f`; none at NO_SCREEN) may hold `key`: a hash and one load.
// A key it refuses is in no run (decision 0488).
UKL_INLINE bool join_may_hold(UKL_GLOBAL const uint8_t* f, uint32_t at, uint64_t key)
{{
  if (at == 0xffffffffu) return true;
  return clif_bloom_may(f, (uint64_t)at, clif_bloom_hash(f, 0ul, key)) != 0u;
}}

// Adds `key` to the screen at byte `at` of `f`: its word's two 32-bit halves set with atomics (bit b of 64-bit word
// w is bit b & 31 of 32-bit word 2w + b / 32, the host's bytes).
UKL_INLINE void join_screen_add(UKL_GLOBAL uint8_t* f, uint32_t at, uint64_t key)
{{
  uint64_t h = clif_bloom_hash(f, 0ul, key);
  uint64_t m = clif_bloom_mask(f, 0ul, h);
  UKL_GLOBAL const int64_t* d = (UKL_GLOBAL const int64_t*)(f + at);
  uint32_t w = (uint32_t)(h >> 32) & (uint32_t)((uint64_t)d[1] - 1ul);
  UKL_GLOBAL uint32_t* words = (UKL_GLOBAL uint32_t*)(f + (uint64_t)at + (uint64_t)d[0]);
  if ((uint32_t)m != 0u) ukl_atomic_or(&words[2u * w], (uint32_t)m);
  if ((uint32_t)(m >> 32) != 0u) ukl_atomic_or(&words[2u * w + 1u], (uint32_t)(m >> 32));
}}
"#,
        reads = rerail_blocks::RELATION_READS
    )
}

/// The kernel that makes a sort's screen: a thread a run, its key (the sort word plus `b0`) added. `p0`: the rows
/// (a bound on the runs).
fn screen_source(key: Width) -> String {
    format!(
        r#"{COMMON}{reads}
DEFINE_KERNEL2(join, screen_bloom,
               ROBUFFER, {k}, rkeys, "[a.p0]",
               ROBUFFER, uint32_t, nruns, "[1]",
               RWBUFFER, uint8_t, f, "[screen]",
               ARGS, JoinP, a, ,
               GID0, uint32_t, r, )
{{
  if (r >= a.p0 || r >= nruns[0]) return;
  join_screen_add(f, 0u, (uint64_t)rkeys[r] + (uint64_t)a.b0);
}}
"#,
        reads = screen_reads(),
        k = key.ty()
    )
}

/// The fewest rows a sort is screened for (`Keyed::screen`): below them a search of its runs is a few cached steps.
pub const SCREENED_ROWS: usize = 4096;

/// A sort's filter words: a byte a row (at most its distinct keys), a power of two, 256 B to 4 MiB (a sort of 4 M
/// rows holds 64 MB). `None` past 8 M rows, where it would hold under 4 bits a key: full, it refuses little, and a
/// probe pays its hash for nothing (4 M rows in 1 MiB: 4% slower).
fn screen_words(rows: usize) -> Option<usize> {
    const MOST: usize = 1 << 19;
    (rows <= MOST * 16).then(|| (rows / 8).next_power_of_two().clamp(32, MOST))
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
    /// The submission this sort was encoded into, committed and not waited for: `settle` waits for it.
    pub(super) fn pending_on(&self, s: &Submitted) {
        *self.pending.lock().expect("a sort's pending submission") = Some(s.watch());
    }

    /// Waits for the sort, once (a sort encoded into a caller's submission is the caller's to wait for).
    pub fn settle(&self) {
        use std::sync::atomic::Ordering::Relaxed;
        let mut pending = self.pending.lock().expect("a sort's pending submission");
        if let Some(s) = pending.take() {
            let (seconds, ok) = s.wait_ok();
            self.gpu.store(seconds.to_bits(), Relaxed);
            if !ok {
                self.failed.store(true, Relaxed);
            }
        }
        if self.failed.load(Relaxed) {
            self.runs.store(0, Relaxed);
            return;
        }
        if self.runs.load(Relaxed) == usize::MAX {
            // SAFETY: one word, written by `run_compact` (none when there are no rows), its submission completed.
            self.runs.store(if self.rows > 0 { (unsafe { *self.nruns.buffer.contents().cast::<u32>() }) as usize } else { 0 }, Relaxed);
        }
    }

    /// Whether its submission failed on the device (after it completes): its rows are not to be read.
    pub fn failed(&self) -> bool {
        self.settle();
        self.failed.load(std::sync::atomic::Ordering::Relaxed)
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
        // SAFETY: `run_compact` wrote `n` keys of the sort's word, its submission completed (`runs` settled it).
        (0..n).map(|i| unsafe {
            let k = match self.key {
                Width::W16 => *p.cast::<u16>().add(i) as u64,
                Width::W32 => *p.cast::<u32>().add(i) as u64,
                Width::W64 => *p.cast::<u64>().add(i),
            };
            (k as i64).wrapping_add(self.base)
        }).collect()
    }

    /// The sort's device seconds (after it completes).
    pub fn gpu_seconds(&self) -> f64 {
        self.settle();
        f64::from_bits(self.gpu.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Screens its probes by a column's kept filter (decision 0488): a `bloom::Bloom` at byte `at` of `buffer` (its
    /// block's), kept bound while `keep` lives, which must hold every key of its column 0. It takes the place of the
    /// screen the sort made, if any: the device reads the host's filter where it lies.
    pub fn screen_by(&mut self, buffer: Buffer, at: usize, keep: Box<dyn std::any::Any + Send + Sync>) {
        assert!(at > 0 && at < NO_SCREEN as usize, "a filter's descriptor in a block, past its header");
        self.bytes -= self.screen.as_ref().map_or(0, Screen::bytes);
        self.screen = Some(Screen { held: Held::kept(buffer, keep), at: at as u32 });
    }

    /// Whether its probes are screened.
    pub fn screened(&self) -> bool {
        self.screen.is_some()
    }

    /// `columns` sorted by column 0 on the device, in one submission, committed and not waited for: a probe of it
    /// on the process's queue runs after it. The intermediates are in scratch, the sort's order, runs and gathered
    /// columns in buffers of their own. The columns it reads are kept until it completes.
    pub fn sort(columns: Vec<Column>) -> Result<Keyed, String> {
        // An evaluation of the launch budget (`launch`), or the caller's when it is in one.
        let _evaluation = launch::LaunchScope::enter();
        let pass = Pass::new()?;
        let (mut keyed, reads) = Keyed::encode(&pass, columns)?;
        let mut pending = pass.commit()?;
        pending.reads = reads;
        keyed.pending = Mutex::new(Some(pending));
        Ok(keyed)
    }

    /// `columns` sorted by column 0, encoded into `pass` (what follows in it reads the sort). The columns it reads
    /// come back with it: they must outlive the submission. The sort is settled by whoever waits for the submission.
    pub fn encode(pass: &Pass, columns: Vec<Column>) -> Result<(Keyed, Vec<Column>), String> {
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
        // Its runs' keys' filter, one thread a run: read from the runs where the sort wrote them.
        let screen = if n >= SCREENED_ROWS && let Some(words) = screen_words(n) {
            let bloom = pipeline(&format!("screen {:?}", parts.key.bytes()), "screen_bloom", || screen_source(parts.key))?;
            let f = Screen::own(words)?;
            pass.run(&bloom, &[&parts.rkeys.buffer, &parts.nruns.buffer, &f.held.buffer], &[n as u32], &[base], groups(n));
            Some(f)
        } else {
            None
        };
        let bytes = bytes + screen.as_ref().map_or(0, Screen::bytes);
        let SortParts { perm, starts, rkeys, nruns, key } = parts;
        let keyed = Keyed { rows: n, key, base, perm, starts, rkeys, nruns, others, screen, pending: Mutex::new(None), failed: std::sync::atomic::AtomicBool::new(false), runs: std::sync::atomic::AtomicUsize::new(usize::MAX), gpu: std::sync::atomic::AtomicU64::new(0), bytes };
        Ok((keyed, reads))
    }
}

/// `n` pairs of 32-bit words (`pairs`, as `[first, second]`) sorted by (first, second), encoded into `pass`: `out`
/// gets them in that order. The second word takes `second_bits` bits and the first `first_bits`, so the sort's key
/// is `first << second_bits | second`, in 32 bits when they fit. The radix sort's passes alone, without runs: a
/// tile join's pairs come out in the kernel's order (left row, then right row) with no host sort (decision 0100 §9).
/// Its intermediates are kept in `keep` until the submission completes.
pub fn encode_sorted_pairs(pass: &Pass, pairs: &Held, n: usize, first_bits: u32, second_bits: u32, out: &Held, keep: &mut Vec<Held>) -> Result<(), String> {
    let (words, bits) = (pairs, first_bits + second_bits);
    if n == 0 {
        return Ok(());
    }
    let width = if bits <= 32 { Width::W32 } else { Width::W64 };
    let [init, ..] = sort_pipes(width)?;
    let narrow = fixed("pair_keys")?;
    let gather = gather_pipe(Width::W64)?;
    let kb = width.bytes();
    let lead = Held::new(kb * n)?;
    let (keys_a, keys_b) = (Held::new(kb * n)?, Held::new(kb * n)?);
    let passes = bits.div_ceil(8);
    let (idx_a, idx_b) = (Held::new(4 * n)?, Held::new(4 * n)?);
    let mut owned = Vec::new();
    let space = RadixSpace::new_in(device_count(n)?, |_, words| {
        let h = Held::new(4 * words)?;
        let b = h.buffer.clone();
        owned.push(h);
        Ok(b)
    })?;
    let nn = n as u32;
    // The words as sort keys of the sort's width (a 32-bit key when the bits fit).
    pass.run(&narrow, &[&words.buffer, &lead.buffer], &[nn, u32::from(width == Width::W64), second_bits], &[], groups(n));
    pass.run(&init, &[&lead.buffer, &keys_a.buffer, &idx_a.buffer], &[nn], &[], groups(n));
    radix_sort(pass, width, [&keys_a.buffer, &idx_a.buffer], [&keys_b.buffer, &idx_b.buffer], n, bits, &space);
    let order = if passes.is_multiple_of(2) { &idx_a } else { &idx_b };
    pass.run(&gather, &[&words.buffer, &order.buffer, &out.buffer], &[nn], &[], groups(n));
    keep.extend([lead, keys_a, keys_b, idx_a, idx_b]);
    keep.extend(owned);
    Ok(())
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
///
/// Every allocation that can fail is made before the first dispatch: a sort refused after encoding would drop
/// buffers (`perm`, `starts`) that work already in the submission writes, back to the pool at once.
pub fn encode_sort(pass: &Pass, lead: &Buffer, width: Width, bits: u32, n: usize) -> Result<SortParts, String> {
    let key = width.key();
    let [init, heads, compact] = sort_pipes(width)?;
    let kb = key.bytes();
    let (keys_a, keys_b) = (scratch(KEYS_A, kb * n)?, scratch(KEYS_B, kb * n)?);
    // The order ends in `idx_a` after an even number of passes: that one is the relation's own, the other scratch.
    let passes = bits.div_ceil(8);
    let perm = Held::new(4 * n)?;
    let other = scratch(IDX, 4 * n)?;
    let (idx_a, idx_b): (&Buffer, &Buffer) = if passes.is_multiple_of(2) { (&perm.buffer, &other) } else { (&other, &perm.buffer) };
    let starts = Held::new(4 * (n + 1))?;
    let rkeys = Held::new(kb * n)?;
    let nruns = Held::new(4)?;
    if n > 0 {
        // The histograms' scan and the heads' share scratch levels, one after the other in the command buffer. Both
        // are made here, before any dispatch, so its levels are at their size for both.
        let space = RadixSpace::new_in(device_count(n)?, |k, words| scratch(if k == 0 { HIST } else { SCAN + k - 1 }, 4 * words))?;
        let heads_buf = scratch(HEADS, 4 * (n + 1))?;
        let rscan = Scan::new(n)?;
        let nn = n as u32;
        pass.run(&init, &[lead, &keys_a, idx_a], &[nn], &[], groups(n));
        radix_sort(pass, key, [&keys_a, idx_a], [&keys_b, idx_b], n, bits, &space);
        let keys = if passes.is_multiple_of(2) { &keys_a } else { &keys_b };
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

/// A column for `column_stats_of`: values on the host, uploaded as narrow as their span lets them be; or 64-bit values
/// the device already holds (a resident relation's column, `rows` longs from long `at` of `buffer`), read in place.
pub enum StatCol<'a> {
    Host(&'a [i64]),
    /// Values the device holds: `rows` of them at value `at` of `buffer`, or, with `col`, column `col` of the
    /// `rerail_blocks` relation whose descriptor is `at` bytes into the block `buffer` binds (decision 0325).
    /// `far`: the block its relation's chain reaches past `buffer`'s (decision 0353 step 6), else `buffer`.
    Device { buffer: &'a Buffer, at: usize, rows: usize, col: Option<u32>, far: &'a Buffer },
}

impl StatCol<'_> {
    fn rows(&self) -> usize {
        match self {
            StatCol::Host(v) => v.len(),
            StatCol::Device { rows, .. } => *rows,
        }
    }
}

/// Decision 0209 §6, the GPU batch over host columns (`column_stats_of`).
pub fn column_stats(cols: &[&[i64]], k: usize) -> Result<Vec<ColStats>, String> {
    let cols: Vec<StatCol<'_>> = cols.iter().map(|c| StatCol::Host(c)).collect();
    column_stats_of(&cols, k)
}

/// Decision 0209 §6, the GPU batch: each column's statistics (`ColStats`) and its `k` most common values, all in one
/// submission. Per column: the radix sort with its runs (`encode_sort`), a value's count its run's length;
/// `col_stats` reduces the lengths a tile a threadgroup, `col_sum` the tiles'; a second sort of the lengths gives
/// the longest runs (`col_top`). The host reads one small buffer a column. Exact, as the CPU's batch is. A column's
/// keys are not made here: the statistics read the column's own (`logic::bloom::Keys`, decision 0488).
///
/// A column the device holds (`StatCol::Device`) is not read by the host: its span is found on the device
/// (`col_span`, `col_base`, `col_narrow`), and its keys sorted 32 bits wide. A column whose span passes 32 bits is made again in a second
/// submission, 64 bits wide.
pub fn column_stats_of(cols: &[StatCol<'_>], k: usize) -> Result<Vec<ColStats>, String> {
    device()?;
    // An evaluation of the launch budget (`launch`), or the caller's when it is in one: both rounds.
    let _evaluation = launch::LaunchScope::enter();
    let mut got = stats_round(cols, k, false)?;
    let wide: Vec<usize> = got.iter().enumerate().filter(|(_, g)| g.is_none()).map(|(i, _)| i).collect();
    if !wide.is_empty() {
        let again: Vec<StatCol<'_>> = wide.iter().map(|&i| match &cols[i] {
            StatCol::Device { buffer, at, rows, col, far } => StatCol::Device { buffer, at: *at, rows: *rows, col: *col, far },
            StatCol::Host(v) => StatCol::Host(v),
        }).collect();
        for (i, g) in wide.into_iter().zip(stats_round(&again, k, true)?) {
            got[i] = g;
        }
    }
    Ok(got.into_iter().map(|g| g.expect("made 64 bits wide")).collect())
}

/// One submission of `column_stats_of`; a device column whose span passes 32 bits is `None` unless `wide`.
fn stats_round(cols: &[StatCol<'_>], k: usize, wide: bool) -> Result<Vec<Option<ColStats>>, String> {
    let pass = Pass::new()?;
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
            StatCol::Device { buffer, at, rows: _, col, far } => {
                let spans = fixed("col_span")?;
                let based = fixed("col_base")?;
                let narrow = fixed("col_narrow")?;
                let (lo, hi) = (*at as u32, (*at >> 32) as u32);
                // A relation's column (its number + 1), or 0 for values at `at`.
                let rel_col = col.map_or(0, |c| c + 1);
                let part = Held::new(16 * tiles)?;
                let span = Held::new(16)?;
                let width = if wide { Width::W64 } else { Width::W32 };
                let keys = Held::new(width.bytes() * n)?;
                pass.run(&spans, &[buffer, &part.buffer, far], &[nn, lo, hi, rel_col], &[], tiles);
                pass.run(&based, &[&part.buffer, &span.buffer], &[tiles as u32], &[], 1);
                pass.run(&narrow, &[buffer, &span.buffer, &keys.buffer, far], &[nn, lo, hi, u32::from(wide), rel_col], &[], groups(n));
                let buf = keys.buffer.clone();
                keep.extend([part, keys]);
                (buf, width, if wide { 64 } else { 32 }, Err(span))
            }
        };
        let parts = encode_sort(&pass, &keys, width, bits, n)?;
        // The statistics' kernels of the sort word's variant (its `KEY_T`).
        let variant = format!("sort {:?}", parts.key);
        let stats = pipeline(&variant, "col_stats", || sort_source(parts.key))?;
        let sum = pipeline(&variant, "col_sum", || sort_source(parts.key))?;
        let top = pipeline(&variant, "col_top", || sort_source(parts.key))?;
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
    let mut done = pass.commit()?;
    done.reads = reads;
    let (_, ok) = done.wait_ok();
    drop(keep);
    if !ok {
        return Err("a submission the device failed".into());
    }
    Ok(outs
        .into_iter()
        .zip(cols)
        .map(|(o, col)| {
            let Some((out, base)) = o else { return Some(ColStats::default()) };
            let base = match base {
                Ok(b) => b,
                Err(span) => {
                    // SAFETY: two longs, written by `col_base`; the submission completed.
                    let s = unsafe { std::slice::from_raw_parts(span.buffer.contents().cast::<i64>(), 2) };
                    if s[1] != 0 && !wide {
                        return None;
                    }
                    s[0]
                }
            };
            // SAFETY: `3 + 2k` words, written by `col_stats` and `col_top`; the submission completed.
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

/// The probe join's kernels (UKL) for the left keys' widths, the right's sort word and other keys' widths. Buffers:
/// the runs' keys, starts and count, the right's order, the left keys, the right's other keys, then the counts (or
/// the offsets and the pairs); its `JoinP`: the left rows (and the pairs' space), and the keys' bases (the left's,
/// then the right's, key 0 first).
fn probe_source(left: &[Width], key: Width, right: &[Width]) -> String {
    let k = left.len();
    let kt = key.ty();
    let lrows: String = left.iter().enumerate().map(|(i, w)| format!("               ROBUFFER, {}, l{i}, \"[a.p0]\",\n", w.ty())).collect();
    let rrows: String = right.iter().enumerate().map(|(i, w)| format!("               ROBUFFER, {}, r{}, \"[rows]\",\n", w.ty(), i + 1)).collect();
    let head = format!(
        "               ROBUFFER, {kt}, rkeys, \"[runs]\",\n               ROBUFFER, uint32_t, starts, \"[runs + 1]\",\n               ROBUFFER, uint32_t, nruns, \"[1]\",\n               ROBUFFER, uint32_t, perm, \"[rows]\",\n{lrows}{rrows}               ROBUFFER, uint8_t, bloom, \"[screen]\",\n"
    );
    let mut test = String::from("true");
    for i in 1..k {
        test.push_str(&format!(" && (uint64_t)l{i}[t] + (uint64_t)a.b{i} == (uint64_t)r{i}[q] + (uint64_t)a.b{}", k + i));
    }
    format!(
        r#"{COMMON}{reads}
// The run of a left row's key 0 (`*lo`, and whether there is one): a bounded binary search of the runs' keys.
UKL_INLINE bool probe_find(uint64_t key, UKL_GLOBAL const {kt}* rkeys, uint32_t runs, int64_t rbase, UKL_PRIVATE uint32_t* lo)
{{
  uint32_t l = 0u, hi = runs;
  for (uint32_t s = 0u; s < 34u && l < hi; s++) {{
    uint32_t mid = (l + hi) / 2u;
    int64_t v = (int64_t)((uint64_t)rkeys[mid] + (uint64_t)rbase);
    if (v < (int64_t)key) l = mid + 1u; else hi = mid;
  }}
  *lo = l;
  return l < runs && (int64_t)((uint64_t)rkeys[l] + (uint64_t)rbase) == (int64_t)key;
}}

// Each left row's pairs. p0: the left rows; p1: the screen's offset (`NO_SCREEN`: none).
DEFINE_KERNEL2(join, probe_count,
{head}               RWBUFFER, uint32_t, counts, "[a.p0]",
               ARGS, JoinP, a, ,
               GID0, uint32_t, t, )
{{
  if (t >= a.p0) return;
  uint32_t run = 0u, c = 0u;
  uint64_t key = (uint64_t)l0[t] + (uint64_t)a.b0;
  if (join_may_hold(bloom, a.p1, key) && probe_find(key, rkeys, nruns[0], a.b{k}, &run)) {{
    for (uint32_t q = starts[run]; q < starts[run + 1u]; q++) c += (uint32_t)({test});
  }}
  counts[t] = c;
}}

// Each left row's pairs at its offset, those within the space of p1 pairs. p0: the left rows; p2: the screen's offset.
DEFINE_KERNEL2(join, probe_scatter,
{head}               ROBUFFER, uint32_t, offsets, "[a.p0]",
               RWBUFFER, uint32_t, out, "[2 * a.p1]",
               ARGS, JoinP, a, ,
               GID0, uint32_t, t, )
{{
  if (t >= a.p0) return;
  uint32_t run = 0u;
  uint64_t key = (uint64_t)l0[t] + (uint64_t)a.b0;
  if (!join_may_hold(bloom, a.p2, key) || !probe_find(key, rkeys, nruns[0], a.b{k}, &run)) return;
  uint32_t at = offsets[t];
  for (uint32_t q = starts[run]; q < starts[run + 1u]; q++) {{
    if ({test}) {{
      if (at < a.p1) {{ out[2u * at] = t; out[2u * at + 1u] = perm[q]; }}
      at++;
    }}
  }}
}}
"#,
        reads = screen_reads()
    )
}

/// The equi-join of `left`'s columns with `right` (sorted by its key 0), key `i` each one's column `i`. One command
/// buffer counts, scans and scatters into space for `expect` pairs (at least two a left row); when there are more,
/// a second scatters them all into space of the exact size.
pub fn probe(left: &[Column], right: &Keyed, expect: usize) -> Result<Probed, String> {
    // An evaluation of the launch budget (`launch`), or the caller's when it is in one; a scatter again a rerun's.
    let _evaluation = launch::LaunchScope::enter();
    let pass = Pass::new()?;
    let job = ProbeJob::encode(&pass, left, right, expect)?;
    let mut gpu = pass.commit_wait()?;
    let mut submissions = 1;
    if job.overflowed() {
        let _rerun = launch::rerun();
        let pass = Pass::new()?;
        let job = job.rescatter(&pass, left, right)?;
        gpu += pass.commit_wait()?;
        submissions += 1;
        return Ok(Probed { pairs: job.pairs(), gpu_seconds: gpu, submissions });
    }
    if job.empty {
        submissions = 0;
    }
    Ok(Probed { pairs: job.pairs(), gpu_seconds: gpu, submissions })
}

/// A probe encoded into a caller's pass (several in one submission, decision 0100 §5): its counts, scan and
/// pairs in buffers of its own. After the submission completes, its pairs, or (`overflowed`) a second scatter
/// into the exact space.
pub struct ProbeJob {
    nl: usize,
    cap: usize,
    counts: Option<Held>,
    scan: Option<Scan>,
    space: Option<Held>,
    bases: Vec<i64>,
    scatter: Option<Arc<Pipe>>,
    /// No pairs can be: no left or right rows (nothing was encoded).
    pub empty: bool,
}

impl ProbeJob {
    fn inputs<'a>(left: &'a [Column], right: &'a Keyed) -> Vec<&'a Buffer> {
        let mut inputs: Vec<&Buffer> = vec![&right.rkeys.buffer, &right.starts.buffer, &right.nruns.buffer, &right.perm.buffer];
        inputs.extend(left.iter().map(|c| &c.buffer.buffer));
        inputs.extend(right.others.iter().map(|c| &c.buffer.buffer));
        // The screen, or (none: its offset is `NO_SCREEN`) a buffer the kernels do not read.
        inputs.push(right.screen.as_ref().map_or(&right.rkeys.buffer, |f| &f.held.buffer));
        inputs
    }

    /// The screen's descriptor's offset (`NO_SCREEN`: none).
    fn screen(right: &Keyed) -> u32 {
        right.screen.as_ref().map_or(NO_SCREEN, |f| f.at)
    }

    /// The probe of `right` by `left`, encoded after whatever `pass` holds (a sort of `right` in it is read in order).
    pub fn encode(pass: &Pass, left: &[Column], right: &Keyed, expect: usize) -> Result<ProbeJob, String> {
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
        let counts = Held::new(4 * (nl + 1))?;
        let scan = Scan::owned(nl)?;
        let cap = expect.max(2 * nl).min(u32::MAX as usize / 2);
        let space = Held::new(8 * cap)?;
        let inputs = Self::inputs(left, right);
        let mut with = inputs.clone();
        with.push(&counts.buffer);
        let screen = Self::screen(right);
        pass.run(&count, &with, &[nl as u32, screen], &bases, groups(nl));
        scan.encode(pass, &counts.buffer)?;
        let mut with = inputs;
        with.extend([&counts.buffer, &space.buffer]);
        pass.run(&scatter, &with, &[nl as u32, cap as u32, screen], &bases, groups(nl));
        Ok(ProbeJob { nl, cap, counts: Some(counts), scan: Some(scan), space: Some(space), bases, scatter: Some(scatter), empty: false })
    }

    /// The pairs, after the submission completed: the scan's total, after the counts.
    pub fn total(&self) -> usize {
        match (&self.scan, &self.counts) {
            // SAFETY: `nl + 1` counts, the last the scan's total, read after its submission completed.
            (Some(_), Some(counts)) => unsafe { *counts.buffer.contents().cast::<u32>().add(self.nl) as usize },
            _ => 0,
        }
    }

    /// Whether the pairs outgrew their space (after the submission completed).
    pub fn overflowed(&self) -> bool {
        !self.empty && self.total() > self.cap
    }

    /// The scatter again into space of the exact size, encoded into `pass` (the same `left` and `right`).
    pub fn rescatter(mut self, pass: &Pass, left: &[Column], right: &Keyed) -> Result<ProbeJob, String> {
        let total = self.total();
        let out = Held::new(8 * total)?;
        let (Some(counts), Some(scatter)) = (&self.counts, &self.scatter) else {
            return Ok(self);
        };
        let mut with = Self::inputs(left, right);
        with.extend([&counts.buffer, &out.buffer]);
        pass.run(scatter, &with, &[self.nl as u32, total as u32, Self::screen(right)], &self.bases, groups(self.nl));
        self.space = Some(out);
        self.cap = total;
        Ok(self)
    }

    /// The pairs, (left row, right host row), in left row order: after the submission completed.
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
            let buffer = p.device_buffer().clone();
            return Ok(Column { buffer: Held { _lease: Lease::Col(col.clone()), buffer }, width: Width::W64, base: 0, bits: 64, rows });
        }
        let buffer = Held::new((rows * 8).max(8))?;
        // SAFETY: the buffer holds `rows` longs, and nothing else refers to it yet.
        unsafe { std::ptr::copy_nonoverlapping(col.as_ptr(), buffer.buffer.contents().cast::<i64>(), rows) };
        Ok(Column { buffer, width: Width::W64, base: 0, bits: 64, rows })
    }
}


/// Compiles the lane's kernels ahead of their first use (`super::kernels_ahead`: the UKL library, the joins' small
/// path, the scans', the radix sort's of each width with the gathers, the tile pairs' sort, and the relational
/// kernels), as a job on the pool, one at a time. A job that needs one still being compiled waits for it rather than
/// compiling it again. A kernel that fails is left to its use, which fails saying why (each entry catches its panic:
/// a job's panic would end the process).
pub fn prewarm() {
    crate::exec::configure();
    // In its caller's run (decision 0442): the pipelines are the caller's machine's, and the job's wall its run's
    // (`as_job` reads it), so the run is entered first. Each entry carries the run too (`super::ahead`).
    let run = rerail_context::job_run();
    let list = super::kernels_ahead();
    rerail_context::cores_spawn(move || {
        let _run = rerail_context::enter(run);
        crate::exec::as_job("the GPU's prewarm", || _ = rerail_gpu::kernels::compile_ahead(list, 1, Default::default()));
    });
}

/// A batch's pairs: one buffer the probes reserve their threadgroups' pairs in with one atomic each, so no probe
/// needs a second submission to learn its size. `cap` is what the host expects in all; a probe whose pairs do
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

    /// Whether any probe's pairs did not fit (after the submission completed).
    pub fn overflowed(&self) -> bool {
        // SAFETY: one word, zeroed by the host and written by the device, its submission completed.
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
    /// Whether this probe's pairs did not all fit, and how many it found (after the submission completed).
    pub fn spilled(&self, arena: &Arena) -> (bool, usize) {
        let b = self.read_blocks(arena);
        (b.iter().any(|&(at, n)| at == u32::MAX && n > 0), b.iter().map(|&(_, n)| n as usize).sum())
    }

    fn read_blocks(&self, arena: &Arena) -> Vec<(u32, u32)> {
        // SAFETY: this probe's blocks, written by the device (or the host for an empty probe), the submission
        // completed.
        let b = unsafe { std::slice::from_raw_parts(arena.blocks.buffer.contents().cast::<u32>(), 2 * (self.first + self.blocks)) };
        (self.first..self.first + self.blocks).map(|i| (b[2 * i], b[2 * i + 1])).collect()
    }

    /// The pairs read in place, in left row order (block by block), after the submission completed: a view of
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
        // completed submission and never written again: the arena is this batch's alone.
        Some(unsafe { crate::kernel::PairView::new(keep, arena.pairs.buffer.contents().cast::<[u32; 2]>(), segs) })
    }

    /// The pairs, in left row order (block by block), after the submission completed. `None` when they did not
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
// Each job's buffers are reached through device addresses in a table the host writes (`Buffer::device_address`),
// and made resident with the launches: a batch of any number of joins costs the host two launches. A device without
// addresses for its allocations (OpenCL without Intel's unified shared memory) takes the radix sort and the probe
// join instead (`Fused::can`).

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

/// `join.uglykl`'s `ProbeJob`, byte for byte (its `UKL_STATIC_ASSERT`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProbeDesc {
    rkeys: u64,
    starts: u64,
    nruns: u64,
    perm: u64,
    l: [u64; 3],
    r: [u64; 2],
    screen: u64,
    lbase: [i64; 3],
    kbase: i64,
    rbase: [i64; 2],
    nl: u32,
    k: u32,
    kw: u32,
    screen_at: u32,
    rw: [u32; 2],
    pad2: [u32; 2],
}

const _: () = assert!(std::mem::size_of::<ProbeDesc>() == 160);


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
    /// Whether the process's device can run the fused sorts and probes: their tables hold the addresses of leased
    /// buffers, which the device allocated (`Device::allocations_have_addresses`).
    pub fn can() -> bool {
        device().is_ok_and(|d| d.allocations_have_addresses())
    }

    fn addr(&mut self, b: &Buffer) -> u64 {
        self.used.push(b.clone());
        b.device_address().expect("a fused join on a device with addresses (`Fused::can`)")
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
            screen: None,
            pending: std::sync::Mutex::new(None),
            failed: std::sync::atomic::AtomicBool::new(false),
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
            // SAFETY: the blocks' words for this probe, the host's alone before the submission is committed.
            let b = unsafe { std::slice::from_raw_parts_mut(arena.blocks.buffer.contents().cast::<u32>(), 2 * arena.used) };
            b[2 * first..2 * (first + blocks)].fill(0);
            return Ok(ProbeOne { nl, first, blocks });
        }
        let mut d = ProbeDesc { nl: nl as u32, k: k as u32, kw: right.key.bytes() as u32, kbase: right.base, ..Default::default() };
        d.rkeys = self.addr(&right.rkeys.buffer);
        d.starts = self.addr(&right.starts.buffer);
        d.nruns = self.addr(&right.nruns.buffer);
        d.perm = self.addr(&right.perm.buffer);
        d.screen_at = NO_SCREEN;
        if let Some(f) = &right.screen {
            d.screen = self.addr(&f.held.buffer);
            d.screen_at = f.at;
        }
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
    /// keeps the tables alive in `keep` until the submission completes.
    pub fn dispatch(self, pass: &Pass, arena: &Arena, keep: &mut Vec<Held>) -> Result<(), String> {
        let table = |bytes: &[u8]| -> Result<Held, String> {
            let h = Held::new(bytes.len().max(16))?;
            // SAFETY: the buffer holds at least `bytes`, and nothing else refers to it yet.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), h.buffer.contents().cast::<u8>(), bytes.len()) };
            Ok(h)
        };
        // SAFETY: plain `repr(C)` values, read as their bytes.
        let raw = |p: *const u8, n: usize| unsafe { std::slice::from_raw_parts(p, n) };
        for b in &self.used {
            pass.use_read_write(b);
        }
        if !self.sorts.is_empty() {
            let pipe = fixed("sort_all")?;
            let t = table(raw(self.sorts.as_ptr().cast(), std::mem::size_of_val(&self.sorts[..])))?;
            pass.run(&pipe, &[&t.buffer], &[], &[], self.sorts.len());
            keep.push(t);
        }
        if !self.table.is_empty() {
            let pipe = fixed("probe_all")?;
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
        // SAFETY: the buffer holds at least `n` words, written by a completed submission.
        unsafe { std::slice::from_raw_parts(b.buffer.contents().cast::<u32>(), n) }.to_vec()
    }

    /// A pass's launches are the evaluation's (`launch`): charged as they are made, so passes share one budget and
    /// none has a count of its own. A launch past it, or outside an evaluation, is the hard failure (decision 0601):
    /// it raises `exec::OverBudget` before it is made.
    #[test]
    fn a_pass_charges_the_evaluations_launch_budget() {
        if device().is_err() {
            return;
        }
        let over = |f: &dyn Fn()| match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
            Ok(()) => false,
            Err(p) => p.downcast::<crate::exec::OverBudget>().is_ok(),
        };
        let keys = fixed("pair_keys").unwrap();
        let (words, out) = (Held::zeroed(8 * 256).unwrap(), Held::zeroed(8 * 256).unwrap());
        let launches = |pass: &Pass, n: usize| {
            for _ in 0..n {
                pass.run(&keys, &[&words.buffer, &out.buffer], &[256], &[], 1);
            }
        };
        let outside = Pass::new().unwrap();
        assert!(over(&|| launches(&outside, 1)), "a launch outside an evaluation is the hard failure");
        assert_eq!(outside.dispatches(), 0, "and is not made");
        drop(outside);
        let evaluation = launch::LaunchScope::enter();
        let first = Pass::new().unwrap();
        launches(&first, launch::KERNEL_LAUNCH_LIMIT - 10);
        first.commit_wait().unwrap();
        let second = Pass::new().unwrap();
        assert!(over(&|| launches(&second, 11)), "the launch past the budget is the hard failure");
        assert_eq!(second.dispatches(), 10, "the launch past the budget is not made");
        drop(second);
        let records = evaluation.snapshot();
        assert_eq!(records.len(), launch::KERNEL_LAUNCH_LIMIT);
        assert!(records.iter().filter(|r| r.command == records[0].command).all(|r| r.completed), "the first pass's launches completed");
    }

    /// Pairs sorted on the device come out in (first, second) order: keys of 32 bits and of 64.
    #[test]
    fn pairs_sorted_on_the_device_are_in_row_order() {
        if device().is_err() {
            return;
        }
        for (n, fb, sb) in [(5000usize, 12u32, 13u32), (70_000, 20, 30), (1, 0, 0), (3000, 32, 32)] {
            let firsts = rows(n, 7 + n as u64, 1i64 << fb.min(31), 0);
            let seconds = rows(n, 11 + n as u64, 1i64 << sb.min(31), 0);
            let want_in: Vec<[u32; 2]> = firsts.iter().zip(&seconds).map(|(&a, &b)| [a as u32, b as u32]).collect();
            let pairs = Held::of(want_in.iter().copied()).unwrap();
            let out = Held::new(8 * n).unwrap();
            let _evaluation = launch::LaunchScope::enter();
            let pass = Pass::new().unwrap();
            let mut keep = Vec::new();
            encode_sorted_pairs(&pass, &pairs, n, fb, sb, &out, &mut keep).unwrap();
            pass.commit_wait().unwrap();
            let got: Vec<(u32, u32)> = read(&out, 2 * n).as_chunks::<2>().0.iter().map(|p| (p[0], p[1])).collect();
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

    /// Decision 0488, one format: the device's screen of a sort's keys is, word for word, the filter the host makes of
    /// them in a block (`rerail_blocks::bloom`), and a sort screened by that host filter, bound where it lies, gives a
    /// probe the host join's pairs, alone and in the fused dispatch (which tests a sort's screen too), as does the
    /// sort's own screen.
    #[test]
    fn the_devices_screen_is_the_hosts_filter() {
        let Ok(dev) = device() else { return };
        use rerail_blocks::bloom::{Bloom, hash};
        let right: Vec<i64> = (0..20_000i64).map(|i| i * 6 - 50_000).collect();
        let left: Vec<i64> = (0..60_000i64).map(|i| i * 3 - 80_000).collect();
        // copy: not block data (a test's literal keys)
        let own = Keyed::sort(vec![Column::upload(&right).unwrap()]).unwrap();
        own.settle();
        let words = screen_words(right.len()).unwrap();
        let block = std::sync::Arc::new(rerail_blocks::Block::anon(rerail_blocks::block::kind::FACTS, 1 << 20).with_free_list());
        let at = Bloom::try_create(&block, words).unwrap();
        // SAFETY: the descriptor just made in `block`.
        let host = unsafe { &*block.ptr::<Bloom>(at) };
        for &k in &right {
            host.add_hashed(hash(k));
        }
        let f = own.screen.as_ref().expect("a screened sort");
        // SAFETY: the screen's words, 64 bytes on, written by its sort's submission, which completed.
        let on_device = unsafe { std::slice::from_raw_parts(f.held.buffer.contents().cast::<u8>().add(64).cast::<u64>(), words) };
        assert!((0..words).all(|i| on_device[i] == host.word(i)), "the device's screen is the host's filter");
        // copy: not block data (a test's literal keys)
        let mut kept = Keyed::sort(vec![Column::upload(&right).unwrap()]).unwrap();
        kept.screen_by(block.device_buffer(dev).unwrap(), at, Box::new(block.clone()));
        let mut want = crate::rows::join(&[&left], &[&right]);
        want.sort_unstable();
        assert!(!want.is_empty());
        let raw = crate::col::Col::from(left.clone());
        for right in [&own, &kept] {
            // copy: not block data (a test's literal keys)
            let mut got = probe(&[Column::upload(&left).unwrap()], right, 0).unwrap().pairs;
            got.sort_unstable();
            assert_eq!(got, want, "a probe");
            if !Fused::can() {
                continue;
            }
            let _evaluation = launch::LaunchScope::enter();
            let pass = Pass::new().unwrap();
            let mut fused = Fused::default();
            let mut arena = Arena::new(2 * left.len(), probe_blocks(left.len())).unwrap();
            // The probed column is held until the dispatch binds it (decision 0491 step 2: the tables hold its
            // address alone, so a column dropped here would give its lease back while the kernel reads it).
            let probed = [Column::raw(&raw).unwrap()];
            let one = fused.probe(&probed, right, &mut arena).unwrap();
            let mut keep = Vec::new();
            fused.dispatch(&pass, &arena, &mut keep).unwrap();
            pass.commit_wait().unwrap();
            let mut got = one.pairs(&arena).expect("the pairs fit");
            got.sort_unstable();
            assert_eq!(got, want, "a fused probe");
        }
    }

    /// Decision 0488: a sort of `SCREENED_ROWS` rows or more is screened by its runs' filter, made on the device from
    /// its runs. A probe whose keys mostly miss (60,000 left keys, 1% of them among 8,192 or 65,536 right ones, the
    /// others between them; 16 and 64-bit sorts) finds the host join's pairs with the filter and without it. The
    /// filter at its most words (`screen_words`' clamp, 4,000,000 rows) is the same kernel at another size: its sizes
    /// are held below, not sorted here (a sort of millions on the CPU backend took 155 s, a test's seconds many times).
    #[test]
    fn a_screened_probe_finds_the_hosts_pairs() {
        assert_eq!(screen_words(4_000_000), Some(1 << 19));
        assert_eq!(screen_words(65_536), Some(8192));
        assert_eq!(screen_words(1 << 23 | 1), None);
        screened_probes(0);
    }

    /// The probe's GPU time with the filter and without it, interleaved, the median of 20 each (a probe's GPU time
    /// moves by a fifth between runs at a millisecond). A measurement: run it alone with `--ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn a_screened_probes_gpu_time() {
        screened_probes(20);
    }

    /// The probes of [`a_screened_probe_finds_the_hosts_pairs`], each shape's checked against the host join, then
    /// `timed` more of each, interleaved, their median GPU time printed.
    fn screened_probes(timed: usize) {
        if device().is_err() {
            return;
        }
        for (nr, modulus, high) in [(8192, 60_000, 0), (8192, 1 << 50, -(1 << 55)), (65_536, 1 << 50, -(1 << 55))] {
            // The right's keys even, the left's misses odd and among them: a miss's search ends anywhere in the runs.
            let right: Vec<i64> = rows(nr, 21, modulus, high).into_iter().map(|v| v & !1).collect();
            let mut left: Vec<i64> = rows(60_000, 22, modulus, high).into_iter().map(|v| v | 1).collect();
            for (i, l) in left.iter_mut().enumerate().filter(|(i, _)| i % 100 == 0) {
                *l = right[i % right.len()];
            }
            let screened = Keyed::sort(vec![Column::upload(&right).unwrap()]).unwrap();
            let mut plain = Keyed::sort(vec![Column::upload(&right).unwrap()]).unwrap();
            plain.screen = None;
            assert!(screened.screen.is_some());
            let column = || vec![Column::upload(&left).unwrap()];
            let (with, without) = (probe(&column(), &screened, 0).unwrap(), probe(&column(), &plain, 0).unwrap());
            let mut want = crate::rows::join(&[&left], &[&right]);
            want.sort_unstable();
            let (mut a, mut b) = (with.pairs, without.pairs);
            a.sort_unstable();
            b.sort_unstable();
            assert!(want.len() >= 600, "{} pairs", want.len());
            assert_eq!(a, want);
            assert_eq!(b, want);
            if timed == 0 {
                continue;
            }
            let (mut gpu_with, mut gpu_without) = (Vec::new(), Vec::new());
            for _ in 0..timed {
                gpu_with.push(probe(&column(), &screened, 0).unwrap().gpu_seconds);
                gpu_without.push(probe(&column(), &plain, 0).unwrap().gpu_seconds);
            }
            let median = |v: &mut Vec<f64>| { v.sort_by(f64::total_cmp); v[v.len() / 2] * 1e6 };
            println!("{nr} right rows, modulus {modulus}: {} pairs; probe GPU (median of {timed}) with the filter {:.0} us, without {:.0} us", want.len(), median(&mut gpu_with), median(&mut gpu_without));
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

    /// The process's CPU, user and system: the device layer's own threads' too.
    fn cpu() -> f64 {
        rerail_device::os::process_cpu()
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
        let Ok(device) = device() else { return };
        let alloc = per(500, || drop(rerail_device::device::alloc_uninitialized_array_sync::<u8>(device, 16384)));
        let pooled = per(500, || drop(Held::new(16384).unwrap()));
        let empty = per(300, || {
            let _evaluation = launch::LaunchScope::enter();
            Pass::new().unwrap().commit_wait().unwrap();
        });
        let keys = fixed("pair_keys").unwrap();
        let (words, out) = (Held::zeroed(8 * 256).unwrap(), Held::zeroed(8 * 256).unwrap());
        let dispatch = per(100, || {
            // Each submission an evaluation: its 50 launches within the budget.
            let _evaluation = launch::LaunchScope::enter();
            let pass = Pass::new().unwrap();
            for _ in 0..50 {
                pass.run(&keys, &[&words.buffer, &out.buffer], &[256], &[], 1);
            }
            pass.commit_wait().unwrap();
        });
        let dispatch = (dispatch - empty) / 50.0;
        println!("host per buffer allocated {alloc:.1} us, pooled {pooled:.2} us; per empty submission {empty:.1} us; per launch {dispatch:.2} us");
        for rows in [1000usize, 20_000, 200_000] {
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
                "{rows} rows ({pairs} pairs): upload {upload:.0} us; device sort host {sort:.0} us, device {:.0} us; probe host {probe_cpu:.0} us, device {:.0} us; host radix sort {host:.0} us, host sort join (left sorted, runs merged) {host_join:.0} us",
                sort_gpu / runs as f64 * 1e6,
                probe_gpu / runs as f64 * 1e6
            );
        }
    }
}
