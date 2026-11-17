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
        }
    }
}

/// Room a block keeps past a new relation's: what its other relations' tails may take meanwhile.
const MARGIN: usize = 1 << 20;

/// A block of relations: shared by every segment and tail in it, given back (to the pool) with the last.
pub struct Facts {
    pub block: Block,
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
}

/// The blocks new relations go in, while they have room: one a `Life`.
static OPEN: [Mutex<Option<Arc<Facts>>>; 2] = [Mutex::new(None), Mutex::new(None)];

/// A block of at least `bytes`: the pool's when it has room to spare (nothing evicted for it, and half the Metal
/// budget left to the device's own buffers), else anonymous. Either is read by the device in place.
fn lease(bytes: usize, life: Life) -> Block {
    let bytes = bytes.max(life.block_bytes());
    #[cfg(target_os = "macos")]
    if let Ok(b) = rerail_accel::gpu::shared::lease_block_spare(kind::FACTS, bytes, rerail_accel::budget::metal() / 2) {
        return b;
    }
    Block::anon(kind::FACTS, bytes)
}

/// One relation of a block: a segment or a tail.
#[derive(Clone)]
pub struct Part {
    facts: Arc<Facts>,
    life: Life,
    rel: *const Relation,
    /// Each column resolved once (its directory, which never moves while the relation is in its block): a read pays
    /// the chunk arithmetic and two loads, not the descriptor's walk too.
    cols: Arc<[ColRead]>,
}

// SAFETY: the relation lies in `facts`' block, which the part keeps; its writer is the one `Rows` holding it.
unsafe impl Send for Part {}
unsafe impl Sync for Part {}

impl Part {
    /// An empty relation of `k` columns with room for `rows` rows, in the open block or a new one.
    fn new(life: Life, k: usize, rows: usize) -> Part {
        let spec = Spec { widths: vec![8; k], order: Vec::new(), indexes: Vec::new(), min_log: rerail_blocks::rows::MIN_LOG };
        let need = rows * k * 8 + k * size_of::<rerail_blocks::rows::ColDir>() + 4096;
        let mut open = OPEN[life as usize].lock().unwrap_or_else(|e| e.into_inner());
        for fresh in [false, true] {
            let facts = match &*open {
                Some(f) if !fresh && f.block.reserved().saturating_sub(f.block.used()) >= need + MARGIN => f.clone(),
                _ => {
                    let f = Arc::new(Facts { block: lease(2 * need + MARGIN, life) });
                    *open = Some(f.clone());
                    f
                }
            };
            if let Some(rel) = Relation::try_create(&facts.block, &spec, None)
                && rel.rows.try_reserve(&facts.block, rows as u64)
            {
                let cols = (0..k).map(|c| rel.rows.reader(c)).collect();
                let rel = std::ptr::from_ref(rel);
                return Part { facts, life, rel, cols };
            }
        }
        unreachable!("a new block of {} bytes holds a relation of {need}", 2 * need + MARGIN)
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
        if self.rel().rows.try_reserve(&self.facts.block, n) {
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
    /// Every part merged into one, made once for a reader that binds one relation (`in_block`).
    merged: std::sync::OnceLock<Part>,
}

/// Where a relation's rows lie for a reader that binds its block (the device, decision 0325): the block, kept while
/// this lives, and the relation's descriptor's offset in it.
pub struct InBlock {
    pub facts: Arc<Facts>,
    pub at: usize,
    pub rows: u32,
}

impl Clone for Rows {
    /// The segments shared; the tail copied (it is written by one `Rows`).
    fn clone(&self) -> Rows {
        let tail = self.tail.as_ref().map(|t| {
            let mut c = Part::new(self.life, self.k, t.len() as usize);
            c.copy_from(t, 0, t.len());
            c
        });
        Rows { segs: self.segs.clone(), from: self.from, tail, k: self.k, hint: self.hint, life: self.life, merged: Default::default() }
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

    /// `n` rows of a relation that does not grow (an `InRel` made of them): one frozen segment, shared by a
    /// relation started from it without a copy.
    pub fn frozen(rows: Vec<i64>, n: usize) -> Rows {
        let mut r = Rows::default();
        if n > 0 && !rows.is_empty() {
            r.k = rows.len() / n;
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
        Rows { segs, from: n as u32, tail: None, k: self.k, hint: 0, life: self.life, merged: Default::default() }
    }

    /// The rows as one relation in one block, ids from 0: the one part, or every part merged into one (made once,
    /// and kept while these rows are). `None` for no rows.
    pub fn in_block(&self) -> Option<InBlock> {
        let one = match (&self.segs[..], &self.tail) {
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
        };
        Some(InBlock { facts: one.facts.clone(), at: one.facts.block.offset_of(one.rel), rows: one.len() })
    }

    /// Whether these rows share a frozen segment with `other` (a relation started from it).
    pub fn shares(&self, other: &Rows) -> bool {
        self.segs.iter().any(|(_, a)| other.segs.iter().any(|(_, b)| std::ptr::eq(a.rel, b.rel)))
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
            return (self.tail.as_ref().expect("a row of the tail"), id - self.from);
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

    /// A frozen relation's rows are shared by one started from it, which pushes its own after them; freezing that
    /// one moves its tail into a segment, and both read every row by id.
    #[test]
    fn started_rows_share_the_kept_segments_and_freeze_their_own() {
        let k = 2;
        let kept = Rows::frozen((0..20).collect(), 10);
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
}
