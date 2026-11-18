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
//! written once (`rerail_clif::relation`: the host's are `rerail_blocks::generated_reads`, the kernels' the same
//! reads in MSL). They live in shared FACTS blocks (`Facts`): a block holds many relations, leased from the GPU pool
//! (pinned, a no-copy buffer of the device's), or reserved anonymously where there is none. A block never moves;
//! a tail whose block is full moves to another, one copy of its rows (as a vector's growth copies them). Every row
//! lies whole in one segment. Segments past `MAX_SEGMENTS` are merged from the newest (the small ones a session's
//! edits add), so a row's segment is found among a few.

use std::sync::{Arc, Mutex};

use rerail_blocks::block::{Block, kind};
use rerail_blocks::relation::{Relation, Spec};
use rerail_blocks::rows::ColRead;

/// The most segments a frozen relation holds; past them its newest are merged.
const MAX_SEGMENTS: usize = 8;

/// A FACTS block's size: what relations fill before the next is leased (a relation larger takes one its size).
const FACTS_BYTES: usize = 64 << 20;

impl Life {
    /// The size of its blocks (a relation larger takes one its size): a batch's sources take less, as they die
    /// with it. At 64 MB, their blocks' room held about 60 MB more at the peak of posthog/hogql's CPU binding.
    fn block_bytes(self) -> usize {
        match self {
            Life::Run => FACTS_BYTES,
            Life::Source => FACTS_BYTES / 4,
            // Each alone in a block of its size (`try_new`).
            Life::Device => 0,
        }
    }
}

/// Room a block keeps past a new relation's: what its other relations' tails may take meanwhile.
const MARGIN: usize = 1 << 20;

/// A block of relations: shared by every segment and tail in it, given back (to the pool) with the last.
pub struct Facts {
    pub block: Block,
    /// Held while an index is added to one of its relations (`Relation::try_reserve_index`, decision 0325 step 5).
    pub ix: Mutex<()>,
}

impl Facts {
    fn new(block: Block) -> Facts {
        Facts { block, ix: Mutex::new(()) }
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
    /// A device run's own relations (decision 0325 step 5): the views it makes from a chunk, a fixpoint's totals and
    /// deltas, an upload. Leased as the device's join buffers are, within its budget (a refusal runs the stratum on
    /// the CPU), and apart from the runs' relations, which they would keep alive.
    Device,
}

/// The blocks new relations go in outside a chunk, while they have room: one a `Life`.
static OPEN: [Mutex<Option<Arc<Facts>>>; 3] = [Mutex::new(None), Mutex::new(None), Mutex::new(None)];

/// A chunk's blocks (Q58): the one its run's new relations go in while it has room, and every one it made, to count
/// those that outlive it. A block lives while any relation in it does, so one shared by every chunk at once was kept
/// whole by a few rows of each, and the run's footprint (the line its chunk sizes are cut by) counted it whole.
pub struct Chunk {
    open: Mutex<Option<Arc<Facts>>>,
    made: Mutex<Vec<std::sync::Weak<Facts>>>,
    /// Whether the device may read its blocks: as its run is bound (`placement::block_device`, the engine's), so a
    /// CPU run's blocks are anonymous (Q58).
    device: rerail_blocks::alloc::Device,
}

thread_local! {
    /// The chunk this thread's run is evaluating (`in_chunk`): carried to its jobs (`setfix::carried`).
    pub static CHUNK: std::cell::RefCell<Option<Arc<Chunk>>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` as a chunk's evaluation: the run's relations it and its jobs make (`Life::Run`) go in blocks of the
/// chunk's own, given back with the last of its rows (decision 0289: a chunk is a block), which the device may read
/// as `device` says (the run's binding). What outlives the chunk keeps only its own block; the profile counts those
/// (`a chunk's blocks outlived it`).
pub fn in_chunk<R>(device: rerail_blocks::alloc::Device, f: impl FnOnce() -> R) -> R {
    struct Leave(Option<Arc<Chunk>>, Arc<Chunk>);
    impl Drop for Leave {
        fn drop(&mut self) {
            CHUNK.with(|c| *c.borrow_mut() = self.0.take());
            self.1.open.lock().unwrap_or_else(|e| e.into_inner()).take();
            let made = std::mem::take(&mut *self.1.made.lock().unwrap_or_else(|e| e.into_inner()));
            let (n, live) = (made.len(), made.iter().filter_map(std::sync::Weak::upgrade).map(|f| f.block.used()).collect::<Vec<_>>());
            if !live.is_empty() {
                let bytes: usize = live.iter().sum();
                crate::code_rule_dsl::logic::profile::decision("a chunk's blocks outlived it", || {
                    format!("{} of its {n} blocks, {} MB used in them, held by rows it handed on", live.len(), bytes >> 20)
                });
            }
        }
    }
    let chunk = Arc::new(Chunk { open: Mutex::new(None), made: Mutex::new(Vec::new()), device });
    let _leave = Leave(CHUNK.with(|c| c.borrow_mut().replace(chunk.clone())), chunk);
    f()
}

/// A block of at least `bytes` for relations, from the one block policy (decision 0342): the device may read it in
/// place as `device` says (a pool slab when the budget spares one, half of it left to the device's own buffers; a
/// chunk of a CPU run's never, Q58); its relations are made and dropped, so it keeps a free list.
fn lease(bytes: usize, life: Life, device: rerail_blocks::alloc::Device) -> Result<Block, String> {
    use rerail_blocks::alloc::{Need, Writes};
    rerail_blocks::alloc::block(&Need { kind: kind::FACTS, bytes: bytes.max(life.block_bytes()), writes: Writes::Churn, device })
}

/// The bytes a relation of `k` 8-byte columns and `rows` rows takes in its block: its descriptor and directories,
/// and its chunks as `Rows::try_reserve` makes them (quarter steps of doubling groups, each chunk aligned).
fn room_for(k: usize, rows: usize) -> usize {
    let min_log = rerail_blocks::rows::MIN_LOG;
    let align = |bytes: usize| if bytes >= rerail_blocks::block::PAGE { rerail_blocks::block::PAGE } else { 64 };
    let mut chunks = 0;
    let mut bytes = 0;
    while rerail_blocks::rows::capacity(chunks, min_log) < rows as u64 {
        let b = rerail_blocks::rows::chunk_rows(chunks as usize, min_log) as usize * 8;
        bytes += k * (b + align(b));
        chunks += 1;
    }
    bytes + (k + 1) * size_of::<rerail_blocks::rows::ColDir>() + 4096
}

/// A block of the device's own of at least `bytes`: a relation alone in it. Without Metal, anonymous memory. Leased
/// as a device join's buffers are (bounded inside the join's budget, a refusal running it on the CPU), which the
/// block policy (decision 0342) has no need for yet: the one block made outside it.
fn lease_exact(bytes: usize, life: Life) -> Result<Block, String> {
    #[cfg(target_os = "macos")]
    if life == Life::Device {
        return rerail_accel::gpu::shared::lease_block_for_join(kind::FACTS, bytes);
    }
    let _ = life;
    Ok(Block::anon(kind::FACTS, bytes.max(4 * rerail_blocks::block::PAGE)))
}

/// A relation of `k` columns holding `vals` (row by row), alone in a block of anonymous memory, as a run's block is
/// when the device's spare budget refuses it a slab: bound on the device by a wrap of its used extent
/// (`Block::gpu_buffer`), which what is added to the block after that wrap lies past.
#[cfg(any(test, feature = "test-hooks"))]
pub fn anon_in_block(k: usize, vals: &[i64]) -> InBlock {
    let rows = vals.len() / k;
    let facts = Arc::new(Facts::new(Block::anon(kind::FACTS, 2 * room_for(k, rows) + 8 * MARGIN)));
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
    InBlock { facts, at, rows: rows as u32, dead: 0, part: None }
}

/// A relation of `k` values a row, 8 bytes each, read in their order.
fn spec(k: usize) -> Spec {
    Spec { widths: vec![8; k], order: Vec::new(), indexes: Vec::new(), min_log: rerail_blocks::rows::MIN_LOG }
}

/// One relation of a block: a segment or a tail, shared by its holders; its chunks given back to the block's free
/// list when the last lets go (decision 0342).
#[derive(Clone)]
pub struct Part {
    held: Arc<Held>,
    life: Life,
    /// Each column resolved once (its directory, which never moves while the relation is in its block): a read pays
    /// the chunk arithmetic and two loads, not the descriptor's walk too.
    cols: Arc<[ColRead]>,
}

/// A relation in its block, given back with its last holder.
struct Held {
    facts: Arc<Facts>,
    rel: *const Relation,
}

impl Drop for Held {
    fn drop(&mut self) {
        // SAFETY: the relation lies in `facts`' block, and this was its last holder.
        unsafe { (*self.rel).release(&self.facts.block) };
    }
}

// SAFETY: the relation lies in `facts`' block, which the part keeps; its writer is the one `Rows` holding it.
unsafe impl Send for Held {}
unsafe impl Sync for Held {}
// SAFETY: as `Held`; its columns' directories lie in the same block and never move while it is there.
unsafe impl Send for Part {}
unsafe impl Sync for Part {}

impl Part {
    /// An empty relation of `k` columns with room for `rows` rows, in the open block or a new one.
    fn new(life: Life, k: usize, rows: usize) -> Part {
        Part::try_new(life, k, rows).unwrap_or_else(|e| panic!("a relation of {k} columns and {rows} rows: {e}"))
    }

    /// `new`, or why no block could be had (a device block past the device's budget).
    fn try_new(life: Life, k: usize, rows: usize) -> Result<Part, String> {
        Part::try_new_with(life, k, rows, 0)
    }

    /// `try_new`, a block of its own (a device relation's) with `extra` bytes past the relation's room.
    fn try_new_with(life: Life, k: usize, rows: usize, extra: usize) -> Result<Part, String> {
        let spec = spec(k);
        let need = room_for(k, rows);
        // A device relation is written once at its size, and lives as the device's buffers did: alone in a block of
        // its size (leased within the device's budget), given back with it. A run's tail grows: its block has room for
        // it to double.
        let alone = life == Life::Device;
        // A run's relations made in a chunk go in the chunk's blocks (Q58).
        let chunk = if life == Life::Run { CHUNK.with(|c| c.borrow().clone()) } else { None };
        let mut open = chunk.as_ref().map_or(&OPEN[life as usize], |c| &c.open).lock().unwrap_or_else(|e| e.into_inner());
        for fresh in [false, true] {
            let facts = match &*open {
                _ if alone => Arc::new(Facts::new(lease_exact(need + extra, life)?)),
                Some(f) if !fresh && f.block.reserved().saturating_sub(f.block.used()) >= need + MARGIN => f.clone(),
                _ => {
                    // Outside a chunk the device may read it (a run's relations made between chunks, a batch's sources).
                    let device = chunk.as_ref().map_or(rerail_blocks::alloc::Device::May, |c| c.device);
                    let f = Arc::new(Facts::new(lease(2 * need + MARGIN, life, device)?));
                    if let Some(c) = &chunk {
                        c.made.lock().unwrap_or_else(|e| e.into_inner()).push(Arc::downgrade(&f));
                    }
                    *open = Some(f.clone());
                    f
                }
            };
            if let Some(rel) = Relation::try_create(&facts.block, &spec, None)
                && rel.rows.try_reserve(&facts.block, rows as u64)
            {
                let cols = (0..k).map(|c| rel.rows.reader(c)).collect();
                let rel = std::ptr::from_ref(rel);
                return Ok(Part { held: Arc::new(Held { facts, rel }), life, cols });
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
        let cap = rows.next_power_of_two().max(1);
        let index = 4 * (cap + 1) + 4 * rows.max(1) + rerail_blocks::relation::ADDED_INDEXES * size_of::<rerail_blocks::relation::Index>() + 3 * 64;
        let part = Part::try_new_with(Life::Device, k, rows, index)?;
        // SAFETY: rows `0..rows` are reserved; the caller's device work writes every one before a read.
        unsafe { part.rel().rows.claim_reserved(rows as u32) };
        Ok(part)
    }

    /// Its relation's descriptor's offset in its block: what a reader's `rel_*` reads take.
    pub fn rel_at(&self) -> usize {
        self.held.facts.block.offset_of(self.held.rel)
    }

    #[inline(always)]
    pub fn rel(&self) -> &Relation {
        // SAFETY: the part keeps its block.
        unsafe { &*self.held.rel }
    }

    /// The block it lies in.
    pub fn facts(&self) -> &Arc<Facts> {
        &self.held.facts
    }

    #[inline]
    pub fn len(&self) -> u32 {
        self.rel().rows.len()
    }

    /// An empty relation of `k` columns in a run's block, with room for `rows` rows that a writer other than the
    /// host fills (decision 0325 step 4: the device writes a head's rows in place, through the reads written once,
    /// `rows_at`), then says how many it wrote (`written`).
    pub fn for_writes(k: usize, rows: usize) -> Part {
        Part::new(Life::Run, k, rows)
    }

    /// Its rows' descriptor's offset in its block: what a writer's `rows_at` takes.
    pub fn rows_at(&self) -> usize {
        self.held.facts.block.offset_of(&raw const self.rel().rows)
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
        if self.rel().rows.try_reserve(&self.held.facts.block, n) {
            return;
        }
        let k = self.rel().rows.ncols as usize;
        let mut moved = Part::new(self.life, k, 2 * n as usize);
        moved.copy_from(self, 0, self.len());
        *self = moved;
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
    /// Every part merged into one, made once for a reader that binds one relation (`in_block`).
    merged: std::sync::OnceLock<Part>,
    /// The dead rows' delta of that one relation (`in_block_dead`), its descriptor's offset; `None` when its block
    /// could not hold it.
    kills: std::sync::OnceLock<Option<usize>>,
}

/// Where a relation's rows lie for a reader that binds its block (the device, decision 0325): the block, kept while
/// this lives, and the relation's descriptor's offset in it.
pub struct InBlock {
    pub facts: Arc<Facts>,
    pub at: usize,
    pub rows: u32,
    /// The offset of a delta of it whose dead bits mark its dead rows (`Relation::try_kills`); 0 when none is.
    pub dead: usize,
    /// The relation read, held while this lives: its chunks are not given back meanwhile (decision 0342). `None`
    /// for one made alone in a block of its own (`anon_in_block`).
    pub part: Option<Part>,
}

impl Clone for Rows {
    /// The segments shared; the tail copied (it is written by one `Rows`).
    fn clone(&self) -> Rows {
        let tail = self.tail.as_ref().map(|t| {
            let mut c = Part::new(self.life, self.k, t.len() as usize);
            c.copy_from(t, 0, t.len());
            c
        });
        Rows { segs: self.segs.clone(), from: self.from, tail, k: self.k, hint: self.hint, life: self.life, merged: Default::default(), kills: Default::default() }
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
            None => self.tail = Some(Part::new(self.life, self.k, values / self.k)),
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
        self.kills.take();
        let n = rows.len() / self.k;
        let tail = self.tail.get_or_insert_with(|| Part::new(self.life, self.k, self.hint.max(n).max(16)));
        if n > 1 {
            tail.grow(u64::from(tail.len()) + n as u64);
        }
        for row in rows.chunks_exact(self.k) {
            tail.push(row);
        }
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
    pub fn freeze(mut self, n: usize) -> Rows {
        if let Some(tail) = self.tail.take().filter(|t| t.len() > 0) {
            self.segs.push((self.from, tail));
        }
        self.from = n as u32;
        while self.segs.len() > MAX_SEGMENTS {
            // The newest two, merged: a session's edits add small segments after one large one.
            let (_, b) = self.segs.pop().expect("two segments");
            let (first, a) = self.segs.pop().expect("two segments");
            let mut both = Part::new(self.life, self.k, (a.len() + b.len()) as usize);
            both.copy_from(&a, 0, a.len());
            both.copy_from(&b, 0, b.len());
            self.segs.push((first, both));
        }
        self
    }

    /// The `n` rows of a frozen relation, shared: its segments taken (and its tail copied, if it has one), with an
    /// empty tail from `n` on, for a relation that starts as them.
    pub fn shared(&self, n: usize) -> Rows {
        let mut segs = self.segs.clone();
        if let Some(t) = self.tail.as_ref().filter(|t| t.len() > 0) {
            let mut c = Part::new(self.life, self.k, t.len() as usize);
            c.copy_from(t, 0, t.len());
            segs.push((self.from, c));
        }
        Rows { segs, from: n as u32, tail: None, k: self.k, hint: 0, life: self.life, merged: Default::default(), kills: Default::default() }
    }

    /// The rows as one relation in one block, ids from 0: the one part, or every part merged into one (made once,
    /// and kept while these rows are). `None` for no rows.
    pub fn in_block(&self) -> Option<InBlock> {
        let one = self.one()?;
        Some(InBlock { facts: one.facts().clone(), at: one.rel_at(), rows: one.len(), dead: 0, part: Some(one.clone()) })
    }

    /// `in_block`, with the rows `dead` marks (bit `id`, as setfix keeps them) dead in it: a delta of the one
    /// relation in its block, no rows of its own, whose dead bits are these (made once, and kept while these rows
    /// are). `None` for no rows, or when the block cannot hold the delta.
    pub fn in_block_dead(&self, dead: &[u64]) -> Option<InBlock> {
        let one = self.one()?;
        let at = (*self.kills.get_or_init(|| {
            let block = &one.facts().block;
            Relation::try_kills(block, &spec(self.k), one.rel(), dead).map(|r| block.offset_of(r))
        }))?;
        Some(InBlock { facts: one.facts().clone(), at: one.rel_at(), rows: one.len(), dead: at, part: Some(one.clone()) })
    }

    /// The rows as one relation: the one part, or every part merged into one (made once and kept).
    fn one(&self) -> Option<&Part> {
        Some(match (&self.segs[..], &self.tail) {
            ([], None) => return None,
            ([], Some(t)) if self.from == 0 => t,
            ([(0, p)], None) => p,
            (segs, tail) => self.merged.get_or_init(|| {
                let parts: Vec<&Part> = segs.iter().map(|(_, p)| p).chain(tail.iter()).collect();
                let mut one = Part::new(self.life, self.k, parts.iter().map(|p| p.len() as usize).sum());
                for p in parts {
                    one.copy_from(p, 0, p.len());
                }
                one
            }),
        })
    }

    /// `part`'s rows after these, as a segment of their own: not copied (decision 0325 step 4, a head's rows the
    /// device wrote in place). The tail, if it has rows, becomes a segment first; the next row pushed starts a new one.
    pub fn append_part(&mut self, part: Part) {
        let n = part.len();
        if n == 0 {
            return;
        }
        let k = part.rel().rows.ncols as usize;
        debug_assert!(self.k == 0 || self.k == k, "a part of {k} columns after rows of {}", self.k);
        self.k = k;
        self.merged.take();
        self.kills.take();
        let at = match self.tail.take() {
            Some(t) if t.len() > 0 => {
                let next = self.from + t.len();
                self.segs.push((self.from, t));
                next
            }
            _ => self.from,
        };
        self.segs.push((at, part));
        self.from = at + n;
    }

    /// Whether these rows share a frozen segment with `other` (a relation started from it).
    pub fn shares(&self, other: &Rows) -> bool {
        self.segs.iter().any(|(_, a)| other.segs.iter().any(|(_, b)| std::ptr::eq(a.held.rel, b.held.rel)))
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

    /// A part another writer filled, after rows pushed and before rows pushed again: every row read by its id, the
    /// part's not copied.
    #[test]
    fn a_part_written_elsewhere_is_appended_as_a_segment() {
        let k = 2;
        let mut rows = Rows::new(k);
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
        rows.extend_from_slice(&[5, 6]);
        assert_eq!(rows.rows(), 6);
        let all: Vec<Vec<i64>> = (0..6).map(|i| rows.row(i, k).to_vec()).collect();
        assert_eq!(all, [[1, 2], [3, 4], [10, 20], [11, 21], [12, 22], [5, 6]].map(|r| r.to_vec()));
        assert!(rows.segs.iter().any(|(_, p)| std::ptr::eq(p.rel(), place)), "the part itself is a segment");
        let frozen = rows.freeze(6);
        assert_eq!(*frozen.row(4, k), [12, 22]);
    }

    /// The block a relation's rows lie in.
    fn block_of(rows: &Rows) -> Arc<Facts> {
        rows.in_block().expect("rows").facts
    }

    fn some_rows() -> Rows {
        let mut r = Rows::new(2);
        r.extend_from_slice(&[1, 2, 3, 4]);
        r
    }

    /// A chunk's relations go in blocks of its own (Q58): none is shared with the run's or another chunk's, and
    /// each is given back once the chunk and its rows end, whatever outlives other chunks.
    #[test]
    fn a_chunks_relations_go_in_its_own_blocks_and_go_with_it() {
        let outside = some_rows();
        let (a, b) = std::thread::scope(|s| {
            let a = s.spawn(|| in_chunk(rerail_blocks::alloc::Device::May, || { let r = some_rows(); (Arc::downgrade(&block_of(&r)), Arc::as_ptr(&block_of(&r)) as usize) }));
            let b = s.spawn(|| in_chunk(rerail_blocks::alloc::Device::May, || { let r = some_rows(); (Arc::downgrade(&block_of(&r)), Arc::as_ptr(&block_of(&r)) as usize) }));
            (a.join().unwrap(), b.join().unwrap())
        });
        let run = Arc::as_ptr(&block_of(&outside)) as usize;
        assert_ne!(a.1, run, "a chunk's relation is not in the run's block");
        assert_ne!(a.1, b.1, "two chunks at once share no block");
        assert!(a.0.upgrade().is_none() && b.0.upgrade().is_none(), "a chunk's blocks go with it");
        // A relation the chunk keeps past its end keeps its block, and only that.
        let kept = in_chunk(rerail_blocks::alloc::Device::May, some_rows);
        assert_eq!(*kept.row(1, 2), [3, 4]);
        assert_ne!(Arc::as_ptr(&block_of(&kept)) as usize, run);
    }

    /// A run bound to the CPU takes anonymous blocks (Q58): resident only where written, never a pinned buffer of
    /// the device's pool, which the run's footprint would count whole.
    #[test]
    fn a_cpu_run_takes_anonymous_blocks() {
        // `Device::No` is what a CPU run's binding asks (`placement::block_device`).
        let r = in_chunk(rerail_blocks::alloc::Device::No, some_rows);
        assert!(!block_of(&r).block.in_slab(), "a CPU run's block is anonymous");
    }

    /// A relation a chunk drops gives its chunks back to the chunk's block (decision 0342): the next relation of
    /// its size takes them, so the block's used bytes grow by the descriptors alone.
    #[test]
    fn a_dropped_relations_chunks_are_taken_by_the_next() {
        in_chunk(rerail_blocks::alloc::Device::May, || {
            let fill = |base: i64| {
                let mut r = Rows::new(3);
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
        in_chunk(rerail_blocks::alloc::Device::May, || {
            let mut r = Rows::new(2);
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
