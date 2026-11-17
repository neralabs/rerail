//! A relation's rows as segments (smoking gun 8, t34), each a `rerail_blocks` relation in a block the CPU and the
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

use super::runs::RowData;

/// The most segments a frozen relation holds; past them its newest are merged.
const MAX_SEGMENTS: usize = 8;

/// A FACTS block's size: what relations fill before the next is leased (a relation larger takes one its size).
const FACTS_BYTES: usize = 64 << 20;

/// Room a block keeps past a new relation's: what its other relations' tails may take meanwhile.
const MARGIN: usize = 1 << 20;

/// A block of relations: shared by every segment and tail in it, given back (to the pool) with the last.
pub struct Facts {
    pub block: Block,
}

/// The block new relations go in outside a chunk, while it has room.
static OPEN: Mutex<Option<Arc<Facts>>> = Mutex::new(None);

/// A chunk's blocks (Q58): the one its new relations go in while it has room, and every one it made, to count
/// those that outlive it. A block lives while any relation in it does, so one shared by every chunk at once was kept
/// whole by a few rows of each, and the run's footprint (the line its chunk sizes are cut by) counted it whole.
#[derive(Default)]
pub struct Chunk {
    open: Mutex<Option<Arc<Facts>>>,
    made: Mutex<Vec<std::sync::Weak<Facts>>>,
}

thread_local! {
    /// The chunk this thread's run is evaluating (`in_chunk`): carried to its jobs (`setfix::carried`).
    pub(super) static CHUNK: std::cell::RefCell<Option<Arc<Chunk>>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` as a chunk's evaluation: the relations it and its jobs make go in blocks of the chunk's own, given back
/// with the last of its rows (decision 0289: a chunk is a block). What outlives the chunk keeps only its own block;
/// the profile counts those (`a chunk's blocks outlived it`).
pub fn in_chunk<R>(f: impl FnOnce() -> R) -> R {
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
    let chunk = Arc::new(Chunk::default());
    let _leave = Leave(CHUNK.with(|c| c.borrow_mut().replace(chunk.clone())), chunk);
    f()
}

/// A block of at least `bytes` for relations, from the one block policy (decision 0342): a run bound to the device
/// may read it in place (a pool slab when one is spare), a CPU run's never (Q58); its relations are made and
/// dropped, so it keeps a free list.
fn lease(bytes: usize) -> Block {
    use rerail_blocks::alloc::{Need, Writes};
    let device = crate::code_rule_dsl::logic::plan::placement::block_device();
    rerail_blocks::alloc::block(&Need { kind: kind::FACTS, bytes: bytes.max(FACTS_BYTES), writes: Writes::Churn, device })
        .expect("a block a run may read on the host")
}

/// One relation of a block: a segment or a tail, shared by its holders; given back to the block's free list when
/// the last lets go (decision 0342).
#[derive(Clone)]
pub struct Part(Arc<Held>);

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

impl Part {
    /// An empty relation of `k` columns with room for `rows` rows, in the open block or a new one.
    fn new(k: usize, rows: usize) -> Part {
        let spec = Spec { widths: vec![8; k], order: Vec::new(), indexes: Vec::new(), min_log: rerail_blocks::rows::MIN_LOG };
        let need = rows * k * 8 + k * size_of::<rerail_blocks::rows::ColDir>() + 4096;
        let chunk = CHUNK.with(|c| c.borrow().clone());
        let mut open = chunk.as_ref().map_or(&OPEN, |c| &c.open).lock().unwrap_or_else(|e| e.into_inner());
        for fresh in [false, true] {
            let facts = match &*open {
                Some(f) if !fresh && f.block.reserved().saturating_sub(f.block.used()) >= need + MARGIN => f.clone(),
                _ => {
                    let f = Arc::new(Facts { block: lease(2 * need + MARGIN) });
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
                let rel = std::ptr::from_ref(rel);
                return Part(Arc::new(Held { facts, rel }));
            }
        }
        unreachable!("a new block of {} bytes holds a relation of {need}", 2 * need + MARGIN)
    }

    #[inline]
    pub fn rel(&self) -> &Relation {
        // SAFETY: the part keeps its block.
        unsafe { &*self.0.rel }
    }

    /// The block it lies in.
    pub fn facts(&self) -> &Arc<Facts> {
        &self.0.facts
    }

    #[inline]
    fn len(&self) -> u32 {
        self.rel().rows.len()
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
        if self.rel().rows.try_reserve(&self.0.facts.block, n) {
            return;
        }
        let k = self.rel().rows.ncols as usize;
        let mut moved = Part::new(k, 2 * n as usize);
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
    /// Every part merged into one, made once for a reader that binds one relation (`in_block`).
    merged: std::sync::OnceLock<Part>,
}

/// Where a relation's rows lie for a reader that binds its block (the device, decision 0325): the block, kept while
/// this lives, and the relation's descriptor's offset in it.
pub struct InBlock {
    pub facts: Arc<Facts>,
    pub at: usize,
    pub rows: u32,
    /// The relation read, held while this lives: its chunks are not given back meanwhile (decision 0342).
    pub part: Part,
}

impl Clone for Rows {
    /// The segments shared; the tail copied (it is written by one `Rows`).
    fn clone(&self) -> Rows {
        let tail = self.tail.as_ref().map(|t| {
            let mut c = Part::new(self.k, t.len() as usize);
            c.copy_from(t, 0, t.len());
            c
        });
        Rows { segs: self.segs.clone(), from: self.from, tail, k: self.k, hint: self.hint, merged: Default::default() }
    }
}

impl Rows {
    /// No rows, of `k` values each.
    pub(super) fn new(k: usize) -> Rows {
        Rows { k, ..Rows::default() }
    }

    /// `n` rows of `k` values of a relation that does not grow (an `InRel` made of them): one frozen segment, shared
    /// by a relation started from it without a copy. `k` is given, not read off the rows: with none, a relation started
    /// from them (kept empty, grown in a later run) had no width, and its tail took no row.
    pub(super) fn frozen(k: usize, rows: Vec<i64>, n: usize) -> Rows {
        debug_assert_eq!(rows.len(), k * n, "{n} rows of {k} values");
        let mut r = Rows::new(k);
        if n > 0 && !rows.is_empty() {
            r.hint = n;
            r.extend_from_slice(&rows);
        }
        r.freeze(n)
    }

    /// The rows held.
    fn rows(&self) -> usize {
        self.segs.iter().map(|(_, p)| p.len() as usize).sum::<usize>() + self.tail.as_ref().map_or(0, |t| t.len() as usize)
    }

    /// The values held, every segment's and the tail's.
    pub(super) fn len(&self) -> usize {
        self.rows() * self.k
    }

    /// The bytes of the values held (the frozen segments counted whole, shared or not).
    pub(super) fn bytes(&self) -> usize {
        self.len() * 8
    }

    /// The bytes the chunks hold: the frozen segments' and the tail's.
    pub(super) fn capacity_bytes(&self) -> usize {
        self.segs.iter().map(|(_, p)| p.capacity_bytes()).sum::<usize>() + self.tail.as_ref().map_or(0, Part::capacity_bytes)
    }

    /// Room for `values` more values in the tail.
    pub(super) fn reserve(&mut self, values: usize) {
        if self.k == 0 {
            return;
        }
        match &mut self.tail {
            Some(t) => t.grow(u64::from(t.len()) + (values / self.k) as u64),
            None => self.tail = Some(Part::new(self.k, values / self.k)),
        }
    }

    /// Appends rows' values (`k` a row) to the tail.
    #[inline]
    pub(super) fn extend_from_slice(&mut self, rows: &[i64]) {
        if self.k == 0 {
            return;
        }
        debug_assert_eq!(rows.len() % self.k, 0, "rows of {} values", self.k);
        let n = rows.len() / self.k;
        let tail = self.tail.get_or_insert_with(|| Part::new(self.k, self.hint.max(n).max(16)));
        if n > 1 {
            tail.grow(u64::from(tail.len()) + n as u64);
        }
        for row in rows.chunks_exact(self.k) {
            tail.push(row);
        }
    }

    /// The values of rows `lo..hi`, copied into one vector.
    pub(super) fn to_vec(&self, lo: usize, hi: usize, k: usize) -> Vec<i64> {
        let mut out = Vec::with_capacity((hi - lo) * k);
        for id in lo..hi {
            out.extend_from_slice(&self.row(id as u32, k));
        }
        out
    }

    /// These `n` rows with the tail frozen into a segment of its own (moved, not copied): as a closed relation holds
    /// them, for a later run to share (`shared`).
    pub(super) fn freeze(mut self, n: usize) -> Rows {
        if let Some(tail) = self.tail.take().filter(|t| t.len() > 0) {
            self.segs.push((self.from, tail));
        }
        self.from = n as u32;
        while self.segs.len() > MAX_SEGMENTS {
            // The newest two, merged: a session's edits add small segments after one large one.
            let (_, b) = self.segs.pop().expect("two segments");
            let (first, a) = self.segs.pop().expect("two segments");
            let mut both = Part::new(self.k, (a.len() + b.len()) as usize);
            both.copy_from(&a, 0, a.len());
            both.copy_from(&b, 0, b.len());
            self.segs.push((first, both));
        }
        self
    }

    /// The `n` rows of a frozen relation, shared: its segments taken (and its tail copied, if it has one), with an
    /// empty tail from `n` on, for a relation that starts as them.
    pub(super) fn shared(&self, n: usize) -> Rows {
        let mut segs = self.segs.clone();
        if let Some(t) = self.tail.as_ref().filter(|t| t.len() > 0) {
            let mut c = Part::new(self.k, t.len() as usize);
            c.copy_from(t, 0, t.len());
            segs.push((self.from, c));
        }
        Rows { segs, from: n as u32, tail: None, k: self.k, hint: 0, merged: Default::default() }
    }

    /// The rows as one relation in one block, ids from 0: the one part, or every part merged into one (made once,
    /// and kept while these rows are). `None` for no rows.
    pub(super) fn in_block(&self) -> Option<InBlock> {
        let one = match (&self.segs[..], &self.tail) {
            ([], None) => return None,
            ([], Some(t)) if self.from == 0 => t,
            ([(0, p)], None) => p,
            (segs, tail) => self.merged.get_or_init(|| {
                let parts: Vec<&Part> = segs.iter().map(|(_, p)| p).chain(tail.iter()).collect();
                let mut one = Part::new(self.k, parts.iter().map(|p| p.len() as usize).sum());
                for p in parts {
                    one.copy_from(p, 0, p.len());
                }
                one
            }),
        };
        Some(InBlock { facts: one.0.facts.clone(), at: one.0.facts.block.offset_of(one.0.rel), rows: one.len(), part: one.clone() })
    }

    /// Whether these rows share a frozen segment with `other` (a relation started from it).
    pub(super) fn shares(&self, other: &Rows) -> bool {
        self.segs.iter().any(|(_, a)| other.segs.iter().any(|(_, b)| std::ptr::eq(a.0.rel, b.0.rel)))
    }

    /// The relation holding row `id`, and the row's place in it.
    #[inline]
    fn part_of(&self, id: u32) -> (&Part, u32) {
        if id >= self.from {
            return (self.tail.as_ref().expect("a row of the tail"), id - self.from);
        }
        let (first, p) = match &self.segs[..] {
            [only] => only,
            segs => &segs[segs.partition_point(|(first, _)| *first <= id) - 1],
        };
        (p, id - first)
    }

}

impl RowData for Rows {
    #[inline]
    fn row(&self, id: u32, k: usize) -> Row {
        if k == 0 {
            return Row::of(&[]);
        }
        let (p, i) = self.part_of(id);
        let rows = &p.rel().rows;
        Row::from_fn(k, |c| rows.get(i, c))
    }
    /// Row `id`'s value in column `c`: one read where it lies.
    #[inline]
    fn value(&self, id: u32, _k: usize, c: usize) -> i64 {
        let (p, i) = self.part_of(id);
        p.rel().rows.get(i, c)
    }
}

impl RowData for Vec<i64> {
    #[inline]
    fn row(&self, id: u32, k: usize) -> Row {
        Row::of(&self[id as usize * k..][..k])
    }
    #[inline]
    fn value(&self, id: u32, k: usize, c: usize) -> i64 {
        self[id as usize * k + c]
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
        assert_eq!(next.value(0, k, 2), 3);
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

    /// A job the chunk starts (its strata's, on another thread) puts its relations in the chunk's blocks: the
    /// scope is the run's context, carried.
    #[test]
    fn a_job_of_the_chunk_puts_its_relations_in_the_chunks_block() {
        in_chunk(|| {
            let mine = Arc::as_ptr(&block_of(&some_rows())) as usize;
            let theirs = std::sync::Mutex::new(0usize);
            crate::threads::scope(|jobs| {
                let got = &theirs;
                jobs.spawn("a test job", move |_| *got.lock().unwrap() = Arc::as_ptr(&block_of(&some_rows())) as usize);
            });
            assert_eq!(mine, theirs.into_inner().unwrap(), "the job wrote in the chunk's block");
        });
    }

    /// A run bound to the CPU takes anonymous blocks (Q58): resident only where written, never a pinned buffer of
    /// the device's pool, which the run's footprint would count whole.
    #[test]
    fn a_cpu_run_takes_anonymous_blocks() {
        let was = crate::code_rule_dsl::logic::plan::placement::set_binding(crate::code_rule_dsl::logic::plan::placement::Binding::Cpu);
        let r = in_chunk(some_rows);
        crate::code_rule_dsl::logic::plan::placement::set_binding(was);
        assert!(!block_of(&r).block.in_slab(), "a CPU run's block is anonymous");
    }

    /// A relation a chunk drops gives its chunks back to the chunk's block (decision 0342): the next relation of
    /// its size takes them, so the block's used bytes grow by the descriptors alone.
    #[test]
    fn a_dropped_relations_chunks_are_taken_by_the_next() {
        in_chunk(|| {
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
        in_chunk(|| {
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
