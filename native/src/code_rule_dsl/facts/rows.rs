//! A relation's rows as segments (smoking gun 8, t34; setfix's relations' and the vtab's `Relation`'s), each a `rerail_blocks` relation in a block the CPU and the
//! GPU read in place (decision 0325).
//!
//! A maintained stratum's relation starts as its kept one less the rows gone. The kept relation's segments are
//! shared (`Arc`), its dead rows marked beside them, and the rows a run pushes go in a tail of its own (ids from the
//! kept relation's count on). When the run's relation is closed (`InRel`), its tail is frozen into one more segment,
//! not copied, so the next run shares it in turn.
//!
//! ```text
//! kept run's rows:   [ segment 0: ids 0..9,000 ]
//! this run's start:  [ segment 0 (shared) ] + tail (ids 9,000..)        // nothing copied
//! closed:            [ segment 0 ] [ segment 1: the tail, moved ]
//! ```
//!
//! A segment and the tail are relations of `rerail_blocks` (columns in doubling chunks), read through the reads
//! written once (`rerail_clif::relation`: the host's written directly in `rerail_blocks`, the kernels' the same
//! reads in UKL). They live in shared FACTS blocks (`Facts`): a block holds many relations, in host memory the device
//! reads through a no-copy wrap of its used extent (decision 0435 §1). A block never moves, and reserves address space
//! enough that a tail grows in it in place (decision 0445): its pages are taken as rows are written, and a dead
//! relation's go back to the OS. Where a reservation is memory (WASI), a block is sized, and a tail whose block is full
//! moves to another, one copy of its rows (as a vector's growth copies them; `moved` counts them). Every row
//! lies whole in one segment. Segments past `MAX_SEGMENTS` are merged from the newest (the small ones a session's
//! edits add), so a row's segment is found among a few.

use std::sync::{Arc, Mutex};

use rerail_blocks::block::{Block, kind};
use rerail_blocks::relation::{Relation, Spec};
use rerail_blocks::rows::ColRead;

/// The most segments a frozen relation holds; past them its newest are merged.
const MAX_SEGMENTS: usize = 8;

/// Whether a reservation is address space only (a 64-bit Unix or Windows: `rerail_blocks::mem::reserve`), its pages
/// taken (on Windows, committed) as they are written; elsewhere (WASI) it is a zeroed allocation of its whole length.
const RESERVES_ADDRESS_SPACE: bool = cfg!(all(any(unix, windows), target_pointer_width = "64"));

/// What a block of relations reserves where a reservation is address space (decision 0445, t49 item 2): enough that
/// no relation's tail outgrows its block within a run's memory. A Mac gave 4,000 such reservations at once.
const BLOCK_RESERVE: usize = 16 << 30;

impl Life {
    /// What its blocks reserve (a relation larger takes one its size). Where a reservation is memory, a batch's
    /// sources' and kept rows' blocks are smaller, as they die with it.
    fn block_bytes(self) -> usize {
        match self {
            // Each alone in a block of its size (`try_new`).
            Life::Device => 0,
            _ if RESERVES_ADDRESS_SPACE => BLOCK_RESERVE,
            Life::Run => 64 << 20,
            Life::Source | Life::Kept => 16 << 20,
        }
    }
}

/// Relations moved out of a full block (`Part::moved_whole`), and the bytes of their rows copied: none where a block
/// reserves address space (decision 0445), read by the profile. On the shared object (decision 0442), as `Merged` is.
#[derive(Default)]
struct Moved([std::sync::atomic::AtomicU64; 2]);

fn count_move(rows: u32, k: usize) {
    let m = crate::code_rule_dsl::logic::profile::shared::<Moved>();
    m.0[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    m.0[1].fetch_add(u64::from(rows) * k as u64 * 8, std::sync::atomic::Ordering::Relaxed);
}

/// The relations moved out of a full block so far, and their rows' bytes copied (`count_move`), for a profile.
pub fn moved() -> (u64, u64) {
    let m = crate::code_rule_dsl::logic::profile::shared::<Moved>();
    (m.0[0].load(std::sync::atomic::Ordering::Relaxed), m.0[1].load(std::sync::atomic::Ordering::Relaxed))
}

/// Room a block keeps past a new relation's: what its other relations' tails may take meanwhile.
const MARGIN: usize = 1 << 20;

/// A block of relations: shared by every segment and tail in it, given back (to the pool) with the last.
pub struct Facts {
    /// Held by handle (decision 0491 step 3): a region of it keeps it as its relations do.
    pub block: Arc<Block>,
    /// Held while an index is added to one of its relations (`Relation::try_reserve_index`, decision 0325 step 5).
    pub ix: Mutex<()>,
    /// The count of bytes given back of the run that leased it (`given_back`): a relation's last holder may drop it
    /// on any thread.
    given: Arc<GivenBack>,
    /// The chunk's tally it adds its bytes written to when given back (`in_chunk_measured`), when a chunk made it.
    tally: Option<Arc<std::sync::atomic::AtomicUsize>>,
    /// How long its relations live.
    life: Life,
}

impl Facts {
    /// A block of relations of `life`: the chunk's the run is evaluating, if one is, counted in its tally.
    fn new(block: Block, life: Life) -> Facts {
        let tally = current_chunk().map(|c| Arc::clone(&c.tally));
        Facts { block: Arc::new(block), ix: Mutex::new(()), given: crate::code_rule_dsl::logic::profile::local::<GivenBack>(), tally, life }
    }
}

impl Drop for Facts {
    fn drop(&mut self) {
        if let Some(t) = &self.tally {
            t.fetch_add(self.block.used(), std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// What a chunk's evaluation wrote in blocks of relations (decision 0435 §4, Q314): every block it made, at its
/// most (a block's used bytes never fall), and of the blocks still held when it ended (rows it handed on), the bytes
/// past their free lists (decision 0445). A
/// function of the chunk and its plans, not of the threads' timing, as the process's count is not.
///
/// `heap` is its scopes' heap at their most (decision 0469): each scope of the chunk on a thread (its evaluation, a
/// job carried into it) from the allocator's per-thread peak, reset at the scope's entry, so a scope's is its own
/// allocation, whatever ran on other threads meanwhile. Their sum bounds what they held at once, and a scope's peak
/// is a function of its work, not of the threads' timing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Written {
    pub bytes: usize,
    pub held: usize,
    pub heap: usize,
}

impl Written {
    /// What sizes the chunks after it (decision 0469): its blocks' bytes and its scopes' heap at their most.
    pub fn measure(&self) -> usize {
        self.bytes + self.heap
    }
}

/// How long a relation's rows live, which picks the blocks they go in: a block is given back with its last part,
/// so rows that die together share blocks, and rows that outlive them do not keep their blocks.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Life {
    /// A run's relations (setfix's), some of them kept for a session's later runs.
    #[default]
    Run,
    /// A batch's sources (the vtab's `Relation`s of its syntax facts), dropped with the batch. In blocks with the
    /// runs' relations, they kept those blocks alive past them: on posthog/hogql every block leased stayed, and the
    /// peak resident size rose 125 MB.
    Source,
    /// The relations a run keeps past it (a chunk's exports, read by the cross-file part where they lie): apart from
    /// the run's other relations, so holding them does not keep those relations' blocks.
    Kept,
    /// A device run's own relations (decision 0325 step 5): the views it makes from a chunk, a fixpoint's totals and
    /// deltas, an upload. Leased as the device's join buffers are, within its budget (a refusal runs the stratum on
    /// the CPU), and apart from the runs' relations, which they would keep alive.
    Device,
}

/// The blocks a run's new relations go in outside a chunk, while they have room: one a `Life` (decision 0442: the
/// run's, so no two runs' relations share a block).
#[derive(Default)]
struct Open([Mutex<Option<Arc<Facts>>>; 4]);

/// The open blocks new relations go in outside a chunk (`Open`): the run's, or outside a run the machine's (decision
/// 0442: every piece of state has an owner). A session's work between its runs shares the machine's, as a run's
/// relations share the run's: a relation's tables grow in its block (0384), and a relation dropped gives its chunks
/// back to it (0342). Outside a run `local` is a value of its own each time it is asked for: every relation then
/// leased a block of a run's size, 16 GiB of address space where blocks reserve it (decision 0445), and a session's
/// 1,000 one-document batches' relations reserved all of it (mmap refused at 256 TiB).
fn open_blocks() -> Arc<Open> {
    if crate::code_rule_dsl::logic::profile::run_own().is_some() {
        crate::code_rule_dsl::logic::profile::local::<Open>()
    } else {
        crate::code_rule_dsl::logic::profile::machine::<Open>()
    }
}

/// Every block leased on this machine, by `Life`, while it lives: what a profile reads (`held`; decision 0442: the
/// machine's, as the memory it counts is).
#[derive(Default)]
struct Live(Mutex<[Vec<std::sync::Weak<Facts>>; 3]>);

/// Per `Life`, the blocks held now, their bytes written and their bytes reserved: a profile's account of the rows
/// outside the heap.
pub fn held() -> [(usize, usize, usize); 3] {
    let live_blocks = crate::code_rule_dsl::logic::profile::machine::<Live>();
    let live = live_blocks.0.lock().unwrap_or_else(|e| e.into_inner());
    std::array::from_fn(|l| {
        live[l].iter().filter_map(std::sync::Weak::upgrade).fold((0, 0, 0), |(n, u, r), f| (n + 1, u + f.block.used(), r + f.block.reserved()))
    })
}

/// A chunk's blocks (Q58): the ones its new relations go in while they have room (one a `Life`), and every one it
/// made, to count those that outlive it. A block lives while any relation in it does, so one shared by every chunk
/// at once was kept whole by a few rows of each, and the run's footprint (the line its chunk sizes are cut by)
/// counted it whole.
///
/// It also keeps what its work holds (Q203 item 3), the measure its chunks are sized by: the bytes of its blocks that
/// live, plus what its jobs allocated and did not free (`enter_chunk`), at their most so far (`held_most`). The
/// process's footprint is not a chunk's: the run's one-time allocations and the other chunks' in flight are in it,
/// and pages freed before a chunk hid its growth; the same 29,000-node chunks so measured 1 to 1,471 MB.
#[derive(Default)]
pub struct Chunk {
    open: [Mutex<Option<Arc<Facts>>>; 4],
    made: Mutex<Vec<std::sync::Weak<Facts>>>,
    /// The bytes written in its blocks given back (`Facts`'s drop), and in those held when it ended (`Written`).
    tally: Arc<std::sync::atomic::AtomicUsize>,
    /// Of its blocks held when it ended, their used bytes, and those past their free lists (`Written::held`).
    live: std::sync::atomic::AtomicUsize,
    held: std::sync::atomic::AtomicUsize,
    /// What its jobs allocated less what they freed, so far (bytes; another chunk's frees can make it negative).
    heap: std::sync::atomic::AtomicI64,
    /// The most its live blocks and its jobs' heap held at once, seen at a block made or a job's end.
    most: std::sync::atomic::AtomicUsize,
    /// Its scopes' heap at their most, summed (`Written::heap`): each scope's own (`enter_chunk`), whatever ran
    /// beside it.
    peaks: std::sync::atomic::AtomicUsize,
}

impl Chunk {
    /// Notes what it holds now (`most`).
    fn note(&self) {
        let blocks: usize = self.made.lock().unwrap_or_else(|e| e.into_inner()).iter().filter_map(std::sync::Weak::upgrade).map(|f| f.block.used()).sum();
        let heap = self.heap.load(std::sync::atomic::Ordering::Relaxed).max(0) as usize;
        self.most.fetch_max(blocks + heap, std::sync::atomic::Ordering::Relaxed);
    }

    /// The most its work held at once so far (bytes): its live blocks and its jobs' heap.
    pub fn held_most(&self) -> usize {
        self.note();
        self.most.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// This thread's bytes allocated less those freed, from the allocator's own counters (installed by the engine, which
/// links the allocator: `install_heap_net`); 0 without one.
static HEAP_NET: std::sync::OnceLock<fn() -> i64> = std::sync::OnceLock::new();

/// Installs the allocator's per-thread counter of bytes allocated less freed; the first one stays.
pub fn install_heap_net(net: fn() -> i64) {
    let _ = HEAP_NET.set(net);
}

fn heap_net() -> i64 {
    HEAP_NET.get().map_or(0, |f| f())
}

/// This thread's most allocated less freed since its last reset, above what it was at the reset, and the reset, from
/// the allocator's own counters (jemalloc's `thread.peak`; installed by the engine: `install_heap_peak`); 0 without.
static HEAP_PEAK: std::sync::OnceLock<(fn() -> i64, fn())> = std::sync::OnceLock::new();

/// Installs the allocator's per-thread peak (`read`, since the last `reset`) and its reset; the first one stays.
pub fn install_heap_peak(read: fn() -> i64, reset: fn()) {
    let _ = HEAP_PEAK.set((read, reset));
}

fn peak_reset() {
    if let Some((_, reset)) = HEAP_PEAK.get() {
        reset();
    }
}

/// The innermost scope's own allocation at its most so far: its most before the last reset, or what it held at the
/// reset and the most since.
fn scope_peak() -> i64 {
    let (most, at) = PEAK.with(std::cell::Cell::get);
    most.max(at + HEAP_PEAK.get().map_or(0, |(read, _)| read()))
}

thread_local! {
    /// The bytes this thread's chunk scopes (`enter_chunk`) gave their chunks so far: a scope's own are its thread's
    /// net less what the scopes inside it gave theirs (a job of another chunk run while it waited).
    static GIVEN: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
    /// The innermost scope's thread net and `GIVEN` at its entry: its allocation so far, for `chunk_held_most`.
    static TOP: std::cell::Cell<(i64, i64)> = const { std::cell::Cell::new((0, 0)) };
    /// The innermost scope's own allocation at its most before the allocator's peak was last reset, and its own
    /// allocation at that reset (`scope_peak`).
    static PEAK: std::cell::Cell<(i64, i64)> = const { std::cell::Cell::new((0, 0)) };
}

/// A chunk scope entered on this thread (`enter_chunk`): when it drops, what this thread allocated less freed meanwhile
/// (less the scopes inside) is the chunk's. The chunk itself is the run's context's (`InChunk`), which a job carries.
pub struct ChunkScope {
    chunk: Option<Arc<Chunk>>,
    net: i64,
    given: i64,
    top: (i64, i64),
    /// The scope it is inside: its own allocation at its most so far (`scope_peak`), held while this one runs.
    outer_peak: i64,
}

/// Enters a scope of `chunk` on this thread, for a job carried into it (`setfix::carried`) or its own evaluation
/// (`in_chunk`).
pub fn enter_chunk(chunk: Option<Arc<Chunk>>) -> ChunkScope {
    // The scope it is inside stops at its most so far; this one's peak counts from here (decision 0469).
    let outer_peak = scope_peak();
    PEAK.with(|p| p.set((0, 0)));
    peak_reset();
    let (net, given) = (heap_net(), GIVEN.with(std::cell::Cell::get));
    ChunkScope { chunk, net, given, top: TOP.with(|t| t.replace((net, given))), outer_peak }
}

impl Drop for ChunkScope {
    fn drop(&mut self) {
        let own = (heap_net() - self.net) - (GIVEN.with(std::cell::Cell::get) - self.given);
        let most = scope_peak().max(0) as usize;
        GIVEN.with(|g| g.set(g.get() + own));
        TOP.with(|t| t.set(self.top));
        // The scope it was inside goes on from its own allocation now, its most so far kept.
        let (net, given) = self.top;
        let outer_now = (heap_net() - net) - (GIVEN.with(std::cell::Cell::get) - given);
        PEAK.with(|p| p.set((self.outer_peak, outer_now)));
        peak_reset();
        if let Some(c) = &self.chunk {
            c.heap.fetch_add(own, std::sync::atomic::Ordering::Relaxed);
            c.peaks.fetch_add(most, std::sync::atomic::Ordering::Relaxed);
            c.note();
        }
    }
}

/// The chunk the run is evaluating (`InChunk`), for a job to enter a scope of (`setfix::carried`).
pub fn current_chunk() -> Option<Arc<Chunk>> {
    crate::code_rule_dsl::logic::profile::bound::<InChunk>().0
}

/// What the chunk this thread evaluates has held at its most so far (`Chunk::held_most`); `None` outside one.
pub fn chunk_held_most() -> Option<usize> {
    let chunk = current_chunk()?;
    // The scope this thread is in, its allocation so far: its chunk's, not given yet.
    let (net, given) = TOP.with(std::cell::Cell::get);
    let open = (heap_net() - net) - (GIVEN.with(std::cell::Cell::get) - given);
    chunk.heap.fetch_add(open, std::sync::atomic::Ordering::Relaxed);
    let most = chunk.held_most();
    chunk.heap.fetch_sub(open, std::sync::atomic::Ordering::Relaxed);
    Some(most)
}

/// The chunk the run is evaluating (`in_chunk`), in the run's context: its jobs' relations go in its blocks too.
#[derive(Clone, Default)]
struct InChunk(Option<Arc<Chunk>>);

/// Runs `f` as a chunk's evaluation: the relations it and its jobs make go in blocks of the chunk's own, given back
/// with the last of its rows (decision 0289: a chunk is a block). What outlives the chunk keeps only its own block;
/// the profile counts those (`a chunk's blocks outlived it`).
pub fn in_chunk<R>(f: impl FnOnce() -> R) -> R {
    in_chunk_measured(f).0
}

/// Of a chunk's blocks alive at its end (life, used, used past the free list), the bytes it holds: rows it handed on,
/// a device run's own left out (`in_chunk_measured`).
fn held_of(live: &[(Life, usize, usize)]) -> usize {
    live.iter().filter(|l| l.0 != Life::Device).map(|l| l.2).sum()
}

/// `in_chunk`, and what the chunk's blocks wrote and its scopes' heap at their most (`Written`).
pub fn in_chunk_measured<R>(f: impl FnOnce() -> R) -> (R, Written) {
    struct Leave(Option<(ChunkScope, crate::code_rule_dsl::logic::profile::Bound)>, Arc<Chunk>);
    impl Drop for Leave {
        fn drop(&mut self) {
            drop(self.0.take());
            for open in &self.1.open {
                open.lock().unwrap_or_else(|e| e.into_inner()).take();
            }
            let made = std::mem::take(&mut *self.1.made.lock().unwrap_or_else(|e| e.into_inner()));
            // Of a block still held, its used bytes are what the chunk wrote in it, and those past its free list what
            // it holds (decision 0445: a block one kept relation holds lives, its dead relations' pages given back).
            // A device run's own relations (views, totals, deltas) are handed on to nothing: a block of theirs outlives
            // the chunk only while device work still in flight holds it, so it is the device's timing, not rows held
            // (Q314: on Metal 15-30 of a chunk's blocks outlived it, a different number every run).
            let (n, live) = (made.len(), made.iter().filter_map(std::sync::Weak::upgrade).map(|f| (f.life, f.block.used(), f.block.used().saturating_sub(f.block.free_bytes()))).collect::<Vec<_>>());
            self.1.live.store(live.iter().map(|l| l.1).sum(), std::sync::atomic::Ordering::Relaxed);
            self.1.held.store(held_of(&live), std::sync::atomic::Ordering::Relaxed);
            if !live.is_empty() {
                let of = |device: bool| {
                    let v: Vec<_> = live.iter().filter(|l| (l.0 == Life::Device) == device).collect();
                    (v.len(), v.iter().map(|l| l.2).sum::<usize>() >> 20)
                };
                let ((held, mb), (device, device_mb)) = (of(false), of(true));
                let by = |life: Life| {
                    let v: Vec<_> = live.iter().filter(|l| l.0 == life).collect();
                    format!("{} {} KB", v.len(), v.iter().map(|l| l.2).sum::<usize>() >> 10)
                };
                crate::code_rule_dsl::logic::profile::decision("a chunk's blocks outlived it", || {
                    format!("{held} of its {n} blocks, {mb} MB used in them, held by rows it handed on (a run's {}, sources' {}, kept rows' {}); {device} of a device run's own, {device_mb} MB, until its work ends", by(Life::Run), by(Life::Source), by(Life::Kept))
                });
            }
        }
    }
    let chunk = Arc::new(Chunk::default());
    // The chunk is the run's context's (its jobs' too); its scope here counts this thread's allocation into it.
    let bound = crate::code_rule_dsl::logic::profile::bind(InChunk(Some(chunk.clone())));
    let leave = Leave(Some((enter_chunk(Some(chunk.clone())), bound)), chunk.clone());
    let r = f();
    // Its open blocks are let go here: a block none of its relations holds is given back, and tallied.
    drop(leave);
    let held = chunk.held.load(std::sync::atomic::Ordering::Relaxed);
    let heap = chunk.peaks.load(std::sync::atomic::Ordering::Relaxed);
    (r, Written { bytes: chunk.tally.load(std::sync::atomic::Ordering::Relaxed) + chunk.live.load(std::sync::atomic::Ordering::Relaxed), held, heap })
}

/// A block of at least `bytes` for relations, from the one block policy (decision 0342): a run bound to the device
/// may read it in place (a pool slab when one is spare), a CPU run's never (Q58); its relations are made and
/// dropped, so it keeps a free list.
fn lease(bytes: usize, life: Life) -> Result<Block, String> {
    lease_room(bytes.max(life.block_bytes()))
}

/// `lease`'s block of `bytes`, no more: a relation's alone in it outside a run (`Part::try_new_with`).
fn lease_room(bytes: usize) -> Result<Block, String> {
    use rerail_blocks::alloc::{Need, Writes};
    let device = BLOCK_DEVICE.get().map_or(rerail_blocks::alloc::Device::May, |f| f());
    rerail_blocks::alloc::block(&Need { kind: kind::FACTS, bytes, writes: Writes::Churn, device, ..Need::SLAB })
}

/// What a block of relations asks of the policy about the device: the run's binding's (`placement::block_device`,
/// above this crate, installed by `install_block_device`); `May` until one is installed.
static BLOCK_DEVICE: std::sync::OnceLock<fn() -> rerail_blocks::alloc::Device> = std::sync::OnceLock::new();

/// Installs what a block of relations asks about the device (`placement::block_device`); the first one stays.
pub fn install_block_device(device: fn() -> rerail_blocks::alloc::Device) {
    let _ = BLOCK_DEVICE.set(device);
}

/// A new block for relations of `life` with room for `need` bytes and its others' margin: the chunk's, if a chunk's
/// evaluation runs, counted while it lives.
fn new_open(chunk: &Option<Arc<Chunk>>, life: Life, need: usize) -> Result<Arc<Facts>, String> {
    let f = Arc::new(Facts::new(lease(2 * need + MARGIN, life)?, life));
    if let Some(c) = chunk {
        c.made.lock().unwrap_or_else(|e| e.into_inner()).push(Arc::downgrade(&f));
        c.note();
    }
    let live_blocks = crate::code_rule_dsl::logic::profile::machine::<Live>();
    let mut live = live_blocks.0.lock().unwrap_or_else(|e| e.into_inner());
    live[life as usize].retain(|w| w.strong_count() > 0);
    live[life as usize].push(Arc::downgrade(&f));
    Ok(f)
}

/// A chunk of `need` bytes (aligned to `align`) in the open block of `life`'s relations (the chunk's, if a chunk's
/// evaluation runs), past its others' margin, taken under the block's lock. Checked for room under the lock and taken
/// after it, the room could be another thread's by then, or not in one piece (a free list's), and the chunk failed
/// (an index's 2.5 MB, the data layer's 50 policies on 901 files). A new open block when that one has no such chunk.
fn open_chunk(life: Life, need: usize, align: usize) -> Result<(Arc<Facts>, (usize, usize)), String> {
    let chunk = current_chunk();
    let runs = open_blocks();
    let mut open = chunk.as_ref().map_or(&runs.0[life as usize], |c| &c.open[life as usize]).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(f) = &*open
        && f.block.reserved().saturating_sub(f.block.used()) >= need + MARGIN
        && let Some(c) = f.block.try_alloc_chunk_fit(need, align)
    {
        return Ok((f.clone(), c));
    }
    let f = new_open(&chunk, life, need)?;
    *open = Some(f.clone());
    let c = f.block.try_alloc_chunk_fit(need, align).ok_or_else(|| format!("a new open block holds no chunk of {need} bytes"))?;
    Ok((f, c))
}

/// Where the structures derived from a relation's rows go (its indexes' arrays, `BlockVec`, decision 0384 step 4):
/// the block its rows are written in, and how long they live.
#[derive(Clone)]
pub struct Home {
    facts: Arc<Facts>,
    life: Life,
    /// The room its relation's tail would take to double (its chunks' bytes; none for frozen rows): kept for it, so
    /// an index never makes the relation it indexes move.
    grow: usize,
}

impl Home {
    /// The block.
    pub fn facts(&self) -> &Arc<Facts> {
        &self.facts
    }
}

/// An array of `T`s in a block of relations (decision 0384 step 4): a relation's derived structure (a sorted run's
/// ids, tags, fences, prints), written once where it is computed, in its relation's block when that has room past
/// its relations' margin (`Home`), else in the open block of the relation's life. Its chunk is a region it owns
/// (decision 0491 step 2): back to the block's free list with it, after the GPU's work, and the block kept while it
/// lives. Grows as a vector does, in the same block.
pub struct BlockVec<T: rerail_blocks::Pod> {
    facts: Option<Arc<Facts>>,
    /// Its first `T` in the block (which never moves): read as a vector's buffer is, with no walk to the block.
    ptr: std::ptr::NonNull<T>,
    life: Life,
    at: usize,
    len: usize,
    cap: usize,
    /// Its own chunk, as a region (`None` for a borrowed one, `shared`, and an empty one).
    region: Option<rerail_blocks::region::Mutable<T>>,
    /// A borrowed one's relation, whose directory owns its chunk (decision 0478 step 5b): kept while it lives.
    owner: Option<Part>,
    /// Its block's ledger, beside the pointer: each read and write is checked against the GPU's queued uses of its
    /// chunk (decision 0487 R4), with no walk to the block.
    ledger: Option<Arc<rerail_blocks::ledger::Ledger>>,
    _t: std::marker::PhantomData<T>,
}

// SAFETY: its `T`s lie in the block it keeps, written only through `&mut self`.
unsafe impl<T: rerail_blocks::Pod + Send> Send for BlockVec<T> {}
unsafe impl<T: rerail_blocks::Pod + Sync> Sync for BlockVec<T> {}

impl<T: rerail_blocks::Pod> Default for BlockVec<T> {
    fn default() -> Self {
        BlockVec::new()
    }
}

impl<T: rerail_blocks::Pod> BlockVec<T> {
    /// Empty, with no chunk.
    pub const fn new() -> Self {
        BlockVec { facts: None, ptr: std::ptr::NonNull::dangling(), life: Life::Run, at: 0, len: 0, cap: 0, region: None, owner: None, ledger: None, _t: std::marker::PhantomData }
    }

    /// The `len` `T`s at `at` of `owner`'s block, which `owner`'s relation keeps (its directory's: decision 0478 step
    /// 5b), read where they lie: borrowed, never given back by this, and never written (a write moves them first).
    ///
    /// # Safety
    /// `len` `T`s are written at `at` of `owner`'s block, and `owner`'s relation keeps them while it lives.
    pub unsafe fn shared(owner: &Part, at: usize, len: usize) -> Self {
        if len == 0 {
            return BlockVec::new();
        }
        let facts = owner.facts().clone();
        let ptr = std::ptr::NonNull::new(facts.block.ptr::<T>(at)).expect("a block's chunk");
        let ledger = Some(facts.block.ledger().clone());
        BlockVec { facts: Some(facts), ptr, life: owner.life(), at, len, cap: len, region: None, owner: Some(owner.clone()), ledger, _t: std::marker::PhantomData }
    }

    /// The chunk `into_chunk` handed on, its own again (no directory took it).
    pub fn from_chunk(facts: &Arc<Facts>, life: Life, g: rerail_blocks::relation::Given) -> Self {
        if g.len == 0 {
            return BlockVec::new();
        }
        let ptr = std::ptr::NonNull::new(facts.block.ptr::<T>(g.at)).expect("a block's chunk");
        let ledger = Some(facts.block.ledger().clone());
        let cap = g.kept / size_of::<T>();
        // SAFETY: the chunk was this vector's (`into_chunk`), handed back whole: `kept` bytes, `cap` `T`s.
        let region = Some(unsafe { rerail_blocks::region::Mutable::adopt(&facts.block, g.at, cap, g.kept, "a relation's array") });
        BlockVec { facts: Some(facts.clone()), ptr, life, at: g.at, len: g.len, cap, region, owner: None, ledger, _t: std::marker::PhantomData }
    }

    /// `n` items, those past its length `v`.
    pub fn resize(&mut self, n: usize, v: T)
    where
        T: Copy,
    {
        if n <= self.len {
            if self.owner.is_some() {
                let near = self.facts.clone();
                self.realloc(near.as_ref().map(|f| (f, 0)), self.len);
            }
            self.len = n;
            return;
        }
        if self.owner.is_some() || n > self.cap {
            let near = self.facts.clone();
            self.realloc(near.as_ref().map(|f| (f, 0)), n.max(2 * self.cap));
        }
        self.check(self.len, n - self.len, true);
        // SAFETY: room for `n` past the chunk's start, this vector's alone.
        unsafe {
            for i in self.len..n {
                self.ptr_mut().add(i).write(v);
            }
        }
        self.len = n;
    }

    /// Whether `into_chunk` would hand its chunk on to `facts`' block (or it has none to hand).
    pub fn can_give(&self, facts: &Arc<Facts>) -> bool {
        self.len == 0 || (self.facts.as_ref().is_some_and(|f| Arc::ptr_eq(f, facts)) && self.region.is_some())
    }

    /// Whether it is borrowed (`shared`).
    pub fn is_shared(&self) -> bool {
        self.owner.is_some()
    }

    /// Its chunk, handed on to `facts`' block's owner of it (an order's directory entry, `Relation::try_add_order`)
    /// when it lies there and is its own: (offset, items, kept bytes). Otherwise it stays this vector's (`Err`).
    pub fn into_chunk(mut self, facts: &Arc<Facts>) -> Result<rerail_blocks::relation::Given, Self> {
        if self.len == 0 {
            return Ok(rerail_blocks::relation::Given::default());
        }
        match &self.facts {
            Some(f) if Arc::ptr_eq(f, facts) && self.region.is_some() => {}
            _ => return Err(self),
        }
        let (at, kept) = self.region.take().expect("its own chunk").into_chunk();
        let given = rerail_blocks::relation::Given { at, len: self.len, kept };
        // Handed on: nothing of it is given back here.
        self.facts = None;
        self.ledger = None;
        self.cap = 0;
        self.len = 0;
        self.ptr = std::ptr::NonNull::dangling();
        Ok(given)
    }

    /// Empty, with room for `n` `T`s, near `home` (`None`: a run's open block).
    pub fn with_capacity_in(home: Option<&Home>, n: usize) -> Self {
        let mut v = BlockVec::new();
        v.life = home.map_or(Life::Run, |h| h.life);
        if n > 0 {
            v.realloc(home.map(|h| (&h.facts, h.grow)), n);
        }
        v
    }

    /// The `T`s of `items`, near `home`.
    pub fn from_iter_in(home: Option<&Home>, items: impl IntoIterator<Item = T>) -> Self {
        let items = items.into_iter();
        let mut v = BlockVec::with_capacity_in(home, items.size_hint().0);
        v.extend(items);
        v
    }

    /// Moves its `T`s to a chunk of room for `n` (`n` at least its length): in `near`'s block when it has room past its
    /// relations' margin and the `grow` bytes its relation's tail would take to double, else the open block of its
    /// life.
    fn realloc(&mut self, near: Option<(&Arc<Facts>, usize)>, n: usize) {
        debug_assert!(n >= self.len);
        let bytes = (n * size_of::<T>()).max(1);
        let align = align_of::<T>().max(8);
        let room = |&(f, grow): &(&Arc<Facts>, usize)| f.block.reserved().saturating_sub(f.block.used()) >= bytes + MARGIN + grow;
        let (facts, (at, size)) = match near.filter(room).and_then(|(f, _)| f.block.try_alloc_chunk_fit(bytes, align).map(|c| (f.clone(), c))) {
            Some(found) => found,
            None => {
                let life = if self.life == Life::Device { Life::Run } else { self.life };
                open_chunk(life, bytes, align).unwrap_or_else(|e| panic!("a block for {bytes} bytes of an index: {e}"))
            }
        };
        if self.len > 0 {
            self.check(0, self.len, false);
            // SAFETY: the old chunk holds `len` written `T`s; the new one is this vector's alone, apart from it.
            // copy: growth: an array that outgrew its chunk, moved once, as a vector's growth moves it
            unsafe { std::ptr::copy_nonoverlapping(self.as_ptr(), facts.block.ptr::<T>(at), self.len) };
        }
        self.free();
        let cap = size / size_of::<T>();
        // SAFETY: the chunk was just taken for this vector: `size` bytes, `cap` `T`s.
        let mut region = unsafe { rerail_blocks::region::Mutable::adopt(&facts.block, at, cap, size, "a relation's array") };
        self.ptr = std::ptr::NonNull::new(region.as_mut_ptr()).expect("a block's chunk");
        self.region = Some(region);
        self.ledger = Some(facts.block.ledger().clone());
        self.facts = Some(facts);
        self.at = at;
        self.cap = cap;
    }

    /// Its chunk given back: its region's last handle goes (a borrowed one's chunk is its owner's).
    fn free(&mut self) {
        self.owner = None;
        self.region = None;
        self.facts = None;
        self.ledger = None;
        self.ptr = std::ptr::NonNull::dangling();
        self.cap = 0;
    }

    /// Room for `extra` more `T`s.
    pub fn reserve(&mut self, extra: usize) {
        let need = self.len + extra;
        if need > self.cap {
            // Grown where it lies: its relation's room was kept when it was placed.
            let near = self.facts.clone();
            self.realloc(near.as_ref().map(|f| (f, 0)), need.max(2 * self.cap));
        }
    }

    #[inline]
    pub fn push(&mut self, v: T) {
        if self.len == self.cap {
            self.reserve(1);
        }
        self.check(self.len, 1, true);
        // SAFETY: `len` is within the chunk's room, this vector's alone.
        unsafe { self.ptr_mut().add(self.len).write(v) };
        self.len += 1;
    }

    // copy: derived: an array's values written once where they are computed (a merged run's)
    pub fn extend_from_slice(&mut self, src: &[T]) {
        self.reserve(src.len());
        self.check(self.len, src.len(), true);
        // SAFETY: room for `src` past `len` is this vector's alone, apart from `src`.
        // copy: derived: the values `extend_from_slice` writes
        unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), self.ptr_mut().add(self.len), src.len()) };
        self.len += src.len();
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// `T`s it has room for.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    #[inline]
    pub fn as_ptr(&self) -> *const T {
        self.ptr.as_ptr()
    }

    /// The CPU reads (or writes) `n` `T`s from its `from`th: none may be under a GPU write (or use) of its chunk
    /// queued (decision 0487 R4). With none queued on its block, one load.
    #[inline]
    #[track_caller]
    fn check(&self, from: usize, n: usize, write: bool) {
        if let Some(l) = &self.ledger {
            let (at, len) = (self.at + from * size_of::<T>(), (n * size_of::<T>()).max(1));
            if write { l.cpu_write(at, len) } else { l.cpu_read(at, len) }
        }
    }

    #[inline]
    fn ptr_mut(&mut self) -> *mut T {
        self.as_ptr().cast_mut()
    }

    /// The block it lies in, if it has a chunk.
    pub fn facts(&self) -> Option<&Arc<Facts>> {
        self.facts.as_ref()
    }

    /// Its first `T`'s offset in its block (0 with no chunk): where a reader that binds the block finds it.
    pub fn offset(&self) -> usize {
        self.at
    }

    /// `n` `T`s near `home`, not written: for a writer other than the host (the device) to write before any read.
    ///
    /// # Safety
    /// Nothing reads them before that writer wrote every one.
    pub unsafe fn unwritten_in(home: Option<&Home>, n: usize) -> Self {
        let mut v = BlockVec::with_capacity_in(home, n);
        v.len = n;
        v
    }
}

impl<T: rerail_blocks::Pod> std::ops::Deref for BlockVec<T> {
    type Target = [T];
    #[inline]
    fn deref(&self) -> &[T] {
        self.check(0, self.len, false);
        // SAFETY: `len` `T`s are written at `as_ptr`, in the block it keeps.
        unsafe { std::slice::from_raw_parts(self.as_ptr(), self.len) }
    }
}

impl<T: rerail_blocks::Pod> std::ops::DerefMut for BlockVec<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        if self.owner.is_some() {
            // A borrowed one is its owner's: written, it moves to a chunk of its own first.
            let near = self.facts.clone();
            self.realloc(near.as_ref().map(|f| (f, 0)), self.len);
        }
        self.check(0, self.len, true);
        // SAFETY: as `deref`, and `&mut self` is the one writer.
        unsafe { std::slice::from_raw_parts_mut(self.ptr_mut(), self.len) }
    }
}

impl<T: rerail_blocks::Pod> Extend<T> for BlockVec<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, items: I) {
        let items = items.into_iter();
        self.reserve(items.size_hint().0);
        for v in items {
            self.push(v);
        }
    }
}

impl<'a, T: rerail_blocks::Pod + Copy> Extend<&'a T> for BlockVec<T> {
    fn extend<I: IntoIterator<Item = &'a T>>(&mut self, items: I) {
        self.extend(items.into_iter().copied());
    }
}

impl<T: rerail_blocks::Pod> Clone for BlockVec<T> {
    /// A copy in the same block (or its life's open block): a derived structure written again, as a merge writes one.
    fn clone(&self) -> Self {
        if let Some(owner) = &self.owner {
            // Borrowed: another borrow of the same items, where they lie (never written in place).
            // SAFETY: `owner` keeps them, as it keeps this one's.
            return unsafe { BlockVec::shared(owner, self.at, self.len) };
        }
        let mut v = BlockVec::new();
        v.life = self.life;
        if self.len > 0 {
            v.realloc(self.facts.as_ref().map(|f| (f, 0)), self.len);
            self.check(0, self.len, false);
            // SAFETY: room for `len` `T`s, this vector's alone.
            // copy: derived: a derived array written again for a writer of its own (a merge of a shared run)
            unsafe { std::ptr::copy_nonoverlapping(self.as_ptr(), v.ptr_mut(), self.len) };
            v.len = self.len;
        }
        v
    }
}

impl<T: rerail_blocks::Pod> Drop for BlockVec<T> {
    fn drop(&mut self) {
        self.free();
    }
}

impl<T: rerail_blocks::Pod + PartialEq> PartialEq for BlockVec<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<'a, T: rerail_blocks::Pod> IntoIterator for &'a BlockVec<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T: rerail_blocks::Pod + std::fmt::Debug> std::fmt::Debug for BlockVec<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// The bytes a relation of `k` 8-byte columns and `rows` rows takes in its block: its descriptor and directories,
/// and its chunks as `Rows::try_reserve` makes them (quarter steps of doubling groups, each chunk aligned).
fn room_for(k: usize, rows: usize) -> usize {
    room_for_widths(&vec![8; k], rows)
}

/// `room_for`, of columns `widths` bytes wide.
fn room_for_widths(widths: &[u32], rows: usize) -> usize {
    let min_log = rerail_blocks::rows::MIN_LOG;
    let align = |bytes: usize| if bytes >= rerail_blocks::block::PAGE { rerail_blocks::block::PAGE } else { 64 };
    let mut chunks = 0;
    let mut bytes = 0;
    while rerail_blocks::rows::capacity(chunks, min_log) < rows as u64 {
        let r = rerail_blocks::rows::chunk_rows(chunks as usize, min_log) as usize;
        bytes += widths.iter().map(|&w| r * w as usize + align(r * w as usize)).sum::<usize>();
        chunks += 1;
    }
    bytes + (widths.len() + 1) * size_of::<rerail_blocks::rows::ColDir>() + 4096
}

/// A block of the device's own of at least `bytes`: a relation alone in it, from the block policy (decision 0342) as
/// every block is (Q205). Exact, and counted as a device join's buffers are (bounded inside the join's budget, a
/// refusal running it on the CPU). Without a device (none opened: a Linux host with no GPU, unless `RERAIL_GPU=cpu`),
/// anonymous memory, which the CPU's reads take as they take any block.
fn lease_exact(bytes: usize, life: Life) -> Result<Block, String> {
    use rerail_blocks::alloc::{Budget, Device, Need, Size, Writes};
    let device = if cfg!(any(target_os = "macos", target_os = "linux", windows)) && life == Life::Device && rerail_accel::gpu::shared::layer().is_some() {
        Device::Must
    } else {
        Device::No
    };
    rerail_blocks::alloc::block(&Need { kind: kind::FACTS, bytes, writes: Writes::Once, device, size: Size::Exact, budget: Budget::Join })
}

/// A relation of `k` columns holding `vals` (row by row), alone in a block of anonymous memory, as a run's block is
/// when the device's spare budget refuses it a slab: bound on the device by a wrap of its used extent
/// (`Block::device_buffer`), which what is added to the block after that wrap lies past.
#[cfg(any(test, feature = "test-hooks"))]
pub fn anon_in_block(k: usize, vals: &[i64]) -> InBlock {
    let rows = vals.len() / k;
    let facts = Arc::new(Facts::new(Block::anon(kind::FACTS, 2 * room_for(k, rows) + 8 * MARGIN), Life::Run));
    let rel = Relation::try_create(&facts.block, &spec(k), None).expect("room for the relation");
    assert!(rel.rows.try_reserve(&facts.block, rows as u64));
    // SAFETY: rows `0..rows` are reserved, and this thread alone writes them before any reader looks.
    unsafe {
        rel.rows.claim_reserved(rows as u32);
        for (i, row) in vals.chunks(k).enumerate() {
            for (c, &v) in row.iter().enumerate() {
                rel.rows.put(i as u32, c, v);
            }
        }
    }
    let at = facts.block.offset_of(rel);
    InBlock { facts, at, rows: rows as u32, dead: 0, kills: None, part: None }
}

/// The room a relation of `rows` rows takes past its rows for an index at every entry of its directory, and the
/// directory (`Part::with_index_room`).
fn index_room(rows: usize) -> usize {
    let cap = rows.next_power_of_two().max(1);
    let one = 4 * (cap + 1) + 4 * rows.max(1) + 3 * 64;
    rerail_blocks::relation::ADDED_INDEXES * (one + size_of::<rerail_blocks::relation::Index>())
}

/// The run table of a relation under key runs (`Part::over_key_runs`), for its writer.
pub struct KeyRuns {
    t: *mut u32,
    runs: usize,
    /// The relation whose block holds it.
    part: Part,
}

impl KeyRuns {
    /// Run `k`: its rows from `first` (past the run's before; the first's 0) hold ids of high bits `key`.
    pub fn put(&self, k: usize, first: u32, key: u32) {
        assert!(k < self.runs, "run {k} of {}", self.runs);
        assert!(self.part.len() == 0, "a run set after its relation's rows");
        // SAFETY: a table of `runs` runs in the relation's block, which `part` keeps; read only once rows are written.
        unsafe { rerail_blocks::rows::put_run(self.t, k, first, key) };
    }

    /// Its offset in its block (a device writer's).
    pub fn at(&self) -> usize {
        self.part.facts.block.offset_of(self.t.cast_const())
    }
}

/// A relation of `k` values a row, 8 bytes each, read in their order.
fn spec(k: usize) -> Spec {
    Spec { widths: vec![8; k], order: Vec::new(), indexes: Vec::new(), min_log: rerail_blocks::rows::MIN_LOG }
}

/// One relation of a block: a segment or a tail, shared by its holders; its chunks given back to the block's free
/// list when the last lets go (decision 0342). A reader in place holds it too (`InBlock::part`).
#[derive(Clone)]
pub struct Part {
    held: Arc<Held>,
    /// Its rows are the relation's first `cap` (`u32::MAX`: all of them): a segment shared with the tail it was
    /// taken from (`Rows::clone`, `shared`), whose writer goes on past it. A capped part is never written.
    cap: u32,
}

/// A part's relation, where it lies.
pub struct Held {
    facts: Arc<Facts>,
    life: Life,
    rel: *const Relation,
    /// Each column resolved once (its directory, which never moves while the relation is in its block): a read pays
    /// the chunk arithmetic and two loads, not the descriptor's walk too.
    cols: Arc<[ColRead]>,
    /// The loans of room past its rows to another writer outstanding (`Part::room`, `Lent::after`): a first from its
    /// rows' end, the next ones each after the one before.
    lent: std::sync::atomic::AtomicU32,
    /// Its rows' count while its writer is free for a relation started from its frozen rows to take (`Rows::freeze`,
    /// `Rows::shared`, decision 0384 step 2); `u32::MAX` while it has a writer (or none may write past its rows).
    free_at: std::sync::atomic::AtomicU32,
    /// The relation it continues (a fork's tail, decision 0384 step 7, `Part::continuing`), kept while it is: its ids
    /// below the base's rows read there.
    base: Option<Part>,
    /// Rows of a continuation's base it took when it moved (`grow`, `moved_room`): its rows start at id 0 then, and the
    /// `Rows` holding it drops the base's segment (`Rows::absorbed`).
    absorbed: u32,
    /// The part whose rows a link reads as its own (`Part::link`), kept while it is: its chunks are not given back.
    aliased: Option<Part>,
}

/// Room past a relation's rows lent to another writer (the device, `Part::room`): rows `at..` of it, which the writer
/// fills and the relation's rows then claim (`Rows::claim_tail`) or read (`Rows::unclaimed`). The first loan starts at
/// its rows' end, a next one after the rows of the one before (`after`); given back when dropped.
pub struct Lent {
    tail: Part,
    pub at: u32,
    /// The relation `tail`'s rows `0..at` were moved from (`Lent::moved`), when its block had no room for the rows
    /// lent: the claim takes `tail` in its place, so the relation stays one (as a push past a full block moves it).
    from: Option<Part>,
}

impl Lent {
    /// The relation lent.
    pub fn tail(&self) -> &Part {
        &self.tail
    }

    /// Room for `n` more rows after this loan's first `written`, on the same relation: a next writer's (the next of a
    /// stratum's groups, writing the same head, decision 0384 step 3). `None` when its block has no room for them.
    pub fn after(&self, written: u32, n: usize) -> Option<Lent> {
        let at = self.at.checked_add(written)?;
        let end = u64::from(at) + n as u64;
        if end > u64::from(u32::MAX) {
            return None;
        }
        self.tail.lent.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let lent = Lent { tail: self.tail.clone(), at, from: None };
        self.tail.rel().rows.try_reserve(&self.tail.facts.block, end).then_some(lent)
    }

    /// `after`, when its block has no room: the relation's rows so far (this loan's first `written` too) moved into
    /// a relation with room for `n` more (`Part::moved_room`), lent past them.
    pub fn moved(&self, written: u32, n: usize) -> Lent {
        self.tail.moved_room(self.at + written, n)
    }
}

impl Drop for Lent {
    fn drop(&mut self) {
        self.tail.lent.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// Rows a wave's writers write past a relation's rows, at once and where they will lie (decision 0353 step 6, row 8:
/// the CPU's head rows, as the device's are). A writer claims slots a span at a time (`claim`); room is made under a
/// lock as the claims pass what is reserved, in the relation's tail's block, and where that has no room left, in a
/// relation of its own (a next region), which the relation takes as its tail with them (`finish`). Its slots are
/// `0..slots()`, in order across the regions, each a row of one region. Rows are settled where they lie, and the
/// wave's end lays the kept ones out at slots `0..m` (`lay_out`), which `finish` makes the relation's rows: what the
/// writers left between their rows (a span's unwritten rest, a repeat dropped) is past them, room the next wave's
/// rows are written into.
pub struct WaveRoom {
    k: usize,
    regions: [std::sync::OnceLock<Region>; WAVE_REGIONS],
    cursor: Mutex<WaveCursor>,
}

/// The regions a wave's room may have (`WaveRoom`): each next one has room for twice the rows before it.
const WAVE_REGIONS: usize = 24;

/// Rows a writer claims of a wave's room at a time (`WaveRoom::claim`).
pub const WAVE_SPAN: u32 = 1024;

/// A region of a wave's room: rows `at..` of `part`, its first slot `first`. `lent`: the loan of the relation's own
/// tail it is (the first region's, `Rows::tail_room`); none, a relation of its own.
struct Region {
    part: Part,
    at: u32,
    first: u32,
    lent: Option<Lent>,
}

#[derive(Default)]
struct WaveCursor {
    /// The region claims go on in, and the rows claimed in it.
    region: usize,
    used: u32,
    /// Every region's rows claimed (`used` the last's).
    ends: Vec<u32>,
}

impl WaveRoom {
    /// Room past `rows`' rows, `k` a row: its tail's own (lent, or the tail moved where its block has no room, as
    /// a push past a full block moves it), else (or `own`: rows a fold reads, not the relation's) a relation of its
    /// own.
    pub fn open(rows: &Rows, k: usize, own: bool) -> WaveRoom {
        let first = if own { None } else { rows.tail_room(WAVE_SPAN as usize).or_else(|| rows.tail_moved(WAVE_SPAN as usize)) };
        let region = match first {
            Some(lent) => Region { part: lent.tail.clone(), at: lent.at, first: 0, lent: Some(lent) },
            None => Region { part: Part::for_writes(k, WAVE_SPAN as usize), at: 0, first: 0, lent: None },
        };
        let room = WaveRoom { k, regions: std::array::from_fn(|_| std::sync::OnceLock::new()), cursor: Mutex::new(WaveCursor { region: 0, used: 0, ends: vec![0] }) };
        let _ = room.regions[0].set(region);
        room
    }

    /// Values a row.
    pub fn k(&self) -> usize {
        self.k
    }

    #[inline(always)]
    fn region(&self, r: u8) -> &Region {
        self.regions[r as usize].get().expect("a region claimed")
    }

    /// `n` slots in a row, claimed for one writer: their region and its first row there.
    pub fn claim(&self, n: u32) -> (u8, u32) {
        let mut cur = self.cursor.lock().unwrap_or_else(|e| e.into_inner());
        let r = cur.region;
        let reg = self.region(r as u8);
        let end = u64::from(reg.at) + u64::from(cur.used) + u64::from(n);
        if end <= u64::from(u32::MAX) && reg.part.reserve(end) {
            let row = reg.at + cur.used;
            cur.used += n;
            *cur.ends.last_mut().expect("a region") = cur.used;
            return (r as u8, row);
        }
        // Its block has no room left: a relation of its own, alone in a block with room for twice the rows before.
        assert!(r + 1 < WAVE_REGIONS, "a wave's rows past {WAVE_REGIONS} regions");
        let before: u32 = cur.ends.iter().sum();
        let rows = (n as usize).max(before as usize);
        let part = Part::for_writes_with_room(self.k, rows, 8 * self.k.max(1) * rows).unwrap_or_else(|e| panic!("a wave's rows past their block: {e}"));
        let _ = self.regions[r + 1].set(Region { part, at: 0, first: before, lent: None });
        cur.region = r + 1;
        cur.used = n;
        cur.ends.push(n);
        (cur.region as u8, 0)
    }

    /// Slots claimed (`claim`) given back unwritten, where they are the last claimed: a writer's upper bound past
    /// the rows it wrote (the device's). Whether they were.
    pub fn give_back(&self, r: u8, row: u32, n: u32) -> bool {
        let mut cur = self.cursor.lock().unwrap_or_else(|e| e.into_inner());
        if cur.region != r as usize || self.region(r).at + cur.used != row + n {
            return false;
        }
        cur.used -= n;
        *cur.ends.last_mut().expect("a region") = cur.used;
        true
    }

    /// The slots claimed.
    pub fn slots(&self) -> u32 {
        self.cursor.lock().unwrap_or_else(|e| e.into_inner()).ends.iter().sum()
    }

    /// Region `r`'s relation: a writer other than the host writes its rows there (the device, through `in_block`).
    pub fn part(&self, r: u8) -> &Part {
        &self.region(r).part
    }

    /// The slot of region `r`'s row `row`.
    #[inline]
    pub fn global(&self, r: u8, row: u32) -> u32 {
        let reg = self.region(r);
        reg.first + (row - reg.at)
    }

    /// Slot `g`'s region and row (`g` below `slots()`).
    pub fn slot(&self, g: u32) -> (u8, u32) {
        let mut r = 0;
        while let Some(next) = self.regions.get(r + 1).and_then(|o| o.get()) {
            if g < next.first {
                break;
            }
            r += 1;
        }
        let reg = self.region(r as u8);
        (r as u8, reg.at + (g - reg.first))
    }

    /// Row `row` of region `r`'s value in column `c`, where a writer wrote it.
    #[inline(always)]
    pub fn get(&self, r: u8, row: u32, c: usize) -> i64 {
        self.region(r).part.get(row, c)
    }

    /// Writes row `row` of region `r`.
    ///
    /// # Safety
    /// The row is claimed by the caller (`claim`), and no one else reads or writes it meanwhile.
    #[inline(always)]
    pub unsafe fn put(&self, r: u8, row: u32, vals: &[i64]) {
        let part = &self.region(r).part;
        for (c, &v) in vals.iter().enumerate() {
            // SAFETY: claimed, so reserved; the caller's.
            unsafe { part.put(row, c, v) };
        }
    }

    /// Row `row` of region `r`.
    #[inline(always)]
    pub fn row(&self, r: u8, row: u32) -> Row {
        let part = &self.region(r).part;
        Row::from_fn(self.k, |c| part.get(row, c))
    }

    /// The wave's kept rows laid out at slots `0..src.len()`, slot `t` taking the row slot `src[t]` holds (`src`
    /// distinct, below `slots()`), each moved once along its cycle, in place; the other slots hold the rest, in no
    /// order. With them, a value per slot (`side`, a row's level) moved the same way.
    ///
    /// # Safety
    /// No writer writes the room meanwhile, and every slot `src` names was written.
    pub unsafe fn lay_out(&self, src: &[u32], side: Option<&mut Vec<u32>>) {
        let n = self.slots();
        let at: Vec<(u8, u32)> = (0..n).map(|g| self.slot(g)).collect();
        permute(n as usize, src, |g| self.row(at[g as usize].0, at[g as usize].1), |g, row: &Row| {
            // SAFETY: the caller's: no writer meanwhile.
            unsafe { self.put(at[g as usize].0, at[g as usize].1, row) }
        }, side);
    }

    /// Slots `0..m` (laid out, `lay_out`) made `rows`' rows, past its rows: its own tail's claimed, the regions past
    /// it appended as its tail (`Rows::append_part`, not copied). The room's loans are given back.
    ///
    /// # Safety
    /// No writer writes the room after, and slots `0..m` hold the rows.
    pub unsafe fn finish(self, rows: &mut Rows, m: u32) {
        let ends = std::mem::take(&mut self.cursor.lock().unwrap_or_else(|e| e.into_inner()).ends);
        let mut left = m;
        for (r, slot) in self.regions.into_iter().enumerate() {
            let Some(reg) = slot.into_inner() else { break };
            let n = left.min(ends.get(r).copied().unwrap_or(0));
            left -= n;
            match &reg.lent {
                Some(_) if n == 0 => {}
                // SAFETY: the caller's: written, the writers done.
                Some(lent) => unsafe { rows.claim_tail(lent, n) },
                None if n > 0 => {
                    // SAFETY: the same.
                    unsafe { reg.part.written(n) };
                    rows.append_part(reg.part);
                }
                None => {}
            }
        }
        debug_assert_eq!(left, 0, "rows laid out past the slots claimed");
    }
}

/// Lays rows out in place over slots `0..n`: slot `t` takes the row slot `src[t]` holds (`src` distinct, below `n`),
/// the others the rest in no order; each row read once and written once along its cycle (one row held aside a
/// cycle). `side`, a value per slot moved with its row.
pub fn permute(n: usize, src: &[u32], read: impl Fn(u32) -> Row, write: impl Fn(u32, &Row), mut side: Option<&mut Vec<u32>>) {
    if src.iter().enumerate().all(|(t, &s)| s as usize == t) {
        return;
    }
    // The whole permutation: the slots no target takes after `src`'s, in order.
    let mut used = vec![0u64; n.div_ceil(64)];
    for &s in src {
        used[s as usize / 64] |= 1 << (s % 64);
    }
    let mut perm: Vec<u32> = Vec::with_capacity(n);
    perm.extend_from_slice(src);
    perm.extend((0..n as u32).filter(|&s| used[s as usize / 64] & (1 << (s % 64)) == 0));
    debug_assert_eq!(perm.len(), n, "distinct sources");
    let mut done = vec![0u64; n.div_ceil(64)];
    for t in 0..n as u32 {
        if done[t as usize / 64] & (1 << (t % 64)) != 0 || perm[t as usize] == t {
            continue;
        }
        let held = read(t);
        let held_side = side.as_deref().map(|s| s[t as usize]);
        let mut cur = t;
        loop {
            done[cur as usize / 64] |= 1 << (cur % 64);
            let s = perm[cur as usize];
            if s == t {
                write(cur, &held);
                if let (Some(sv), Some(v)) = (side.as_deref_mut(), held_side) {
                    sv[cur as usize] = v;
                }
                break;
            }
            write(cur, &read(s));
            if let Some(sv) = side.as_deref_mut() {
                sv[cur as usize] = sv[s as usize];
            }
            cur = s;
        }
    }
}

/// The relations of many parts merged into one for a reader that binds one (`Rows::one`), and their bytes: copies the
/// device's reads of a relation of many parts would not make.
/// On the shared object (decision 0442): a profile's count over the runs that share it.
#[derive(Default)]
struct Merged([std::sync::atomic::AtomicUsize; 2]);

fn merged_counts() -> std::sync::Arc<Merged> {
    crate::code_rule_dsl::logic::profile::shared::<Merged>()
}

/// The relations merged into one so far (`Rows::one`), and their bytes, for a profile.
pub fn merged() -> (usize, usize) {
    let m = merged_counts();
    (m.0[0].load(std::sync::atomic::Ordering::Relaxed), m.0[1].load(std::sync::atomic::Ordering::Relaxed))
}

/// The bytes of a run's dead relations' chunks given back to the OS so far (`Held`'s drop), for a profile: the run's.
#[derive(Default)]
struct GivenBack(std::sync::atomic::AtomicUsize);

/// The bytes of the run's dead relations' chunks given back to the OS so far.
pub fn given_back() -> usize {
    crate::code_rule_dsl::logic::profile::local::<GivenBack>().0.load(std::sync::atomic::Ordering::Relaxed)
}

impl Drop for Held {
    fn drop(&mut self) {
        // SAFETY: the relation lies in `facts`' block, and this was its last holder. Its chunks go to the block's
        // free list, their whole pages past the list's link back to the OS (Q76: a block lives while any relation in
        // it does, so a chunk's dead relations otherwise kept their pages resident; about 500 MB of a 300-file cold
        // run's footprint, Q76).
        // A read-only block is never written: nothing is given back to it.
        if !self.facts.block.is_writable() {
            return;
        }
        let given = unsafe { (*self.rel).release(&self.facts.block) };
        self.facts.given.0.fetch_add(given, std::sync::atomic::Ordering::Relaxed);
    }
}

impl std::ops::Deref for Part {
    type Target = Held;
    #[inline(always)]
    fn deref(&self) -> &Held {
        &self.held
    }
}

// SAFETY: the relation lies in `facts`' block, which the part keeps; its writer is the one `Rows` holding it.
unsafe impl Send for Held {}
unsafe impl Sync for Held {}

/// A column's blocked Bloom filter (decision 0488, `rerail_blocks::bloom`) in a block of the run's relations: the
/// block the relations whose keys it holds are made in, so the device binds it as it binds them and reads it where
/// it lies, as the host does. Its chunks go back to the block's free list when it is dropped.
pub struct FactsBloom {
    facts: Arc<Facts>,
    at: usize,
}

// SAFETY: the filter lies in `facts`' block, which it keeps; its one writer is its owner (`Bloom::add_hashed`).
unsafe impl Send for FactsBloom {}
unsafe impl Sync for FactsBloom {}

impl FactsBloom {
    /// An empty filter of `words` words (a power of two), in the open block of the run's relations or a new one.
    pub fn new(words: usize) -> FactsBloom {
        FactsBloom::try_new(words).unwrap_or_else(|e| panic!("a filter of {words} words: {e}"))
    }

    pub fn try_new(words: usize) -> Result<FactsBloom, String> {
        let life = Life::Run;
        let need = 8 * words + 128;
        let chunk = crate::code_rule_dsl::logic::profile::bound::<InChunk>().0;
        let runs = crate::code_rule_dsl::logic::profile::local::<Open>();
        let mut open = chunk.as_ref().map_or(&runs.0[life as usize], |c| &c.open[life as usize]).lock().unwrap_or_else(|e| e.into_inner());
        for fresh in [false, true] {
            let facts = match &*open {
                Some(f) if !fresh && f.block.reserved().saturating_sub(f.block.used()) >= need + MARGIN => f.clone(),
                _ => {
                    let f = new_open(&chunk, life, need)?;
                    *open = Some(f.clone());
                    f
                }
            };
            if let Some(at) = rerail_blocks::bloom::Bloom::try_create(&facts.block, words) {
                return Ok(FactsBloom { facts, at });
            }
        }
        unreachable!("a new block of {} bytes holds a filter of {need}", 2 * need + MARGIN)
    }

    /// Its block, which the device binds it by.
    pub fn facts(&self) -> &Arc<Facts> {
        &self.facts
    }

    /// Its descriptor's offset in its block.
    pub fn at(&self) -> usize {
        self.at
    }
}

impl std::ops::Deref for FactsBloom {
    type Target = rerail_blocks::bloom::Bloom;
    #[inline(always)]
    fn deref(&self) -> &rerail_blocks::bloom::Bloom {
        // SAFETY: `at` is a filter's descriptor in `facts`' block, which this keeps.
        unsafe { &*self.facts.block.ptr::<rerail_blocks::bloom::Bloom>(self.at) }
    }
}

impl Drop for FactsBloom {
    fn drop(&mut self) {
        let block = &self.facts.block;
        // SAFETY: as `deref`; this is its one holder, and nothing reads it after.
        unsafe { &*block.ptr::<rerail_blocks::bloom::Bloom>(self.at) }.release(block);
    }
}

impl Part {
    /// An empty relation of `k` columns with room for `rows` rows, in the open block or a new one.
    fn new(life: Life, k: usize, rows: usize) -> Part {
        Part::try_new(life, k, rows).unwrap_or_else(|e| panic!("a relation of {k} columns and {rows} rows: {e}"))
    }

    /// `new`, or why no block could be had (a device block past the device's budget).
    fn try_new(life: Life, k: usize, rows: usize) -> Result<Part, String> {
        Part::try_new_with(life, k, rows, 0, false)
    }

    /// `try_new`, a block of its own (a device relation's) with `extra` bytes past the relation's room.
    /// `rewritten`: its rows' values change in place (`for_writes_with_room`): its block alone keeps a free list, and
    /// its indexes are whole chunks the next takes again (`Relation::rewritten`).
    fn try_new_with(life: Life, k: usize, rows: usize, extra: usize, rewritten: bool) -> Result<Part, String> {
        Part::try_new_spec(life, spec(k), rows, extra, rewritten)
    }

    /// `try_new_with`, of columns `spec`'s widths.
    fn try_new_spec(life: Life, spec: Spec, rows: usize, extra: usize, rewritten: bool) -> Result<Part, String> {
        let k = spec.widths.len();
        let need = room_for_widths(&spec.widths, rows);
        // A device relation is written once at its size, and lives as the device's buffers did: alone in a block of
        // its size (leased within the device's budget), given back with it. A run's tail grows: its block has room for
        // it to double.
        // A relation given room past its rows (`extra`: its indexes', `Rows::with_index_room`) is alone in its block
        // too, so its indexes lie with it.
        let alone = life == Life::Device || extra > 0;
        let chunk = crate::code_rule_dsl::logic::profile::bound::<InChunk>().0;
        let (runs, live_blocks) = (open_blocks(), crate::code_rule_dsl::logic::profile::machine::<Live>());
        let mut open = chunk.as_ref().map_or(&runs.0[life as usize], |c| &c.open[life as usize]).lock().unwrap_or_else(|e| e.into_inner());
        for fresh in [false, true] {
            let facts = match &*open {
                _ if alone => {
                    let block = lease_exact(need + extra, life)?;
                    let f = Arc::new(Facts::new(if rewritten { block.with_free_list() } else { block }, life));
                    if let Some(c) = &chunk {
                        c.made.lock().unwrap_or_else(|e| e.into_inner()).push(Arc::downgrade(&f));
                    }
                    if life != Life::Device {
                        let mut live = live_blocks.0.lock().unwrap_or_else(|e| e.into_inner());
                        live[life as usize].retain(|w| w.strong_count() > 0);
                        live[life as usize].push(Arc::downgrade(&f));
                    }
                    f
                }
                Some(f) if !fresh && f.block.reserved().saturating_sub(f.block.used()) >= need + MARGIN => f.clone(),
                _ => {
                    let f = new_open(&chunk, life, need)?;
                    *open = Some(f.clone());
                    f
                }
            };
            if let Some(rel) = Relation::try_create(&facts.block, &spec, None)
                && rel.rows.try_reserve(&facts.block, rows as u64)
            {
                if rewritten {
                    rel.rewritten();
                }
                let cols = (0..k).map(|c| rel.rows.reader(c)).collect();
                let rel = std::ptr::from_ref(rel);
                return Ok(Part { held: Arc::new(Held { facts, life, rel, cols, lent: std::sync::atomic::AtomicU32::new(0), free_at: std::sync::atomic::AtomicU32::new(u32::MAX), base: None, absorbed: 0, aliased: None }), cap: u32::MAX });
            }
        }
        unreachable!("a new block of {} bytes holds a relation of {need}", 2 * need + MARGIN)
    }

    /// A device run's own relation of `k` columns and `rows` rows (decision 0325 step 5), every row the device's to
    /// write (claimed now: `len` is `rows`, and the device writes them all before any reader looks). `Err` when the
    /// device's budget refuses its block.
    ///
    /// Its block has room for one index of its rows (decision 0325 step 5: the index the device probes it by lies
    /// with it, built once; a second key's copies the relation into a block of its own). About 8 to 12 bytes a row,
    /// next to the relation's 8 a column.
    pub fn for_device(k: usize, rows: usize) -> Result<Part, String> {
        Part::for_device_indexed(k, rows, 1)
    }

    /// `for_device` with room for `indexes` indexes of its rows, one for each key its readers probe it by (a
    /// recursive component's relation, decision 0353 step 6): no key of those copies it.
    pub fn for_device_indexed(k: usize, rows: usize, indexes: usize) -> Result<Part, String> {
        Part::claimed(Life::Device, k, rows, indexes)
    }

    /// The same shape in a run's own memory, for the CPU's work (a view's index home, `setfix`): a block of anonymous
    /// memory alone, as `lease_exact` gives a run, so a host with no device makes it too. `Err` when the join's
    /// budget refuses it.
    pub fn for_host(k: usize, rows: usize) -> Result<Part, String> {
        Part::claimed(Life::Run, k, rows, 1)
    }

    /// A relation of `k` columns and `rows` rows alone in a block of `life`'s, every row claimed, with room for
    /// `indexes` indexes of its rows.
    fn claimed(life: Life, k: usize, rows: usize, indexes: usize) -> Result<Part, String> {
        let cap = rows.next_power_of_two().max(1);
        let one = 4 * (cap + 1) + 4 * rows.max(1) + 2 * 64;
        let index = indexes.max(1) * one + rerail_blocks::relation::ADDED_INDEXES * size_of::<rerail_blocks::relation::Index>() + 64;
        let part = Part::try_new_with(life, k, rows, index, false)?;
        // SAFETY: rows `0..rows` are reserved; the caller writes every one before a read.
        unsafe { part.rel().rows.claim_reserved(rows as u32) };
        Ok(part)
    }

    /// Whether nothing else holds its relation (no other part, reader or borrowed array of it).
    pub fn is_unique(&self) -> bool {
        Arc::strong_count(&self.held) == 1
    }

    /// How long its rows live: the blocks it is in.
    pub fn life(&self) -> Life {
        self.life
    }

    /// Its relation's descriptor's offset in its block: what a reader's `rel_*` reads take.
    pub fn rel_at(&self) -> usize {
        self.facts.block.offset_of(self.rel)
    }

    #[inline(always)]
    pub fn rel(&self) -> &Relation {
        // SAFETY: the part keeps its block.
        unsafe { &*self.rel }
    }

    /// The block it lies in.
    pub fn facts(&self) -> &Arc<Facts> {
        &self.facts
    }

    #[inline]
    pub fn len(&self) -> u32 {
        self.rel().rows.len().min(self.cap)
    }

    /// This part's rows as they are now, as a segment: shared, not copied, and blind to rows its writer adds later.
    fn prefix(&self) -> Part {
        Part { held: self.held.clone(), cap: self.len() }
    }

    /// Whether it is a prefix of its relation (`prefix`), which a reader of the relation itself would read past.
    fn capped(&self) -> bool {
        self.cap != u32::MAX
    }

    /// Its row `i`'s value in column `c`, where it lies.
    #[inline(always)]
    pub fn get(&self, i: u32, c: usize) -> i64 {
        self.cols[c].get(i)
    }

    /// An empty relation of `k` columns in a run's block, with room for `rows` rows that a writer other than the
    /// host fills (decision 0325 step 4: the device writes a head's rows in place, through the reads written once,
    /// `rows_at`), then says how many it wrote (`written`).
    pub fn for_writes(k: usize, rows: usize) -> Part {
        Part::new(Life::Run, k, rows)
    }

    /// A relation of `k` columns living `life` long, room for `rows` rows alone in a block, and past them for an index
    /// of its own over its rows at each entry of its directory (`ADDED_INDEXES`), whatever keys its readers probe it by
    /// (`Rows::with_index_room`'s part): no reader's key copies the relation (decision 0353 step 6; room for a count
    /// guessed by its maker, its columns' and two, copied a filtered source probed by its constants' columns too). The
    /// room is the block's address space: its pages are taken as an index is written. Not leased from the device's
    /// budget.
    pub fn with_index_room(life: Life, k: usize, rows: usize) -> Part {
        Part::try_new_with(life, k, rows, index_room(rows), false).unwrap_or_else(|e| panic!("a relation of {k} columns and {rows} rows: {e}"))
    }

    /// `with_index_room` of `k` columns of ids under key runs (an axis's solve, decision 0494 step 7): each column 4
    /// bytes a row, its ids' low bits, their high bits (a document's key) the runs' of one table of `runs` runs in
    /// the relation's block (`rows::ColDir`'s `runs`), which every column reads. The writer sets each run (`KeyRuns::put`)
    /// before it writes a row.
    pub fn over_key_runs(life: Life, k: usize, rows: usize, runs: usize) -> (Part, KeyRuns) {
        let spec = Spec { widths: vec![4; k], order: Vec::new(), indexes: Vec::new(), min_log: rerail_blocks::rows::MIN_LOG };
        let table = 8 * (runs + 2) + 64;
        let part = Part::try_new_spec(life, spec, rows, index_room(rows) + table, false).unwrap_or_else(|e| panic!("an axis of {rows} rows: {e}"));
        let t = rerail_blocks::rows::alloc_runs(&part.facts.block, runs);
        for c in 0..k {
            // SAFETY: the table is in the relation's block, which it lives as long as; its runs are set before a row.
            unsafe { part.rel().rows.set_runs(c, t) };
        }
        let keys = KeyRuns { t, runs, part: part.clone() };
        (part, keys)
    }

    /// `for_writes` alone in a block with `room` bytes past its rows' (decision 0384 step 5): a relation the device
    /// goes on writing round after round (a lattice head, its held rows updated in place, its new keys' rows past
    /// them), its rows' chunks and its key map growing in the block (`reserve`), never moved. Its block reserves
    /// address space: the room is memory only as it is written. `Err` when no block can be had.
    pub fn for_writes_with_room(k: usize, rows: usize, room: usize) -> Result<Part, String> {
        Part::try_new_with(Life::Run, k, rows, room.max(1), true)
    }

    /// Room for `rows` rows in all (its rows' chunks in its block), not moved: for a writer other than the host
    /// (`for_writes_with_room`). Whether the block had it.
    pub fn reserve(&self, rows: u64) -> bool {
        !self.capped() && self.rel().rows.try_reserve(&self.facts.block, rows)
    }

    /// Rows `len..len + n` written by the writer it was made for (`for_writes_with_room`, a round at a time), now its
    /// rows.
    ///
    /// # Safety
    /// The writer has finished, and wrote every value of those rows, within the rows reserved.
    pub unsafe fn written_more(&self, n: u32) {
        assert!(self.len() + n <= self.rel().rows.reserved(), "{n} rows written past {} into room for {}", self.len(), self.rel().rows.reserved());
        // SAFETY: within the rows reserved (asserted); the caller says they are written.
        unsafe { self.rel().rows.claim_reserved(n) };
    }

    /// Its indexes forgotten (`Relation::forget_indexes`): its rows' values were changed in place (a lattice's fold,
    /// decision 0384 step 5).
    ///
    /// # Safety
    /// No reader probes an index of it meanwhile.
    pub unsafe fn forget_indexes(&self) {
        let _g = self.facts.ix.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: under the block's index lock; the caller's.
        unsafe { self.rel().forget_indexes(&self.facts.block) };
    }

    /// Where structures derived from its rows go (`BlockVec`): its block.
    pub fn home(&self) -> Home {
        Home { facts: self.facts.clone(), life: self.life, grow: 0 }
    }

    /// This relation where it lies, `rows` of it, for a reader or writer that binds its block (the device): it keeps
    /// the relation's chunks as the part does.
    pub fn in_block(&self, rows: u32) -> InBlock {
        InBlock { facts: self.facts.clone(), at: self.rel_at(), rows, dead: 0, kills: None, part: Some(self.clone()) }
    }

    /// Row `i`'s value in column `c` written where it lies: a derived value put in its place (a device fold's text
    /// made its symbol, an axis's ids as the host writes them).
    ///
    /// # Safety
    /// Row `i` is reserved, and no reader or other writer touches it meanwhile.
    pub unsafe fn put(&self, i: u32, c: usize, v: i64) {
        debug_assert!(!self.capped(), "a capped part is never written");
        // SAFETY: the caller's.
        unsafe { self.rel().rows.put(i, c, v) };
    }

    /// Its rows' descriptor's offset in its block: what a writer's `rows_at` takes.
    pub fn rows_at(&self) -> usize {
        self.facts.block.offset_of(&raw const self.rel().rows)
    }

    /// Room for `n` rows past its rows, for a writer other than its own (the device, decision 0384 step 3): the first
    /// loan of it (`Lent`; a next one is `Lent::after`). `None` when its room is lent already, or its block has no room
    /// for them (it is never moved for a writer that is not its own).
    pub fn room(&self, n: usize) -> Option<Lent> {
        let at = self.len();
        let end = u64::from(at) + n as u64;
        if self.capped() || end > u64::from(u32::MAX) {
            return None;
        }
        self.lent.compare_exchange(0, 1, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire).ok()?;
        // Made now, so a refusal below gives the loan back.
        let lent = Lent { tail: self.clone(), at, from: None };
        self.rel().rows.try_reserve(&self.facts.block, end).then_some(lent)
    }

    /// Its first `rows` rows (claimed, or written into room lent and finished) moved into a relation of its life with
    /// room for `n` more, lent past them to another writer (the device): where its block has no room for them. A
    /// relation never moves for another writer in place; it moves for its own when the claim takes the moved relation
    /// (`Rows::claim_tail`), as a push past a full block moves it (`grow`). The rows are written once more, as a
    /// vector's growth writes them.
    pub fn moved_room(&self, rows: u32, n: usize) -> Lent {
        let moved = self.moved_whole(rows, (rows as usize + n).max(2 * rows as usize));
        let at = moved.len();
        assert!(moved.rel().rows.try_reserve(&moved.facts.block, u64::from(at) + n as u64), "a new relation's room for its rows");
        moved.lent.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Lent { tail: moved, at, from: Some(self.clone()) }
    }

    /// Rows `0..n` written by the writer it was made for (`for_writes`), now its rows.
    ///
    /// # Safety
    /// The writer has finished, and wrote every value of rows `0..n`, `n` within the rows reserved.
    pub unsafe fn written(&self, n: u32) {
        assert_eq!(self.len(), 0, "rows written into a relation that has some");
        assert!(n <= self.rel().rows.reserved(), "{n} rows written into room for {}", self.rel().rows.reserved());
        // SAFETY: within the rows reserved (asserted); the caller says they are written.
        unsafe { self.rel().rows.claim_reserved(n) };
    }

    /// Appends a row of `k` values, room made first: a full block moves the relation to another.
    #[inline]
    fn push(&mut self, row: &[i64]) {
        debug_assert!(!self.capped(), "a write to a shared prefix");
        debug_assert_eq!(self.lent.load(std::sync::atomic::Ordering::Relaxed), 0, "a push into room lent to another writer");
        let i = self.len();
        if self.rel().rows.reserved() <= i {
            self.grow(u64::from(i) + 1);
        }
        let rows = &self.rel().rows;
        // SAFETY: row `i` is reserved, and this part is the relation's one writer.
        unsafe {
            rows.claim_reserved(1);
            for (c, &v) in row.iter().enumerate() {
                rows.put(i, c, v);
            }
        }
    }

    /// Room for `n` rows: chunks in its block, or the relation moved to a block with room for twice as many.
    fn grow(&mut self, n: u64) {
        debug_assert!(!self.capped(), "a write to a shared prefix");
        if self.rel().rows.try_reserve(&self.facts.block, n) {
            return;
        }
        let moved = self.moved_whole(self.len(), 2 * n as usize);
        *self = moved;
    }

    /// Its first `rows` rows moved into a new relation of its life with room for `room` rows of its own more: a
    /// continuation's base's rows first (it absorbs them: `Held::absorbed`, its ids then from 0), as a relation moves
    /// out of a full block (a vector's growth).
    fn moved_whole(&self, rows: u32, room: usize) -> Part {
        let k = self.rel().rows.ncols as usize;
        let pre = self.base.as_ref().map_or(0, |_| self.rel().base_rows);
        let mut moved = Part::new(self.life, k, room.max(rows as usize) + pre as usize);
        count_move(rows + pre, k);
        if let Some(base) = &self.base {
            // copy: growth: a continuation moved out of a full block takes the rows of its chain with it, once.
            moved.copy_reads(base, pre);
        }
        // copy: growth: a relation moved out of a full block, once, as a vector's growth moves it.
        moved.copy_from(self, 0, rows);
        Arc::get_mut(&mut moved.held).expect("a new relation's one holder").absorbed = pre;
        moved
    }

    /// Appends the first `upto` rows `from` reads, down its chain of bases (a continuation's: its bases' first).
    fn copy_reads(&mut self, from: &Part, upto: u32) {
        let own_from = match &from.base {
            Some(base) => {
                let b = from.rel().base_rows.min(upto);
                self.copy_reads(base, b);
                from.rel().base_rows
            }
            None => 0,
        };
        if upto > own_from {
            self.copy_from(from, 0, upto - own_from);
        }
    }

    /// The rows it reads by id, from 0: a continuation's base's first (`Relation::len_all`), then its own.
    pub fn reads(&self) -> u32 {
        self.base.as_ref().map_or(0, |_| self.rel().base_rows) + self.len()
    }

    /// Whether it continues another relation's rows (`continuing`).
    pub fn is_continuation(&self) -> bool {
        self.base.is_some()
    }

    /// A relation continuing the rows `base` reads (decision 0384 steps 7 and 8, `Relation::try_continue`): its ids
    /// below them read in `base` (and down `base`'s own chain, when it continues another), its own after, in
    /// `base`'s block, `base` kept while it is. A relation started from frozen rows whose writer another took
    /// (`Rows::shared`), or a copy's first row past them (`Rows::clone`), goes on in it: one relation a reader binds
    /// whole. `None` when `base`'s block has no room for it.
    fn continuing(base: &Part) -> Option<Part> {
        let k = base.rel().rows.ncols as usize;
        let rel = Relation::try_continue(&base.facts.block, &spec(k), base.rel(), base.reads())?;
        let cols = (0..k).map(|c| rel.rows.reader(c)).collect();
        let held = Held { facts: base.facts.clone(), life: base.life, rel: std::ptr::from_ref(rel), cols, lent: std::sync::atomic::AtomicU32::new(0),
                          free_at: std::sync::atomic::AtomicU32::new(u32::MAX), base: Some(base.prefix()), absorbed: 0, aliased: None };
        Some(Part { held: Arc::new(held), cap: u32::MAX })
    }

    /// A link of a chain that reads several parts as one relation, nothing copied (decision 0353 step 6,
    /// `Relation::try_link`): in `rows_of`'s block, its ids below `base_rows` `base`'s (in that block, or another:
    /// a far base), then `rows_of`'s rows as they are now, where they lie. `None` when the block cannot hold its
    /// descriptor.
    fn link(base: Option<&Part>, base_rows: u32, rows_of: &Part) -> Option<Part> {
        let k = rows_of.rel().rows.ncols as usize;
        let home = rows_of.facts.clone();
        let rel = Relation::try_link(&home.block, &spec(k), base.map(|b| (&*b.facts.block, b.rel())), base_rows, rows_of.rel(), rows_of.len())?;
        let cols = (0..k).map(|c| rel.rows.reader(c)).collect();
        let rel = std::ptr::from_ref(rel);
        let held = Held { facts: home, life: rows_of.life, rel, cols, lent: std::sync::atomic::AtomicU32::new(0),
                          free_at: std::sync::atomic::AtomicU32::new(u32::MAX), base: base.map(Part::prefix), absorbed: 0, aliased: Some(rows_of.prefix()) };
        Some(Part { held: Arc::new(held), cap: u32::MAX })
    }

    /// `base`'s first `rows` rows read through a link in a device block of its own with room for their indexes
    /// (decision 0353 step 6): where `base`'s block has no room for an index, the index goes there, and a reader binds
    /// both blocks (the link's far base). `None` when no device block is had, or `base` cannot be a far base.
    pub fn far_link(base: &Part, rows: u32) -> Option<Part> {
        let k = base.rel().rows.ncols as usize;
        let room = Part::try_new_with(Life::Device, k, 0, index_room(rows as usize), false).ok()?;
        if Arc::ptr_eq(&room.facts, &base.facts) {
            return None;
        }
        Part::link(Some(base), rows, &room)
    }

    /// The block its chain's far base lies in (`link`), which a reader binds beside its own: none when every
    /// relation it reads lies in its block.
    pub fn far_facts(&self) -> Option<Arc<Facts>> {
        let mut p = self;
        loop {
            let base = p.base.as_ref()?;
            if p.rel().far_base() {
                return Some(base.facts.clone());
            }
            p = base;
        }
    }

    /// Whether it continues `base`'s rows, all it reads (`continuing`).
    fn continues_of(&self, base: &Part) -> bool {
        self.base.as_ref().is_some_and(|b| Arc::ptr_eq(&b.held, &base.held) && b.len() == base.len()) && self.rel().base_rows == base.reads()
    }

    /// A relation of its own with `room` rows more holding every row it reads (its chain's absorbed, `moved_whole`):
    /// where a continuation cannot be made in its block. It absorbed every row it reads, its own too, so a `Rows`
    /// opening it as the tail past it (`Rows::opened`) drops every segment (`Rows::absorbed`): with only its base's
    /// counted, a relation of its own (no base: a kept relation bound over its read-only mapping, or one whose block is
    /// full) kept its segment as well, the tail's first rows a second copy of its rows read at the ids past them, and
    /// the rows pushed after read at no id (an edit's maintained `kind` lost the new nodes').
    fn moved_all(&self, room: usize) -> Part {
        let mut moved = self.moved_whole(self.len(), room);
        Arc::get_mut(&mut moved.held).expect("a new relation's one holder").absorbed = self.reads();
        moved
    }

    /// Appends rows `lo..hi` of `from` (as many columns).
    fn copy_from(&mut self, from: &Part, lo: u32, hi: u32) {
        let n = hi - lo;
        let at = self.len();
        self.grow(u64::from(at + n));
        let (src, dst) = (&from.rel().rows, &self.rel().rows);
        // SAFETY: rows `at..at + n` are reserved, and this part is the relation's one writer.
        unsafe {
            dst.claim_reserved(n);
            for c in 0..dst.ncols as usize {
                let read = src.reader(c);
                for i in 0..n {
                    dst.put(at + i, c, read.get(lo + i));
                }
            }
        }
    }

    /// The bytes its chunks hold.
    fn capacity_bytes(&self) -> usize {
        self.rel().rows.reserved() as usize * self.rel().rows.ncols as usize * 8
    }
}

#[derive(Default)]
pub struct Rows {
    /// Frozen segments, oldest first: each its first row's id and its relation.
    segs: Vec<(u32, Part)>,
    /// The first id the tail holds: every frozen segment's rows come before it.
    from: u32,
    /// The rows pushed since the last freeze (made at the first).
    tail: Option<Part>,
    /// Values a row (known once a row is pushed or a segment held).
    k: usize,
    /// Rows to make room for when the tail is made (`reserve` before the first push).
    hint: usize,
    /// The blocks its parts go in.
    life: Life,
// copy (0353 step 6): the merged part is a copy. It goes: a run's relations lie in one slab's buffer
// (decision 0353).
    /// Every part merged into one, made once for a reader that binds one relation (`in_block`), where its parts lie
    /// in more blocks than `links` reads.
    merged: std::sync::OnceLock<Part>,
    /// Its parts read as one relation through links (`links`), made once: `None` where they cannot be.
    linked: std::sync::OnceLock<Option<Part>>,
    /// The dead rows' delta of that one relation (`in_block_dead`): kept while its dead bits are the relation's.
    kills: Mutex<Option<Arc<Kills>>>,
    /// The rows its tail held when it was taken as the writer of a frozen relation's (`shared`): given back free at
    /// them when these rows are dropped with none written past them (a run abandoned, its relation not kept).
    took: Option<Took>,
}

/// A frozen relation's writer taken (`Rows::shared`), and the rows its relation held then: given back free at them
/// when dropped with none written past them (a run abandoned, or its rows frozen unchanged).
struct Took {
    writer: Part,
    at: u32,
}

impl Drop for Took {
    fn drop(&mut self) {
        let w = &self.writer;
        if w.rel().rows.len() == self.at && w.lent.load(std::sync::atomic::Ordering::Acquire) == 0 {
            w.free_at.store(self.at, std::sync::atomic::Ordering::Release);
        }
    }
}

/// A relation's dead rows' delta in its block (`Rows::in_block_dead`, `Relation::try_kills`): given back to the
/// block's free list when its last holder drops it, the rows that made it or a reader bound to it (`InBlock::kills`).
/// A relation whose dead rows change with no row added (`setfix::Levels`) makes a delta each time; before, each one
/// stayed in the block as long as the block did (Q178).
pub struct Kills {
    /// Its descriptor's offset in its base's block.
    at: usize,
    /// The relation it is a delta of, and its block: kept while it is.
    base: Part,
    /// Its dead bits are its base's own (`try_kills_own`, decision 0478 step 5d), which its base gives back: only
    /// its descriptor is its own.
    own_dead: bool,
}

impl Drop for Kills {
    fn drop(&mut self) {
        let block = &self.base.facts().block;
        // SAFETY: `try_kills` (or `try_kills_own`) made it in `block`, which `base` keeps, and this was its last holder.
        let given = unsafe {
            let rel = &*block.ptr::<Relation>(self.at);
            if self.own_dead { rel.release(block) } else { rel.release_kills(block) }
        };
        self.base.facts().given.0.fetch_add(given, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Where a relation's rows lie for a reader that binds its block (the device, decision 0325): the block, kept while
/// this lives, and the relation's descriptor's offset in it.
#[derive(Clone)]
pub struct InBlock {
    pub facts: Arc<Facts>,
    pub at: usize,
    pub rows: u32,
    /// The offset of a delta of it whose dead bits mark its dead rows (`Relation::try_kills`); 0 when none is.
    pub dead: usize,
    /// That delta, held while this lives (a device's read of it, `DevRel::in_block`): not given back meanwhile.
    pub kills: Option<Arc<Kills>>,
    /// The relation read, held while this lives: its chunks are not given back meanwhile (decision 0342). `None`
    /// for one no `Part` holds (`anon_in_block`'s).
    pub part: Option<Part>,
}

impl InBlock {
    /// The relation it binds, where it lies.
    /// The block its relation's chain reaches past its own (a far base, `Part::link`), which a reader binds beside
    /// `facts`' (the kernels' `far`): none when it reads only its block.
    pub fn far(&self) -> Option<Arc<Facts>> {
        self.part.as_ref().and_then(Part::far_facts)
    }

    pub fn relation(&self) -> &Relation {
        // SAFETY: `at` is a relation's descriptor in `facts`' block, whose pages `own` keeps.
        unsafe { &*self.facts.block.ptr::<Relation>(self.at) }
    }

    /// The relation's own index on its columns `cols` over its rows (decision 0325 step 5): found where it was built
    /// before (by the device, or an earlier probe), else built now into its block, by the host, as the device builds
    /// it. `None` for a delta (dead rows) or a key past `MAX_KEY`, or when the block has no room for it.
    pub fn own_index(&self, cols: &[u16]) -> Option<usize> {
        self.own_index_for(cols, self.rows as usize)
    }

    /// `own_index`, its buckets sized for `keys` distinct keys (`Relation::try_reserve_index_buckets`), when it is
    /// built now.
    pub fn own_index_for(&self, cols: &[u16], keys: usize) -> Option<usize> {
        self.own_index_if(cols, keys, true)
    }

    /// The relation's own index on `cols` over its rows, if one is built (by the device or a probe): none is built.
    pub fn found_index(&self, cols: &[u16]) -> Option<usize> {
        self.own_index_if(cols, self.rows as usize, false)
    }

    /// An index on `cols` of a relation of no columns whose key values are read elsewhere (a view's home: decision
    /// 0478 step 5e), each row's key hash `hash(i)`: found, or built now. `key` names the view's key columns, by which
    /// it is found again.
    pub fn own_index_hashed(&self, key: &[u16], hash: &dyn Fn(u32) -> u64) -> Option<usize> {
        let r = self.relation();
        if self.dead != 0 || r.base_rows != 0 || key.is_empty() || key.len() > rerail_blocks::relation::MAX_KEY {
            return None;
        }
        let _g = self.facts.ix.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = r.find_index(key, self.rows) {
            return Some(i);
        }
        // SAFETY: under the block's index lock; the room is reserved, then built from the hashes, before it is marked.
        unsafe {
            let room = r.try_reserve_index_buckets(&self.facts.block, key, self.rows, self.rows as usize)?;
            r.build_index_hashed(&room, hash);
            r.index_built(&room);
            Some(room.index)
        }
    }

    fn own_index_if(&self, cols: &[u16], keys: usize, build: bool) -> Option<usize> {
        let r = self.relation();
        // A chain reaching another block (`far`) is indexed by the device, which binds both; the host reads it by part.
        if self.dead != 0 || (r.base_rows != 0 && !r.continues()) || cols.is_empty() || cols.len() > rerail_blocks::relation::MAX_KEY || self.far().is_some() {
            return None;
        }
        let _g = self.facts.ix.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = r.find_index(cols, self.rows) {
            return Some(i);
        }
        if !build {
            return None;
        }
        // SAFETY: under the block's index lock; the relation's rows `0..rows` are written.
        let room = unsafe { r.try_reserve_index_buckets(&self.facts.block, cols, self.rows, keys) }?;
        // SAFETY: the room was just reserved under the lock, and nothing reads it until it is marked built.
        unsafe {
            r.build_index(&room);
            r.index_built(&room);
        }
        Some(room.index)
    }
}

impl Clone for Rows {
    /// The segments shared, and the tail too, not copied: as a segment of the rows it holds now (`Part::prefix`). Its
    /// writer, the one `Rows` holding it, goes on past them; the copy's next row starts a tail of its own.
    fn clone(&self) -> Rows {
        let mut segs = self.segs.clone();
        let mut from = self.from;
        if let Some(t) = self.tail.as_ref().filter(|t| t.len() > 0) {
            segs.push((from, t.prefix()));
            from += t.len();
        }
        Rows { segs, from, tail: None, k: self.k, hint: self.hint, life: self.life, ..Rows::default() }
    }
}

impl Rows {
    /// No rows, of `k` values each.
    pub fn new(k: usize) -> Rows {
        Rows { k, ..Rows::default() }
    }

    /// No rows, of `k` values each, living `life` long.
    pub fn living(k: usize, life: Life) -> Rows {
        Rows { k, life, ..Rows::default() }
    }

    /// `n` rows of `k` values of a relation that does not grow (an `InRel` made of them): one frozen segment, shared
    /// by a relation started from it without a copy. `k` is given, not read off the rows: with none, a relation started
    /// from them (kept empty, grown in a later run) had no width, and its tail took no row.
    pub fn frozen(k: usize, rows: Vec<i64>, n: usize) -> Rows {
        debug_assert_eq!(rows.len(), k * n, "{n} rows of {k} values");
        let mut r = Rows::new(k);
        if n > 0 && !rows.is_empty() {
            r.hint = n;
            r.extend_from_slice(&rows);
        }
        r.freeze(n)
    }

    /// No rows yet, of `k` values each, living `life` long, with room for `rows` rows alone in a block, and past them
    /// for an index of the relation's own at every entry of its directory (`InBlock::own_index_for`): a source held so is
    /// probed by its own indexes, not host tables (decision 0325 step 5). The room is the block's address space: its
    /// pages are taken as an index is written.
    pub fn with_index_room(k: usize, life: Life, rows: usize) -> Rows {
        let part = Part::with_index_room(life, k, rows);
        Rows { k, life, hint: rows, tail: Some(part), ..Rows::default() }
    }

    /// The rows held.
    pub fn rows(&self) -> usize {
        self.segs.iter().map(|(_, p)| p.len() as usize).sum::<usize>() + self.tail.as_ref().map_or(0, |t| t.len() as usize)
    }

    /// The values held, every segment's and the tail's.
    pub fn len(&self) -> usize {
        self.rows() * self.k
    }

    /// The bytes of the values held (the frozen segments counted whole, shared or not).
    pub fn bytes(&self) -> usize {
        self.len() * 8
    }

    /// The bytes the chunks hold: the frozen segments' and the tail's.
    pub fn capacity_bytes(&self) -> usize {
        self.segs.iter().map(|(_, p)| p.capacity_bytes()).sum::<usize>() + self.tail.as_ref().map_or(0, Part::capacity_bytes)
    }

    /// Room for `values` more values in the tail.
    pub fn reserve(&mut self, values: usize) {
        if self.k == 0 {
            return;
        }
        match &mut self.tail {
            Some(t) => t.grow(u64::from(t.len()) + (values / self.k) as u64),
            None => {
                let t = self.opened(values / self.k);
                self.tail = Some(t);
            }
        }
        self.absorbed();
    }

    /// A tail for rows past these (decision 0384 steps 7 and 8): with no rows before it, a relation of its own; past
    /// segments that read as one relation (`chain`), a continuation of it in its block, so the rows stay one
    /// relation however many relations went on from the same rows (forks, copies); where that block has no room, a
    /// relation of its own holding every row (a growth move, `Part::moved_all`), which `absorbed` makes the only part.
    fn opened(&self, rows: usize) -> Part {
        if self.segs.is_empty() {
            return Part::new(self.life, self.k, rows);
        }
        let Some(end) = self.chain() else {
            crate::code_rule_dsl::logic::profile::decision("tails opened past rows of many relations (merged when bound)", || format!("{} segments", self.segs.len()));
            return Part::new(self.life, self.k, rows);
        };
        // Past a kept relation bound over its read-only mapping (decision 0401 F3): nothing is written in its block,
        // and its rows are not moved out of it (0353), so the tail is a relation of its own past them.
        if !end.facts.block.is_writable() {
            crate::code_rule_dsl::logic::profile::decision("tails opened past kept rows read where they lie (merged when bound)", || format!("{} rows", self.from));
            return Part::new(self.life, self.k, rows);
        }
        match Part::continuing(end) {
            Some(t) => {
                crate::code_rule_dsl::logic::profile::decision("relations continued from rows another writer went on from (forks)", || format!("{} rows, {} segments", self.from, self.segs.len()));
                t
            }
            None => end.moved_all(rows),
        }
    }

    /// The last segment, when the segments read as one relation: each a continuation of the one before (decision
    /// 0384 steps 7 and 8), the first a relation of its own, so the last reads every row by id. `None` otherwise.
    fn chain(&self) -> Option<&Part> {
        let end = chain_of(self.segs.iter().map(|(f, p)| (*f, p)))?;
        (end.reads() == self.from).then_some(end)
    }

    /// A continuation tail that moved took its base's rows (`Held::absorbed`): its ids are from 0 now, so the base's
    /// segment goes, and it is one relation of its own.
    fn absorbed(&mut self) {
        if let Some(t) = &self.tail
            && t.absorbed > 0
            && self.from == t.absorbed
        {
            debug_assert_eq!(self.segs.iter().map(|(_, p)| p.len()).sum::<u32>(), t.absorbed, "a continuation's chain of segments");
            self.segs.clear();
            self.from = 0;
        }
    }

    /// Appends rows' values (`k` a row) to the tail.
    #[inline]
    pub fn extend_from_slice(&mut self, rows: &[i64]) {
        if self.k == 0 {
            return;
        }
        debug_assert_eq!(rows.len() % self.k, 0, "rows of {} values", self.k);
        // The rows merged for a reader that binds one relation (`in_block`) are these no longer.
        self.merged.take();
        self.linked.take();
        *self.kills.get_mut().unwrap_or_else(|e| e.into_inner()) = None;
        let n = rows.len() / self.k;
        if self.tail.is_none() {
            let t = self.opened(self.hint.max(n).max(16));
            self.tail = Some(t);
        }
        let tail = self.tail.as_mut().expect("opened");
        // Decision 0487 R4 at a write's start: room lent to another writer (the device, `Part::room`) is that writer's
        // until its loan is claimed; the host writing there would write what the device writes.
        assert_eq!(tail.lent.load(std::sync::atomic::Ordering::Acquire), 0, "a CPU write of a relation's rows with its room lent to the device (claim the loan first)");
        if n > 1 {
            tail.grow(u64::from(tail.len()) + n as u64);
        }
        for row in rows.chunks_exact(self.k) {
            tail.push(row);
        }
        self.absorbed();
    }

    /// The values of rows `lo..hi`, copied into one vector.
    pub fn to_vec(&self, lo: usize, hi: usize, k: usize) -> Vec<i64> {
        let mut out = Vec::with_capacity((hi - lo) * k);
        for id in lo..hi {
            out.extend_from_slice(&self.row(id as u32, k));
        }
        out
    }

    /// These `n` rows with the tail frozen into a segment of its own (moved, not copied): as a closed relation holds
    /// them, for a later run to share (`shared`).
    ///
    /// The tail is held as a prefix of its relation, its rows as they are now, and the relation's writer is free at
    /// them for a relation started from these rows to take (`shared`): that one goes on in the same relation, which a
    /// reader binds whole (decision 0384 step 2).
    pub fn freeze(mut self, n: usize) -> Rows {
        self.took = None;
        if let Some(tail) = self.tail.take().filter(|t| t.len() > 0) {
            self.segs.push((self.from, tail.prefix()));
        }
        self.from = n as u32;
        while self.segs.len() > MAX_SEGMENTS {
            // The newest two, merged: a session's edits add small segments after one large one. Segments that read as
            // one relation (a chain of continuations, decision 0384 step 8) stay one: the two go on in one continuation
            // of the one before them, so a read goes down at most `MAX_SEGMENTS` relations (`rel_get`).
            let chained = self.chain().is_some();
            let (_, b) = self.segs.pop().expect("two segments");
            let (first, a) = self.segs.pop().expect("two segments");
            let made = if chained { self.segs.last().and_then(|(_, base)| Part::continuing(base)) } else { None };
            let mut both = made.unwrap_or_else(|| Part::new(self.life, self.k, (a.len() + b.len()) as usize));
            // copy (0353 step 6): a chain's newest two segments written into one, bounding its length: the small ones a
            // session's edits add, once per `MAX_SEGMENTS` forks of forks. It goes with the merged part (0353).
            both.copy_from(&a, 0, a.len());
            both.copy_from(&b, 0, b.len());
            if both.absorbed > 0 {
                // Moved out of a full block: it holds every row.
                self.segs.clear();
                self.segs.push((0, both.prefix()));
            } else {
                self.segs.push((first, both.prefix()));
            }
        }
        // The last segment's writer, free for a relation started from these rows, at its rows: a prefix of its relation,
        // which a writer goes on past.
        if let Some((first, p)) = self.segs.last() && p.capped() && first + p.len() == self.from && p.rel().rows.len() == p.len() {
            p.free_at.store(p.len(), std::sync::atomic::Ordering::Release);
        }
        self
    }

    /// The `n` rows of a frozen relation, shared: its segments taken (and its tail, if it has one, as the rows it holds
    /// now: `Part::prefix`, not copied), with an empty tail from `n` on, for a relation that starts as them.
    ///
    /// The first relation started so from frozen rows takes their last segment's writer as its tail (decision 0384 step
    /// 2): its rows go on in the same relation, past the segment's (a prefix, which these rows read), so it is one
    /// relation, not shared rows and a tail of its own that a reader binding it must merge. A later one (or one
    /// started while rows were written past the prefix) starts a tail of its own.
    pub fn shared(&self, n: usize) -> Rows {
        let mut segs = self.segs.clone();
        // The tail shared as it is now, not copied (`Part::prefix`).
        if let Some(t) = self.tail.as_ref().filter(|t| t.len() > 0) {
            segs.push((self.from, t.prefix()));
        }
        // Taken by one at a time: free at the segment's rows, and none written past them since.
        let take = |first: u32, p: &Part| {
            (first + p.len()) as usize == n
                && p.free_at.compare_exchange(p.len(), u32::MAX, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire).is_ok()
        };
        match segs.last() {
            Some(&(first, ref p)) if take(first, p) => {
                debug_assert_eq!(p.rel().rows.len(), p.len(), "a free writer's relation holds its prefix's rows");
                let (first, p) = segs.pop().expect("the writer's segment");
                let writer = Part { held: p.held.clone(), cap: u32::MAX };
                let took = Some(Took { writer: writer.clone(), at: writer.len() });
                Rows { segs, from: first, tail: Some(writer), k: self.k, hint: 0, life: self.life, took, ..Rows::default() }
            }
            // A fork (decision 0384 steps 7 and 8): the writer is another's, so this one's first row opens a tail that
            // continues the rows' relation (`opened`): a reader binds the rows as one (`one`), each where it lies.
            _ => Rows { segs, from: n as u32, tail: None, k: self.k, hint: 0, life: self.life, ..Rows::default() },
        }
    }

    /// The rows as one relation in one block, ids from 0: the one part, or every part merged into one (made once,
    /// and kept while these rows are). `None` for no rows.
    pub fn in_block(&self) -> Option<InBlock> {
        let (one, rows) = self.one()?;
        Some(InBlock { facts: one.facts.clone(), at: one.facts.block.offset_of(one.rel), rows, dead: 0, kills: None, part: Some(one.clone()) })
    }

    /// `in_block`, with the rows `dead` marks (bit `id`, as setfix keeps them) dead in it: a delta of the one
    /// relation in its block, no rows of its own, whose dead bits are these (kept, and made again when the dead rows
    /// are others: the one before goes back to the block once no reader holds it). `None` for no rows, or when the
    /// block cannot hold the delta.
    pub fn in_block_dead(&self, dead: &[u64]) -> Option<InBlock> {
        let (one, rows) = self.one()?;
        let block = &one.facts.block;
        let mut kills = self.kills.lock().unwrap_or_else(|e| e.into_inner());
        // A relation's dead rows change with no row added (a level's rows killed, then brought back as their levels
        // are checked, `setfix::Levels`): a delta made before holds the dead bits of then. Reused only while they are
        // these (Q173: the device's subtraction of a head's rows read the old bits, and some runs' findings differed).
        // SAFETY: `at` is a delta's descriptor in `one`'s block, which `one` keeps.
        let fresh = |at: usize| unsafe {
            let d = (*block.ptr::<Relation>(at)).dead.as_slice();
            std::ptr::eq(d, dead) || d == dead
        };
        // Its own dead bits, kept in its block (decision 0478 step 5d): the delta reads them there.
        let own = one.rel().own_dead();
        let own = !own.is_empty() && std::ptr::eq(own, dead);
        let k = match &*kills {
            Some(k) if fresh(k.at) => k.clone(),
            _ => {
                let rel = if own { Relation::try_kills_own(block, &spec(self.k), one.rel())? } else { Relation::try_kills(block, &spec(self.k), one.rel(), dead)? };
                let at = block.offset_of(rel);
                let k = Arc::new(Kills { at, base: one.clone(), own_dead: own });
                // (the one before goes here, unless a reader still holds it)
                *kills = Some(k.clone());
                k
            }
        };
        Some(InBlock { facts: one.facts.clone(), at: one.facts.block.offset_of(one.rel), rows, dead: k.at, kills: Some(k), part: Some(one.clone()) })
    }

    /// `in_block`, never merging parts: `None` where the rows are in several parts that read as no one relation and
    /// are not merged yet. A reader that can do without the block (the CPU's probes) asks for this, so it makes no
    /// copy.
    pub fn in_one_block(&self) -> Option<InBlock> {
        let (one, rows) = self.one_unmerged().or_else(|| self.merged.get().map(|m| (m, self.rows() as u32)))?;
        Some(InBlock { facts: one.facts.clone(), at: one.facts.block.offset_of(one.rel), rows, dead: 0, kills: None, part: Some(one.clone()) })
    }

    /// The relation that holds its last rows, in its block, whatever its parts: where its sorted orders' directory
    /// is (decision 0478 step 5b). Its rows are that part's own, not the relation's.
    pub fn last_in_block(&self) -> Option<InBlock> {
        let p = self.parts().last()?;
        Some(p.in_block(p.len()))
    }

    /// The rows as one relation without a copy, and how many rows a reader binding it reads (ids from 0): the one
    /// part, or a continuation of the one segment (a fork, decision 0384 step 7: its base's rows read through it).
    fn one_unmerged(&self) -> Option<(&Part, u32)> {
        let tail = self.tail.as_ref().filter(|t| t.len() > 0);
        if self.segs.is_empty() {
            // One part (or an empty tail from row 0).
            return match (tail, &self.tail) {
                (Some(t), _) => Some((t, t.len())),
                (None, Some(t)) if self.from == 0 => Some((t, 0)),
                _ => None,
            };
        }
        // Segments and the tail that read as one relation (decision 0384 steps 7 and 8: each a continuation of the
        // one before, the first a relation of its own, as a fork's and a fork of a fork's are): the last, read down its
        // chain. A frozen relation's prefix too: every reader of a bound relation reads the rows it is bound with
        // (`InBlock::rows`), and its indexes are of them (`find_index`'s rows).
        chain_of(self.segs.iter().map(|(f, p)| (*f, p)).chain(tail.map(|t| (self.from, t))))
            .filter(|end| end.reads() as usize == self.rows())
            .map(|end| (end, end.reads()))
    }

    /// The rows as one relation, and how many rows a reader binding it reads: `one_unmerged`, or every part merged
    /// into one (made once and kept).
    fn one(&self) -> Option<(&Part, u32)> {
        if let Some(one) = self.one_unmerged() {
            return Some(one);
        }
        if self.segs.is_empty() {
            return None;
        }
        let (segs, tail) = (&self.segs[..], &self.tail);
        // Parts that do not, read as one through links (decision 0353 step 6): each part's rows where they lie.
        if let Some(chain) = self.linked.get_or_init(|| self.links()) {
            return Some((chain, self.rows() as u32));
        }
        // Parts in more blocks than a reader binds: merged (a part appended after rows of another relation,
        // `append_part`, counted there).
        Some((self.merged.get_or_init(|| {
                const COPIED: rerail_prof::copies::Site = rerail_prof::copies::Site::new(9, "6", "a relation's parts merged into one for the device (Rows::in_block)");
                COPIED.time(|| {
                    let parts: Vec<&Part> = segs.iter().map(|(_, p)| p).chain(tail.iter()).collect();
                    let rows: usize = parts.iter().map(|p| p.len() as usize).sum();
                    let m = merged_counts();
                    m.0[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    m.0[1].fetch_add(rows * self.k * 8, std::sync::atomic::Ordering::Relaxed);
                    crate::code_rule_dsl::logic::profile::decision("rows of many parts merged into one", || format!("{} parts, {rows} rows of {} columns ({} KB copied)", parts.len(), self.k, (rows * self.k * 8) >> 10));
                    let mut one = Part::new(self.life, self.k, rows);
                    for p in parts {
                        one.copy_from(p, 0, p.len());
                    }
                    (one, 8 * self.k * rows)
                })
            }), self.rows() as u32))
    }

    /// Its parts read as one relation, nothing copied (decision 0353 step 6): a link in the last part's block for each
    /// part there (`Part::link`), the first continuing the parts before them when they are one relation of another
    /// block (a far base: a kept relation, this run's rows after it). `None` for parts in more blocks, a far part that
    /// is not a relation of its own from row 0, or a block with no room for a link's descriptor.
    fn links(&self) -> Option<Part> {
        let parts: Vec<(u32, &Part)> = self.segs.iter().map(|(f, p)| (*f, p)).chain(self.tail.iter().map(|t| (self.from, t))).filter(|(_, p)| p.len() > 0).collect();
        let home = &parts.last()?.1.facts;
        let near = parts.iter().position(|(_, p)| Arc::ptr_eq(&p.facts, home))?;
        if parts[near..].iter().any(|(_, p)| !Arc::ptr_eq(&p.facts, home)) || near > 1 {
            return None;
        }
        let mut chain: Option<Part> = None;
        if near == 1 {
            let (first, far) = parts[0];
            if first != 0 || far.is_continuation() || far.rel().is_delta() {
                return None;
            }
            chain = Some(far.prefix());
        }
        for &(first, p) in &parts[near..] {
            chain = Some(Part::link(chain.as_ref(), first, p)?);
        }
        chain
    }

    /// `part`'s rows after these, not copied (decision 0325 step 4, a head's rows the device wrote in place): it
    /// becomes the tail, so the rows pushed after it, and the next rows the device writes (`tail_room`), go on in the
    /// same relation (decision 0384 step 3). The tail, if it has rows, becomes a segment first.
    pub fn append_part(&mut self, part: Part) {
        let n = part.len();
        if n == 0 {
            return;
        }
        let k = part.rel().rows.ncols as usize;
        debug_assert!(self.k == 0 || self.k == k, "a part of {k} columns after rows of {}", self.k);
        debug_assert!(!part.capped(), "a shared prefix appended as a tail");
        self.k = k;
        self.merged.take();
        self.linked.take();
        *self.kills.get_mut().unwrap_or_else(|e| e.into_inner()) = None;
        // The device writes past rows in their tail (`tail_room`, which opens one past segments), so a relation of its
        // own comes only after none.
        if self.rows() > 0 {
            crate::code_rule_dsl::logic::profile::decision("parts appended after rows of another relation (merged when bound)", || format!("{n} rows after {}", self.rows()));
        }
        if let Some(t) = self.tail.take().filter(|t| t.len() > 0) {
            let next = self.from + t.len();
            self.segs.push((self.from, t));
            self.from = next;
        }
        self.tail = Some(part);
    }

    /// Room for `n` rows past the tail's, for a writer other than the host (the device, decision 0384 step 3): the
    /// tail as it lies with those rows reserved, and the place of the first. The writer writes rows `at..at + n` of
    /// it; `claim_tail` then makes them rows, or `unclaimed` reads them. `None` when there is no tail, or its block
    /// has no room for them (the tail is never moved for a writer that is not its own), or its room is lent already
    /// (it would be the same rows).
    pub fn tail_room(&self, n: usize) -> Option<Lent> {
        match &self.tail {
            Some(t) => t.room(n),
            // Past segments and no tail yet (a relation started from frozen rows another goes on from, decision 0384
            // steps 7 and 8): a tail opened for them, a continuation of the segments' relation (or, where its block has
            // no room, a relation holding every row), which the claim makes the tail.
            None if !self.segs.is_empty() => {
                let t = self.opened(n);
                t.room(n).or_else(|| Some(t.moved_room(t.len(), n)))
            }
            None => None,
        }
    }

    /// `tail_room`, when the tail's block has no room: the tail moved into a relation with room for them
    /// (`Part::moved_room`), which the claim makes the tail. `None` with no tail that has rows, or one lent already.
    pub fn tail_moved(&self, n: usize) -> Option<Lent> {
        let t = self.tail.as_ref().filter(|t| (t.len() > 0 || t.is_continuation()) && t.lent.load(std::sync::atomic::Ordering::Acquire) == 0)?;
        Some(t.moved_room(t.len(), n))
    }

    /// Rows `at..at + n` of `tail`, written by the writer `tail_room` gave its room to: now its rows.
    ///
    /// # Safety
    /// The writer has finished, and wrote every value of those rows.
    pub unsafe fn claim_tail(&mut self, lent: &Lent, n: u32) {
        // A tail opened for the writer (`tail_room` past segments): it is the tail now.
        if self.tail.is_none() {
            let t = &lent.tail;
            let end = self.chain();
            assert!(end.is_some_and(|e| t.continues_of(e)) || (t.absorbed == self.from && t.len() == lent.at), "rows written into a tail opened past other rows");
            self.tail = Some(t.clone());
            self.absorbed();
        }
        let t = self.tail.as_ref().expect("the tail written in place");
        // Rows lent in a relation the tail was moved into (its block had no room): that relation is the tail now, its
        // rows the tail's (moved when the room was lent, none pushed since: asserted below).
        if let Some(from) = &lent.from && !Arc::ptr_eq(&t.held, &lent.tail.held) {
            assert!(Arc::ptr_eq(&t.held, &from.held), "rows written into a relation moved from another tail");
            assert_eq!(t.len() + lent.tail.absorbed, lent.at, "rows pushed into a tail after it was moved for another writer");
            self.tail = Some(lent.tail.clone());
            self.absorbed();
        }
        let t = self.tail.as_ref().expect("the tail written in place");
        let at = lent.at;
        assert!(Arc::ptr_eq(&t.held, &lent.tail.held), "rows written into a tail the relation no longer has");
        assert_eq!(t.len(), at, "rows pushed into a tail while another writer wrote past them");
        assert!(at + n <= t.rel().rows.reserved(), "{n} rows written at {at} into room for {}", t.rel().rows.reserved());
        self.merged.take();
        self.linked.take();
        *self.kills.get_mut().unwrap_or_else(|e| e.into_inner()) = None;
        // SAFETY: within the rows reserved (asserted); the caller says they are written.
        unsafe { t.rel().rows.claim_reserved(n) };
    }

    /// Row `at + i`'s values of `tail`, written past its rows by the writer `tail_room` gave its room to, not claimed:
    /// rows a reader takes into its own batch (a wave's derived rows beside them, `push_fresh`), whose room the
    /// relation's next pushes reuse.
    ///
    /// # Safety
    /// The writer has finished, and wrote every value of the row, which lies in the room reserved.
    pub unsafe fn unclaimed(lent: &Lent, i: u32, k: usize) -> Row {
        let (tail, at) = (&lent.tail, lent.at);
        debug_assert!(at + i < tail.rel().rows.reserved(), "a row past the room reserved");
        Row::from_fn(k, |c| tail.cols[c].get(at + i))
    }

    /// Where structures derived from its rows go (`BlockVec`): its tail's block, else its newest segment's.
    pub fn home(&self) -> Option<Home> {
        if let Some(t) = &self.tail {
            return Some(Home { facts: t.facts.clone(), life: t.life, grow: t.capacity_bytes() });
        }
        let (_, p) = self.segs.last()?;
        Some(Home { facts: p.facts.clone(), life: p.life, grow: 0 })
    }

    /// Whether `facts` is its home's block (`home`), read with no reference counted.
    pub fn home_is(&self, facts: &Arc<Facts>) -> bool {
        self.tail.as_ref().map(|t| &t.facts).or_else(|| self.segs.last().map(|(_, p)| &p.facts)).is_none_or(|f| Arc::ptr_eq(f, facts))
    }

    /// Its relations, oldest first: the frozen segments, then the tail if it has rows. A reader that keeps them
    /// (`append_part`) holds the rows where they lie.
    pub fn parts(&self) -> impl Iterator<Item = &Part> {
        self.segs.iter().map(|(_, p)| p).chain(self.tail.iter().filter(|t| t.len() > 0))
    }

    /// Rows `lo..hi`, part by part in id order: each part, the rows of it (its own row numbers `from..to`), and the
    /// id of row `from`. A sequential reader finds each part once, not each row's (rows gathered from many relations,
    /// `append_part`, lie in many parts).
    pub fn each_run(&self, lo: u32, hi: u32, mut f: impl FnMut(&Part, u32, u32, u32)) {
        let mut id = lo;
        while id < hi {
            let (p, i) = self.part_of(id);
            let n = (p.len() - i).min(hi - id);
            f(p, i, i + n, id);
            id += n;
        }
    }

    /// Values a row.
    pub fn width(&self) -> usize {
        self.k
    }

    /// These rows with the tail made a segment (moved), and no segments merged, as `freeze` merges them past
    /// `MAX_SEGMENTS`: rows gathered from many relations (`append_part`), read where they lie.
    pub fn sealed(mut self) -> Rows {
        if let Some(tail) = self.tail.take().filter(|t| t.len() > 0) {
            let n = tail.len();
            self.segs.push((self.from, tail));
            self.from += n;
        }
        self
    }

    /// Whether these rows share a relation with `other` (a relation started from it: its segments, or its writer).
    pub fn shares(&self, other: &Rows) -> bool {
        self.parts().any(|a| other.parts().any(|b| std::ptr::eq(a.rel, b.rel)))
    }

    /// Row `id`'s `k` values.
    // Hot: a read written once (decision 0310) is inlined into its reader, across crates too.
    #[inline(always)]
    pub fn row(&self, id: u32, k: usize) -> Row {
        if k == 0 {
            return Row::of(&[]);
        }
        let (p, i) = self.part_of(id);
        Row::from_fn(k, |c| p.cols[c].get(i))
    }

    /// Row `id`'s value in column `c`: one read where it lies.
    // Hot: a read written once (decision 0310) is inlined into its reader, across crates too.
    #[inline(always)]
    pub fn get(&self, id: u32, c: usize) -> i64 {
        let (p, i) = self.part_of(id);
        p.cols[c].get(i)
    }

    /// The relation holding row `id`, and the row's place in it.
    // Hot: a read written once (decision 0310) is inlined into its reader, across crates too.
    #[inline(always)]
    fn part_of(&self, id: u32) -> (&Part, u32) {
        if id >= self.from {
            let Some(tail) = self.tail.as_ref() else {
                panic!("row {id} of a relation of {} rows ({} values a row, the tail from {}, {} segments): past its rows", self.rows(), self.k, self.from, self.segs.len())
            };
            return (tail, id - self.from);
        }
        let (first, p) = match &self.segs[..] {
            [only] => only,
            segs => &segs[segs.partition_point(|(first, _)| *first <= id) - 1],
        };
        (p, id - first)
    }

}

/// The last of `parts` (each its first row's id and the part), when they read as one relation (decision 0384 steps
/// 7 and 8): the last a continuation of the one before, down to the first, a relation of its own from row 0, each
/// part's first id its own rows' first. The last then reads every row by id, down its chain (`rel_get`).
fn chain_of<'a>(parts: impl DoubleEndedIterator<Item = (u32, &'a Part)>) -> Option<&'a Part> {
    let mut parts = parts.rev().peekable();
    let &(_, last) = parts.peek()?;
    let mut want = Some(last);
    for (first, p) in parts {
        let w = want?;
        let start = if w.is_continuation() { w.rel().base_rows } else { 0 };
        if !Arc::ptr_eq(&w.held, &p.held) || w.len() != p.len() || first != start {
            return None;
        }
        want = w.base.as_ref();
    }
    want.is_none().then_some(last)
}

/// The columns a row holds on the stack; a wider row's values are on the heap.
const INLINE: usize = 16;

/// One row's values, read out of the rows that hold it: on the stack (up to `INLINE`, `k` of them written; a read
/// returns it in its caller's slot), a wider one on the heap. It reads as a slice.
#[derive(Clone)]
pub enum Row {
    Inline { k: u8, v: [std::mem::MaybeUninit<i64>; INLINE] },
    Wide(Box<[i64]>),
}

impl Row {
    /// A row of `vals`.
    #[inline]
    pub fn of(vals: &[i64]) -> Row {
        Row::from_fn(vals.len(), |c| vals[c])
    }

    /// A row of `k` values, each `value(c)`.
    #[inline]
    pub fn from_fn(k: usize, mut value: impl FnMut(usize) -> i64) -> Row {
        if k > INLINE {
            return Row::Wide((0..k).map(value).collect());
        }
        let mut v = [std::mem::MaybeUninit::uninit(); INLINE];
        for (c, d) in v.iter_mut().enumerate().take(k) {
            d.write(value(c));
        }
        Row::Inline { k: k as u8, v }
    }
}

impl std::ops::Deref for Row {
    type Target = [i64];
    #[inline]
    fn deref(&self) -> &[i64] {
        match self {
            // SAFETY: the first `k` values were written (`from_fn`).
            Row::Inline { k, v } => unsafe { std::slice::from_raw_parts(v.as_ptr().cast(), *k as usize) },
            Row::Wide(v) => v,
        }
    }
}

impl PartialEq for Row {
    #[inline]
    fn eq(&self, other: &Row) -> bool {
        **self == **other
    }
}

impl PartialEq<[i64]> for Row {
    #[inline]
    fn eq(&self, other: &[i64]) -> bool {
        **self == *other
    }
}

impl PartialEq<&[i64]> for Row {
    #[inline]
    fn eq(&self, other: &&[i64]) -> bool {
        **self == **other
    }
}

impl std::fmt::Debug for Row {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (**self).fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decision 0353 step 6, row 8: a wave's writers write their rows past a relation's rows at once, each a span
    /// at a time, in its tail's room and past its block into regions of their own; the rows laid out in place
    /// (`lay_out`) become the relation's rows, past those it held, the tail claimed and the regions its tail after
    /// (no row copied), whatever order the writers' claims came in.
    #[test]
    fn a_waves_writers_write_in_place_and_their_rows_are_laid_out() {
        let k = 3;
        for held in [0usize, 700] {
            let mut rows = Rows::new(k);
            for i in 0..held as i64 {
                rows.extend_from_slice(&[i, -i, 7]);
            }
            let room = WaveRoom::open(&rows, k, false);
            // Four writers, each its spans; enough rows that a block's room may not hold them all.
            let per = 3 * WAVE_SPAN as usize + 17;
            let spans: Vec<Vec<(u8, u32)>> = std::thread::scope(|s| {
                let room = &room;
                let hs: Vec<_> = (0..4i64).map(|w| s.spawn(move || {
                    let mut mine = Vec::new();
                    for i in 0..per {
                        if i % WAVE_SPAN as usize == 0 {
                            mine.push(room.claim(WAVE_SPAN));
                        }
                        let (r, row) = mine[i / WAVE_SPAN as usize];
                        // SAFETY: this writer's span.
                        unsafe { room.put(r, row + (i % WAVE_SPAN as usize) as u32, &[w, i as i64, 1000 + w]) };
                    }
                    mine
                })).collect();
                hs.into_iter().map(|h| h.join().unwrap()).collect()
            });
            // Laid out writer by writer, each in its order: the rows past every span's unwritten rest.
            let mut src = Vec::new();
            for mine in &spans {
                for i in 0..per {
                    let (r, row) = mine[i / WAVE_SPAN as usize];
                    src.push(room.global(r, row + (i % WAVE_SPAN as usize) as u32));
                }
            }
            assert!(room.slots() as usize >= src.len());
            // SAFETY: the writers are done, and wrote every slot `src` names.
            unsafe {
                room.lay_out(&src, None);
                room.finish(&mut rows, src.len() as u32);
            }
            assert_eq!(rows.rows(), held + 4 * per, "held {held}: every row once");
            let got = rows.to_vec(held, held + 4 * per, k);
            let want: Vec<i64> = (0..4i64).flat_map(|w| (0..per as i64).flat_map(move |i| [w, i, 1000 + w])).collect();
            assert_eq!(got, want, "held {held}: in the order laid out");
            assert_eq!(rows.to_vec(0, held, k), (0..held as i64).flat_map(|i| [i, -i, 7]).collect::<Vec<_>>(), "held {held}: the rows before kept");
        }
    }

    /// `permute`: slot `t` takes the row `src[t]` held; the rest keep the other rows; each moved along its cycle.
    #[test]
    fn rows_are_laid_out_by_their_cycles() {
        let n = 50u32;
        let cells: Vec<std::cell::Cell<i64>> = (0..n).map(|i| std::cell::Cell::new(i64::from(i) * 10)).collect();
        let mut side: Vec<u32> = (0..n).collect();
        let src: Vec<u32> = (0..n).rev().step_by(3).collect();
        permute(n as usize, &src, |q| Row::of(&[cells[q as usize].get()]), |q, row| cells[q as usize].set(row[0]), Some(&mut side));
        for (t, &s) in src.iter().enumerate() {
            assert_eq!((cells[t].get(), side[t]), (i64::from(s) * 10, s), "slot {t}");
        }
        let mut rest: Vec<i64> = cells[src.len()..].iter().map(std::cell::Cell::get).collect();
        rest.sort_unstable();
        let want: Vec<i64> = (0..n).filter(|q| !src.contains(q)).map(|q| i64::from(q) * 10).collect();
        assert_eq!(rest, want, "the other rows kept");
    }

    thread_local! {
        /// The allocator's net counter, for these tests: a test moves it as its "work" allocates.
        static NET: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
    }
    thread_local! {
        /// The allocator's peak, for these tests: the net at its last reset, and the most above it since.
        static PEAK_AT: std::cell::Cell<(i64, i64)> = const { std::cell::Cell::new((0, 0)) };
    }
    fn net() -> i64 {
        NET.with(std::cell::Cell::get)
    }
    fn alloc(bytes: i64) {
        NET.with(|n| n.set(n.get() + bytes));
        PEAK_AT.with(|p| {
            let (base, most) = p.get();
            p.set((base, most.max(net() - base)));
        });
    }
    fn peak() -> i64 {
        PEAK_AT.with(std::cell::Cell::get).1
    }
    fn reset() {
        PEAK_AT.with(|p| p.set((net(), 0)));
    }

    /// Decision 0469: a chunk's heap is its scopes' own allocation at their most: what it freed before its end counts
    /// at its most, a job of another chunk run inside it is that chunk's, and the same work measures the same.
    #[test]
    fn a_chunks_heap_is_its_scopes_own_peaks() {
        install_heap_net(net);
        install_heap_peak(peak, reset);
        let other = Arc::new(Chunk::default());
        let run = || {
            in_chunk_measured(|| {
                alloc(100 << 20);
                alloc(-60 << 20);
                {
                    // Another chunk's job, run while this one waited: 50 MB at its most, all freed.
                    let _job = enter_chunk(Some(other.clone()));
                    alloc(50 << 20);
                    alloc(-50 << 20);
                }
                // From its 40 MB, to 120 MB: its most, past the 100 MB before the job.
                alloc(80 << 20);
                alloc(-120 << 20);
            })
            .1
        };
        let (w, again) = (run(), run());
        assert_eq!(w.heap, 120 << 20, "{} MB", w.heap >> 20);
        assert_eq!(w, again, "the same work measures the same");
        assert_eq!(w.measure(), w.bytes + w.heap);
        assert_eq!(other.peaks.load(std::sync::atomic::Ordering::Relaxed), 2 * (50 << 20), "the job's peak is its own chunk's");
    }

    /// Q203 item 3: a chunk is sized by what its own work held, not the process's footprint: what its scope
    /// allocated and kept is its, a job of another chunk run on its thread meanwhile is that chunk's, and what the
    /// thread allocates outside any chunk (a run's one-time allocations) is no chunk's.
    #[test]
    fn a_chunk_holds_what_its_own_work_allocated() {
        install_heap_net(net);
        alloc(500 << 20);
        let other = Arc::new(Chunk::default());
        let held = in_chunk(|| {
            alloc(100 << 20);
            {
                // Another chunk's job, run while this one waited.
                let _job = enter_chunk(Some(other.clone()));
                alloc(50 << 20);
            }
            alloc(30 << 20);
            alloc(-10 << 20);
            chunk_held_most().expect("in a chunk")
        });
        // Its most when asked: its own 100 + 30 - 10 MB, the other chunk's 50 MB not, the 500 MB before it not.
        assert_eq!(held, 120 << 20, "{} MB", held >> 20);
        assert_eq!(other.held_most(), 50 << 20, "the other chunk's job is its own");
        assert!(current_chunk().is_none(), "the run's chunk is back");
    }

    /// An empty kept relation keeps its width: one started from it holds the rows pushed to it (it dropped them when
    /// the width came from the rows, none for no rows).
    #[test]
    fn a_relation_started_from_an_empty_one_keeps_its_rows() {
        let kept = Rows::frozen(3, Vec::new(), 0);
        let mut next = kept.shared(0);
        next.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        assert_eq!(next.rows(), 2);
        assert_eq!(*next.row(1, 3), [4, 5, 6]);
        assert_eq!(next.get(0, 2), 3);
    }

    /// A frozen relation's rows are shared by one started from it, which pushes its own after them; freezing that
    /// one moves its tail into a segment, and both read every row by id.
    #[test]
    fn started_rows_share_the_kept_segments_and_freeze_their_own() {
        let k = 2;
        let kept = Rows::frozen(k, (0..20).collect(), 10);
        let mut next = kept.shared(10);
        assert!(next.shares(&kept));
        next.extend_from_slice(&[100, 101]);
        next.extend_from_slice(&[102, 103]);
        assert_eq!(*next.row(3, k), [6, 7]);
        assert_eq!(*next.row(11, k), [102, 103]);
        assert_eq!(next.to_vec(9, 12, k), vec![18, 19, 100, 101, 102, 103]);
        let closed = next.freeze(12);
        assert!(closed.shares(&kept), "the kept segment is not copied at close");
        assert_eq!(*closed.row(10, k), [100, 101]);
        assert_eq!(closed.to_vec(10, 12, k), [100, 101, 102, 103]);
        // Edits add small segments; past the most held, the newest are merged and every row stays where it was.
        let mut rows = closed;
        let mut n = 12;
        for e in 0..20 {
            let mut s = rows.shared(n);
            s.extend_from_slice(&[1000 + e, 2000 + e]);
            n += 1;
            rows = s.freeze(n);
        }
        assert!(rows.segs.len() <= MAX_SEGMENTS);
        assert!(rows.shares(&kept), "the first segment is kept, not merged");
        assert_eq!(*rows.row(5, k), [10, 11]);
        assert_eq!(*rows.row(12, k), [1000, 2000]);
        assert_eq!(*rows.row(31, k), [1019, 2019]);
        assert_eq!(rows.len(), n * k);
    }

    /// A copy of rows still being written shares their tail, not a copy of it: it reads the rows the tail held when
    /// it was made, the writer's later rows are not among them, and its own pushes start a tail of its own.
    #[test]
    fn a_copy_shares_the_tail_and_sees_only_its_rows() {
        let k = 2;
        let mut rows = Rows::new(k);
        rows.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let mut copy = rows.clone();
        assert!(std::ptr::eq(copy.segs.last().expect("the tail, shared").1.rel, rows.tail.as_ref().expect("a tail").rel), "the tail was copied");
        rows.extend_from_slice(&[7, 8]);
        assert_eq!((rows.rows(), copy.rows()), (4, 3));
        assert_eq!(copy.to_vec(0, 3, k), [1, 2, 3, 4, 5, 6]);
        copy.extend_from_slice(&[9, 10]);
        assert_eq!(*copy.row(3, k), [9, 10]);
        assert_eq!(*rows.row(3, k), [7, 8]);
        // `shared` takes the tail as it is now, as `clone` does.
        let next = rows.shared(4);
        assert_eq!((next.rows(), next.to_vec(0, 4, k)), (4, vec![1, 2, 3, 4, 5, 6, 7, 8]));
        // A shared prefix is not bound as its relation: one relation of its rows is made for a reader that binds one.
        let one = copy.in_block().expect("rows");
        assert_eq!(one.rows, 4);
    }

    /// A relation kept with no rows and started again (the cross-file part maintained from its frames in a new
    /// process) takes the rows pushed after: its width is the relation's, not read off rows it does not have.
    #[test]
    fn rows_started_from_an_empty_kept_relation_take_their_pushes() {
        let k = 3;
        let kept = Rows::frozen(k, Vec::new(), 0);
        let mut next = kept.shared(0);
        next.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        assert_eq!(next.len(), 2 * k);
        assert_eq!(*next.row(1, k), [4, 5, 6]);
        assert_eq!(*next.freeze(2).row(0, k), [1, 2, 3]);
    }

    /// Q173: a relation's dead rows change with no row added (a level's rows killed, then brought back). The delta a
    /// reader binds holds the dead bits it is given now, not those of an earlier bind.
    #[test]
    fn the_dead_rows_bound_are_those_given_now() {
        let mut rows = Rows::new(1);
        // copy: not block data: a test's rows
        rows.extend_from_slice(&[1, 2, 3, 4]);
        let bits = |b: &InBlock| unsafe { (*b.facts.block.ptr::<Relation>(b.dead)).dead.as_slice().to_vec() };
        let first = rows.in_block_dead(&[0b0101]).expect("a delta");
        assert_eq!(bits(&first), [0b0101]);
        assert_eq!(rows.in_block_dead(&[0b0101]).expect("a delta").dead, first.dead, "the same bits: the same delta");
        let all = rows.in_block_dead(&[0b1111]).expect("a delta");
        assert_eq!(bits(&all), [0b1111], "every row killed");
        let back = rows.in_block_dead(&[0b0010]).expect("a delta");
        assert_eq!(bits(&back), [0b0010], "rows brought back");
        assert_eq!(bits(&first), [0b0101], "a delta a reader holds is kept as it was");
    }

    /// Q178: a delta whose dead bits are no longer the relation's goes back to its block once no reader holds it, so a
    /// relation whose dead rows keep changing takes the room of two deltas, not one a change.
    #[test]
    fn a_dead_rows_delta_replaced_goes_back_to_its_block() {
        let mut rows = Rows::new(1);
        rows.extend_from_slice(&(0..256).collect::<Vec<i64>>());
        let words = |i: u64| [i, !i, i << 1, i >> 1];
        let mut at = std::collections::BTreeSet::new();
        for i in 1..200 {
            let b = rows.in_block_dead(&words(i)).expect("a delta");
            assert_eq!(unsafe { (*b.facts.block.ptr::<Relation>(b.dead)).dead.as_slice() }, words(i));
            at.insert(b.dead);
        }
        // (a few, not two: the block is shared, and another test's relation may take a chunk given back)
        assert!(at.len() <= 10, "{} deltas' room taken for 199 changes: the ones replaced were not given back", at.len());
    }

    /// Decision 0384 step 4: a relation's derived array lies in its relation's block, grows and is cloned there, and
    /// gives its chunk back to the block's free list when dropped.
    #[test]
    fn an_index_array_lies_in_its_relations_block() {
        let mut rows = Rows::new(1);
        // copy: not block data: a test's rows
        rows.extend_from_slice(&[1, 2, 3]);
        let home = rows.home().expect("a relation with rows has a home");
        let mut v = BlockVec::from_iter_in(Some(&home), 0..100u32);
        assert!(Arc::ptr_eq(v.facts().expect("a chunk"), &home.facts), "in the relation's block");
        let cap = v.capacity();
        v.extend(100..cap as u32 + 50);
        assert!(Arc::ptr_eq(v.facts().expect("a chunk"), &home.facts), "grown in the same block");
        assert!(v.iter().copied().eq(0..cap as u32 + 50));
        let c = v.clone();
        assert!(Arc::ptr_eq(c.facts().expect("a chunk"), &home.facts) && *c == *v);
        let free = home.facts.block.free_bytes();
        drop((v, c));
        assert!(home.facts.block.free_bytes() > free, "its chunks back on the free list");
        // Without a home: a run's open block.
        let lone = BlockVec::from_iter_in(None, [7i64, 8]);
        assert_eq!(*lone, [7, 8]);
        assert!(BlockVec::<u8>::new().facts().is_none(), "empty holds no chunk");
    }

    /// Decision 0487 R4 at a relation's arrays: with a GPU read of a `BlockVec`'s items queued, a CPU write of them
    /// fails and a push past them (in place) passes; with a GPU write queued, a read fails. Once the fence completes
    /// both pass.
    #[test]
    fn a_block_vec_under_a_queued_gpu_use_of_its_chunk_fails() {
        let fails = |f: &mut dyn FnMut()| std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err();
        let mut v = BlockVec::from_iter_in(None, 0..64u64);
        // Room past its length, so the push below writes in place.
        v.reserve(8);
        let (facts, at, len) = (v.facts().expect("a chunk").clone(), v.offset(), v.len() * 8);
        let block = &facts.block;
        let r = rerail_blocks::ledger::Fence::new();
        r.reads_chunk(block, at, len);
        assert_eq!(v[3], 3, "reads pass under a GPU read");
        assert!(fails(&mut || v[3] = 7), "a write by index under a GPU read");
        assert!(!fails(&mut || v.push(64)), "a push past what the GPU reads");
        r.complete();
        v[3] = 7;
        let w = rerail_blocks::ledger::Fence::new();
        w.writes_chunk(block, at, len);
        assert!(fails(&mut || { std::hint::black_box(v[3]); }), "a read under a GPU write");
        w.complete();
        assert_eq!(v[3], 7);
    }

    /// A tail opened past a relation of its own that cannot be continued in its block (a full one) holds every row it
    /// reads (`Part::moved_all`): it absorbed them all, so the segment goes and its ids are from 0. With only a
    /// continuation's base counted as absorbed, a relation of its own kept its segment too, and the rows pushed after
    /// were read at no id (a second copy of the old rows at theirs).
    #[test]
    fn a_tail_moved_past_its_rows_holds_every_row_once() {
        let k = 2;
        let mut rows = Rows::new(k);
        // copy: not block data: a test's rows
        rows.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let rows = rows.sealed();
        let end = rows.chain().expect("one relation").clone();
        let mut next = Rows { segs: rows.segs.clone(), from: rows.from, tail: Some(end.moved_all(4)), k, life: rows.life, ..Rows::default() };
        next.absorbed();
        assert_eq!((next.parts().count(), next.from), (1, 0), "the segment absorbed");
        // copy: not block data: a test's rows
        next.extend_from_slice(&[7, 8]);
        for (i, want) in [[1i64, 2], [3, 4], [5, 6], [7, 8]].iter().enumerate() {
            assert_eq!(&*next.row(i as u32, k), &want[..], "row {i}, read where it lies");
        }
    }

    /// Decision 0353 step 6: a relation of parts in two blocks (a kept relation, then rows of a run's block after
    /// it, `append_part`) is bound as one relation through links, nothing merged: its first link continues the kept
    /// relation in the other block (a far base), each run part's rows read where they lie, and every id reads its row.
    #[test]
    fn parts_in_two_blocks_are_bound_through_links() {
        let k = 2;
        let n = 3_000u32;
        let row = |i: u32| [i64::from(i) * 7, i64::from(i) % 13];
        let mut kept = Rows::living(k, Life::Kept);
        // copy: not block data: a test's rows
        kept.extend_from_slice(&(0..1_000).flat_map(row).collect::<Vec<i64>>());
        let mut rows = kept.freeze(1_000);
        for (lo, hi) in [(1_000, 1_800), (1_800, n)] {
            let mut run = Rows::living(k, Life::Run);
            // copy: not block data: a test's rows
            run.extend_from_slice(&(lo..hi).flat_map(row).collect::<Vec<i64>>());
            rows.append_part(run.last_in_block().unwrap().part.unwrap());
        }
        let (copied, _) = merged();
        let b = rows.in_block().expect("bound");
        assert_eq!(merged().0, copied, "nothing merged");
        assert_eq!(b.rows, n, "every row");
        let far = b.far().expect("its chain reaches the kept relation's block");
        assert!(!Arc::ptr_eq(&far, &b.facts), "two blocks");
        let r = b.relation();
        assert!(r.continues() && r.base().is_some_and(|base| base.far_base()), "a link of a link of the far base");
        for id in (0..n).step_by(7).chain([999, 1_000, 1_799, 1_800, n - 1]) {
            for c in 0..k {
                assert_eq!(r.get_far(Some(&far.block), id, c), row(id)[c], "row {id} column {c}, down its chain");
            }
        }
        assert!(b.own_index(&[0]).is_none(), "the host indexes it by part");
    }

    /// Decision 0384 step 7: a fork (a second relation started from frozen rows whose writer the first took) goes on
    /// in a continuation of their relation, in its block: bound as one relation (nothing merged), its base's rows read
    /// through it, frozen and started from again; moved out of a full block it takes its base's rows and is a relation
    /// of its own.
    #[test]
    fn a_fork_goes_on_in_a_continuation_of_its_rows() {
        let k = 2;
        let mut rows = Rows::new(k);
        // copy: not block data: a test's rows
        rows.extend_from_slice(&[1, 2, 3, 4]);
        let kept = rows.freeze(2);
        let mut first = kept.shared(2);
        let mut fork = kept.shared(2);
        // copy: not block data: a test's rows
        first.extend_from_slice(&[5, 6]);
        // copy: not block data: a test's rows
        fork.extend_from_slice(&[7, 8, 9, 10]);
        assert!(fork.tail.as_ref().is_some_and(|t| t.rel().continues()), "the second goes on in a continuation");
        let merged = super::merged().0;
        let b = fork.in_block().expect("bound");
        assert_eq!(b.rows, 4, "the base's 2 and its own 2");
        let rel = b.relation();
        let got: Vec<i64> = (0..4).flat_map(|id| [rel.get(id, 0), rel.get(id, 1)]).collect();
        assert_eq!(got, [1, 2, 3, 4, 7, 8, 9, 10], "each row where it lies, by the read written once");
        assert_eq!(super::merged().0, merged, "nothing merged");
        assert_eq!(first.in_block().expect("bound").rows, 3);
        // Frozen, then started from again: its writer the next one's, still one relation.
        let kept2 = fork.freeze(4);
        assert_eq!(kept2.in_block().expect("bound").rows, 4);
        let mut next = kept2.shared(4);
        // copy: not block data: a test's rows
        next.extend_from_slice(&[11, 12]);
        let b = next.in_block().expect("bound");
        assert_eq!((b.rows, b.relation().get(4, 1), b.relation().get(1, 0)), (5, 12, 3));
        assert_eq!(super::merged().0, merged, "nothing merged");
        // Moved for another writer (as from a full block): its base's rows go with it, and it is one relation.
        let lent = next.tail.as_ref().expect("a tail").moved_room(next.tail.as_ref().expect("a tail").len(), 1);
        assert_eq!(lent.at, 5, "every row it reads, then the room");
        // SAFETY: row 5 is the loan's, reserved, and nothing else writes it.
        unsafe {
            lent.tail().rel().rows.put(5, 0, 13);
            lent.tail().rel().rows.put(5, 1, 14);
            next.claim_tail(&lent, 1);
        }
        drop(lent);
        assert_eq!((next.parts().count(), next.from), (1, 0), "the base's segment absorbed");
        // copy: not block data: a test's rows
        let all: Vec<Vec<i64>> = (0..6).map(|i| next.row(i, k).to_vec()).collect();
        assert_eq!(all, [[1, 2], [3, 4], [7, 8], [9, 10], [11, 12], [13, 14]].map(|r| r.to_vec()));
        assert!(!next.in_block().expect("bound").relation().continues());
    }

    /// Decision 0384 step 8: forks of forks stay one relation. A fork frozen and forked again (while another holds its
    /// writer) goes on in a continuation of the continuation, read down the chain; so does a copy's first row past
    /// the rows it shares (`clone`), and the device's rows written past a fork that has no tail yet (`tail_room`).
    /// Past `MAX_SEGMENTS` levels the newest two go on in one continuation; moved out of a full block, the chain's
    /// rows go with it. Nothing is merged when bound.
    #[test]
    fn forks_of_forks_stay_one_relation() {
        let k = 1;
        let merged = || super::merged().0;
        let before = merged();
        let read = |r: &Rows| -> Vec<i64> {
            let b = r.in_block().expect("bound");
            assert_eq!(b.rows as usize, r.rows(), "bound whole");
            (0..b.rows).map(|id| b.relation().get(id, 0)).collect()
        };
        let mut start = Rows::new(k);
        // copy: not block data: a test's rows
        start.extend_from_slice(&[0, 1]);
        let mut kept = start.freeze(2);
        let mut want = vec![0, 1];
        // Ten levels: at each, one run takes the writer and writes a row nobody else reads; a fork goes on.
        for level in 0..10 {
            let n = kept.rows();
            let mut taker = kept.shared(n);
            // copy: not block data: a test's rows
            taker.extend_from_slice(&[-1]);
            let mut fork = kept.shared(n);
            // copy: not block data: a test's rows
            fork.extend_from_slice(&[10 * level + 2, 10 * level + 3]);
            want.extend([10 * level + 2, 10 * level + 3]);
            assert!(fork.tail.as_ref().is_some_and(|t| t.rel().continues()), "level {level}: a continuation");
            assert_eq!(read(&fork), want, "level {level}: read down the chain");
            assert_eq!(read(&taker)[n], -1, "the taker's row its own");
            kept = fork.freeze(want.len());
            assert!(kept.segs.len() <= MAX_SEGMENTS, "level {level}: a chain of at most MAX_SEGMENTS");
            assert_eq!(read(&kept), want);
        }
        // A copy's first row past the rows it shares: a continuation too.
        let mut copy = kept.clone();
        let taker = kept.shared(kept.rows());
        // copy: not block data: a test's rows
        copy.extend_from_slice(&[99]);
        assert!(copy.tail.as_ref().is_some_and(|t| t.rel().continues()));
        assert_eq!(read(&copy).last(), Some(&99));
        drop(taker);
        // The device's rows past a fork with no tail yet: a tail opened for them, claimed.
        let mut fork = kept.shared(kept.rows());
        let _first = kept.shared(kept.rows());
        let mut fork2 = kept.shared(kept.rows());
        assert!(fork.tail.is_none() || fork2.tail.is_none());
        if fork.tail.is_some() {
            std::mem::swap(&mut fork, &mut fork2);
        }
        let lent = fork.tail_room(2).expect("room past the fork");
        // SAFETY: rows `at..at + 2` are the loan's, reserved, and nothing else writes them.
        unsafe {
            lent.tail().rel().rows.put(lent.at, 0, 500);
            lent.tail().rel().rows.put(lent.at + 1, 0, 501);
            fork.claim_tail(&lent, 2);
        }
        drop(lent);
        let mut w = want.clone();
        w.extend([500, 501]);
        assert_eq!(read(&fork), w);
        // Moved for another writer, as from a full block: the whole chain's rows go with it, one relation.
        let t = fork.tail.as_ref().expect("a tail");
        let lent = t.moved_room(t.len(), 1);
        // SAFETY: as above.
        unsafe {
            lent.tail().rel().rows.put(lent.at, 0, 502);
            fork.claim_tail(&lent, 1);
        }
        drop(lent);
        w.push(502);
        assert_eq!((fork.parts().count(), fork.from), (1, 0), "the chain absorbed");
        assert_eq!(read(&fork), w);
        assert_eq!(merged(), before, "nothing merged");
    }

    /// A chunk takes its size class, the next power of two, from a block with a free list: an open block whose room
    /// passes the check (`bytes` and the margin) can still hold no chunk of the class. "an open block with room holds
    /// 2144940 bytes" (`python/references.rerail` on all): its room was 3 to 4 MiB. Such a chunk goes to a new open block.
    #[test]
    fn a_chunk_the_open_blocks_room_cannot_take_goes_to_a_new_block() {
        // A run of its own: the open blocks are the run's (`Open`), and another test's would take their room.
        let _run = crate::code_rule_dsl::logic::profile::test_run();
        let room = |f: &Arc<Facts>| f.block.reserved().saturating_sub(f.block.used());
        let probe = BlockVec::<u8>::from_iter_in(None, [0u8]);
        let open = probe.facts().expect("a chunk").clone();
        // Fillers of whole classes bring the room to 3.5 MiB: past 2 MiB and the margin, short of 4 MiB.
        let target = 7usize << 19;
        let mut fillers = Vec::new();
        for bit in (6..40).rev() {
            while room(&open) >= target + (1 << bit) {
                let v = BlockVec::<u8>::with_capacity_in(None, 1 << bit);
                assert!(Arc::ptr_eq(v.facts().expect("a chunk"), &open), "a filler in the open block");
                fillers.push(v);
            }
        }
        let need = (1usize << 21) + 4;
        let left = room(&open);
        assert!(left >= need + MARGIN && left < need.next_power_of_two(), "room {left} for {need} bytes");
        let v = BlockVec::<u32>::from_iter_in(None, 0..(need / 4) as u32);
        assert!(!Arc::ptr_eq(v.facts().expect("a chunk"), &open), "in a new open block");
        assert!(v.iter().copied().eq(0..(need / 4) as u32));
    }

    /// Decision 0384: a tail whose block has no room for the device's rows is moved into a relation with room
    /// (`tail_moved`, and `Lent::moved` for a next lot), and the claims make that its tail: one relation, nothing
    /// merged, every row by its id.
    #[test]
    fn a_tail_moved_for_another_writers_rows_stays_one_relation() {
        let k = 2;
        let mut rows = Rows::new(k);
        // copy: not block data: a test's rows
        rows.extend_from_slice(&[1, 2, 3, 4]);
        let write = |lent: &Lent, vals: &[[i64; 2]]| {
            for (i, v) in vals.iter().enumerate() {
                // SAFETY: rows `at..at + n` are the loan's, reserved, and nothing else writes them.
                unsafe {
                    lent.tail().rel().rows.put(lent.at + i as u32, 0, v[0]);
                    lent.tail().rel().rows.put(lent.at + i as u32, 1, v[1]);
                }
            }
        };
        let merged = super::merged().0;
        let old = rows.tail.as_ref().expect("a tail").clone();
        // As if its block were full: moved.
        let lent = rows.tail_moved(2).expect("a tail with rows");
        assert_eq!(lent.at, 2);
        assert!(!Arc::ptr_eq(&lent.tail().held, &old.held), "another relation");
        write(&lent, &[[10, 20], [11, 21]]);
        // A next lot past a full block: the rows so far, the first lot's too, moved again.
        let next = lent.moved(2, 1);
        assert_eq!(next.at, 4);
        write(&next, &[[12, 22]]);
        // SAFETY: the rows are written.
        unsafe {
            rows.claim_tail(&lent, 2);
            rows.claim_tail(&next, 1);
        }
        assert!(Arc::ptr_eq(&rows.tail.as_ref().expect("a tail").held, &next.tail().held), "the last moved relation is the tail");
        drop((lent, next, old));
        assert_eq!(rows.parts().count(), 1, "one relation");
        assert_eq!(rows.in_block().expect("bound whole").rows, 5);
        assert_eq!(super::merged().0, merged, "nothing merged");
        // copy: not block data: a test's rows
        let all: Vec<Vec<i64>> = (0..5).map(|i| rows.row(i, k).to_vec()).collect();
        assert_eq!(all, [[1, 2], [3, 4], [10, 20], [11, 21], [12, 22]].map(|r| r.to_vec()));
        // Pushes go on in it.
        // copy: not block data: a test's rows
        rows.extend_from_slice(&[5, 6]);
        assert_eq!(rows.parts().count(), 1);
        assert_eq!(*rows.row(5, k), [5, 6]);
    }

    /// A part another writer filled, after rows pushed and before rows pushed again: every row read by its id, the
    /// part's not copied.
    #[test]
    fn a_part_written_elsewhere_becomes_the_tail() {
        let k = 2;
        let mut rows = Rows::new(k);
        // copy: not block data: a test's rows
        rows.extend_from_slice(&[1, 2, 3, 4]);
        let part = Part::for_writes(k, 3);
        // SAFETY: rows 0..3 are reserved, and nothing else writes this part.
        unsafe {
            for i in 0..3u32 {
                part.rel().rows.put(i, 0, 10 + i64::from(i));
                part.rel().rows.put(i, 1, 20 + i64::from(i));
            }
            part.written(3);
        }
        let place = part.rel() as *const _;
        rows.append_part(part);
        // copy: not block data: a test's rows
        rows.extend_from_slice(&[5, 6]);
        assert_eq!(rows.rows(), 6);
        let all: Vec<Vec<i64>> = (0..6).map(|i| rows.row(i, k).to_vec()).collect();
        assert_eq!(all, [[1, 2], [3, 4], [10, 20], [11, 21], [12, 22], [5, 6]].map(|r| r.to_vec()));
        // Decision 0384 step 3: the part is the tail, and the rows pushed after it go on in it.
        assert!(rows.tail.as_ref().is_some_and(|t| std::ptr::eq(t.rel, place)), "the part itself is the tail");
        assert_eq!(rows.segs.len(), 1, "the rows before it, one segment");
        let frozen = rows.freeze(6);
        assert_eq!(*frozen.row(4, k), [12, 22]);
    }

    /// Decision 0384 step 3: rows another writer writes into the tail's room are the relation's own, claimed in
    /// place: it stays one relation, bound whole with nothing merged. One loan a tail at a time, given back when
    /// dropped; rows written and not claimed read as written, and the room is the next pushes'.
    #[test]
    fn rows_written_into_the_tails_room_keep_one_relation() {
        let k = 2;
        let mut rows = Rows::new(k);
        // copy: not block data: a test's rows
        rows.extend_from_slice(&[1, 2, 3, 4]);
        let write = |lent: &Lent, vals: &[[i64; 2]]| {
            for (i, v) in vals.iter().enumerate() {
                // SAFETY: rows `at..at + n` are the loan's, reserved, and nothing else writes them.
                unsafe {
                    lent.tail().rel().rows.put(lent.at + i as u32, 0, v[0]);
                    lent.tail().rel().rows.put(lent.at + i as u32, 1, v[1]);
                }
            }
        };
        let merged = super::merged().0;
        let lent = rows.tail_room(3).expect("room in the tail's block");
        assert_eq!(lent.at, 2);
        assert!(rows.tail_room(1).is_none(), "one loan a tail at a time");
        write(&lent, &[[10, 20], [11, 21]]);
        // A next writer's room goes on after the first's rows; both claimed in order.
        let next = lent.after(2, 1).expect("room after the first loan");
        assert_eq!(next.at, 4);
        write(&next, &[[12, 22]]);
        // SAFETY: the rows are written.
        unsafe {
            rows.claim_tail(&lent, 2);
            rows.claim_tail(&next, 1);
        }
        drop((lent, next));
        assert_eq!(rows.parts().count(), 1, "one relation");
        let b = rows.in_block().expect("bound whole");
        assert_eq!(b.rows, 5);
        assert_eq!(super::merged().0, merged, "nothing merged");
        // copy: not block data: a test's rows
        let all: Vec<Vec<i64>> = (0..5).map(|i| rows.row(i, k).to_vec()).collect();
        assert_eq!(all, [[1, 2], [3, 4], [10, 20], [11, 21], [12, 22]].map(|r| r.to_vec()));
        // Lent again once given back; read, not claimed, and the room the next push's.
        let lent = rows.tail_room(1).expect("the loan given back");
        write(&lent, &[[7, 8]]);
        // SAFETY: the row is written.
        assert_eq!(*unsafe { Rows::unclaimed(&lent, 0, k) }, [7, 8]);
        drop(lent);
        // copy: not block data: a test's rows
        rows.extend_from_slice(&[5, 6]);
        assert_eq!((rows.rows(), &*rows.row(5, k)), (6, &[5, 6][..]));
    }

    /// Decision 0384 step 2: a relation started from frozen rows takes their relation's writer and goes on in it, one
    /// relation (bound with nothing merged), while the frozen rows read only their own. A second started from them while
    /// the first holds the writer goes on in a continuation of their relation (step 7); a taker dropped with nothing
    /// written gives the writer back.
    #[test]
    fn a_relation_started_from_frozen_rows_goes_on_in_their_relation() {
        let k = 1;
        let mut first = Rows::new(k);
        // copy: not block data: a test's rows
        first.extend_from_slice(&[1, 2, 3]);
        let kept = first.freeze(3);
        let mut next = kept.shared(3);
        assert!(next.segs.is_empty() && next.from == 0, "the writer taken as the tail, from row 0");
        // copy: not block data: a test's rows
        next.extend_from_slice(&[4]);
        assert_eq!((kept.rows(), next.rows()), (3, 4), "the frozen rows read their own");
        assert!(next.shares(&kept));
        let merged = super::merged().0;
        assert_eq!(next.in_block().expect("bound").rows, 4);
        assert_eq!(kept.in_block().expect("bound").rows, 3, "a prefix bound as its rows");
        assert_eq!(super::merged().0, merged, "nothing merged");
        // The writer is the first's: a second's first row opens a continuation of the kept rows' relation (step 7).
        let mut other = kept.shared(3);
        assert!(other.segs.len() == 1 && other.tail.is_none());
        // copy: not block data: a test's rows
        other.extend_from_slice(&[9]);
        assert!(other.tail.as_ref().is_some_and(|t| t.rel().continues()));
        drop(other);
        // Frozen with a row written, its relation's writer is free at its 4 rows, not the kept 3.
        let kept2 = next.freeze(4);
        assert!(kept.shared(3).tail.is_none(), "rows were written past the first prefix");
        let taker = kept2.shared(4);
        assert!(taker.tail.is_some());
        // Dropped with nothing written: given back.
        drop(taker);
        let again = kept2.shared(4);
        assert_eq!(again.tail.as_ref().map(Part::len), Some(4), "the writer given back");
        assert_eq!((0..4).map(|i| again.get(i, 0)).collect::<Vec<_>>(), [1, 2, 3, 4]);
    }

    /// An empty kept relation has its width: one started from it holds what it pushes. Its width was inferred from its
    /// rows, so 0, and the started one dropped every row and read past its segments (a session's maintained cross-file
    /// stratum over an empty kept relation: "a row of the tail").
    #[test]
    fn a_relation_started_from_an_empty_kept_one_holds_its_rows() {
        let k = 3;
        let kept = Rows::frozen(k, Vec::new(), 0);
        let mut next = kept.shared(0);
        next.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        assert_eq!(next.len(), 6);
        assert_eq!(*next.row(1, k), [4, 5, 6]);
        assert_eq!(next.get(0, 2), 3);
    }

    /// The block a relation's rows lie in.
    fn block_of(rows: &Rows) -> Arc<Facts> {
        rows.in_block().expect("rows").facts
    }

    fn some_rows() -> Rows {
        let mut r = Rows::new(2);
        // copy: not block data (a test's literal rows)
        r.extend_from_slice(&[1, 2, 3, 4]);
        r
    }

    /// A chunk's relations go in blocks of its own (Q58): none is shared with the run's or another chunk's, and
    /// each is given back once the chunk and its rows end, whatever outlives other chunks.
    #[test]
    fn a_chunks_relations_go_in_its_own_blocks_and_go_with_it() {
        let outside = some_rows();
        let (a, b) = std::thread::scope(|s| {
            let a = s.spawn(|| in_chunk(|| { let r = some_rows(); (Arc::downgrade(&block_of(&r)), Arc::as_ptr(&block_of(&r)) as usize) }));
            let b = s.spawn(|| in_chunk(|| { let r = some_rows(); (Arc::downgrade(&block_of(&r)), Arc::as_ptr(&block_of(&r)) as usize) }));
            (a.join().unwrap(), b.join().unwrap())
        });
        let run = Arc::as_ptr(&block_of(&outside)) as usize;
        assert_ne!(a.1, run, "a chunk's relation is not in the run's block");
        assert_ne!(a.1, b.1, "two chunks at once share no block");
        assert!(a.0.upgrade().is_none() && b.0.upgrade().is_none(), "a chunk's blocks go with it");
        // A relation the chunk keeps past its end keeps its block, and only that.
        let kept = in_chunk(some_rows);
        assert_eq!(*kept.row(1, 2), [3, 4]);
        assert_ne!(Arc::as_ptr(&block_of(&kept)) as usize, run);
    }

    /// What a chunk wrote (decision 0435 R6) is its blocks' bytes, whatever was given back before it ended, and the
    /// same on every run of the same chunk: of the rows it keeps past it, the held part.
    #[test]
    fn a_chunk_measures_what_its_blocks_wrote_the_same_every_time() {
        let fill = |n: i64| {
            let mut r = Rows::new(2);
            // copy: not block data (a test's literal rows)
            r.extend_from_slice(&(0..2 * n).collect::<Vec<i64>>());
            r
        };
        let run = || in_chunk_measured(|| {
            drop(fill(200_000));
            fill(1_000)
        });
        let ((kept, w), (_, again)) = (run(), run());
        assert_eq!(w, again, "the same chunk measures the same");
        assert!(w.bytes >= 2 * 200_000 * 8, "the dropped relation's rows count: {w:?}");
        assert!(w.held > 0 && w.held <= w.bytes, "{w:?}");
        // The dropped relation lay in the block the kept one holds: held is the kept part, not the block's extent.
        assert!(w.held < 2 * 200_000 * 8, "the dropped relation's rows are not held: {w:?}");
        assert_eq!(*kept.row(999, 2), [1_998, 1_999]);
        let (_, none) = in_chunk_measured(|| ());
        assert_eq!(none, Written::default());
    }

    /// A tail grows in its block in place, past what a block was sized at before (64 MB), never moved (decision 0445
    /// R1): its block reserves address space, and its pages are taken as the rows are written.
    #[test]
    fn a_tail_grows_in_its_block_past_the_old_size_without_moving() {
        if !RESERVES_ADDRESS_SPACE {
            return;
        }
        // Its own shared object (decision 0442): the moves counted are this test's, none of a test beside it.
        let _own = crate::code_rule_dsl::logic::profile::within(crate::code_rule_dsl::logic::profile::Shared::new());
        in_chunk(|| {
            let before = moved().0;
            let mut r = Rows::new(1);
            // copy: not block data (a test's literal rows)
            let lot: Vec<i64> = (0..1 << 20).collect();
            r.extend_from_slice(&lot);
            let facts = block_of(&r);
            for _ in 1..10 {
                r.extend_from_slice(&lot);
            }
            assert!(Arc::ptr_eq(&facts, &block_of(&r)), "the same block");
            assert!(facts.block.used() > 64 << 20, "past the old size: {} bytes", facts.block.used());
            assert_eq!(moved().0, before, "no relation moved");
            assert_eq!(r.get(9 << 20 | 5, 0), 5);
        });
    }

    /// A chunk holds the rows it handed on, never a device run's own relations, which outlive it only while device
    /// work in flight holds them (Q314: on Metal, a different number of them every run).
    #[test]
    fn a_chunk_holds_what_it_handed_on_not_a_device_runs_own() {
        let live = [(Life::Run, 10 << 20, 4 << 20), (Life::Kept, 2 << 20, 2 << 20), (Life::Device, 64 << 20, 64 << 20), (Life::Source, 1 << 20, 0)];
        assert_eq!(held_of(&live), 6 << 20);
        assert_eq!(held_of(&live[..2]), held_of(&live), "the device's blocks, alive or not, change nothing");
    }

    /// Outside a run the open blocks are the machine's (decision 0442, `open_blocks`): a session's relations made
    /// between its runs share one block, as a run's do, not a block of their own each (16 GiB of address space each
    /// where blocks reserve it: a session's 1,000 one-document batches' relations reserved all of it).
    #[test]
    fn relations_made_outside_a_run_share_the_machines_block() {
        assert!(crate::code_rule_dsl::logic::profile::run_own().is_none(), "outside a run");
        let made: Vec<Rows> = (0..1000i64)
            .map(|i| {
                let mut r = Rows::new(2);
                // copy: not block data (a test's literal rows)
                r.extend_from_slice(&[i, i + 1, i + 2, i + 3]);
                r
            })
            .collect();
        let first = block_of(&made[0]);
        assert!(made.iter().all(|r| Arc::ptr_eq(&block_of(r), &first)), "one block");
        assert_eq!(*made[999].row(1, 2), [1001, 1002]);
    }

    /// A relation a chunk drops gives its chunks back to the chunk's block (decision 0342): the next relation of
    /// its size takes them, so the block's used bytes grow by the descriptors alone.
    #[test]
    fn a_dropped_relations_chunks_are_taken_by_the_next() {
        in_chunk(|| {
            let fill = |base: i64| {
                let mut r = Rows::new(3);
                // copy: not block data (a test's literal rows)
                r.extend_from_slice(&(0..90_000).map(|v| base + v).collect::<Vec<i64>>());
                r
            };
            let first = fill(0);
            let facts = block_of(&first);
            drop(first);
            let used = facts.block.used();
            let next = fill(1_000_000);
            assert!(Arc::ptr_eq(&facts, &block_of(&next)), "the same block");
            assert!(facts.block.used() - used < 64 << 10, "grew {} bytes: the chunks were taken again", facts.block.used() - used);
            assert_eq!(*next.row(29_999, 3), [1_089_997, 1_089_998, 1_089_999]);
        });
    }

    /// A reader of the rows in place (`InBlock`, the device's) holds the relation: its chunks are not given back
    /// while it reads them.
    #[test]
    fn a_reader_in_place_keeps_the_relation() {
        in_chunk(|| {
            let mut r = Rows::new(2);
            // copy: not block data (a test's literal rows)
            r.extend_from_slice(&[5, 6, 7, 8]);
            let reading = r.in_block().unwrap();
            let free = reading.facts.block.free_bytes();
            drop(r);
            assert_eq!(reading.facts.block.free_bytes(), free, "nothing given back while read");
            let facts = reading.facts.clone();
            drop(reading);
            assert!(facts.block.free_bytes() > free, "given back after");
        });
    }
}
