//! A trace: one relation's history as layers, oldest first, all of it in one block (decision 0678 step 2).
//!
//! Every part of a trace's state lies in its block, so the block alone says what each state of the relation is:
//!
//! - **Its directory** (`TraceDir`): the layers committed, and each one's relation and end.
//! - **Its rows**: a layer is a `relation::Relation` that continues the layer before's (`Relation::try_continue`,
//!   decision 0384 step 7), so a row's id is the same in every layer, and a layer's own rows are the ids from the
//!   layer before's end to its own.
//! - **Each row's death** (`died`): one 4-byte column by id, 0 while the row is live, else the first frontier at
//!   which it is dead. A row is written once, and its death once.
//! - **Each layer's kills**: the ids it killed, in its relation's `killed` rows: the change a layer made, for reads of
//!   a change in its size (step 3).
//!
//! A **frontier** is a count of committed layers. `Frontier(k)` reads the rows of layers `..k` that no layer `..k`
//! killed. The rows of an earlier frontier are never written again, so a `Version` at it reads the same rows
//! however far the trace has gone on. A `Bistate` is two versions of one trace.
//!
//! Writing: `open` makes the next layer (`LayerW`), which pushes rows and kills rows of any layer; `commit` writes
//! the layer's end, then the directory's layer count, last. A kill in the open layer marks the row dead from the next
//! frontier, so no committed frontier reads it differently.

use crate::bistate::{Bistate, ColumnRead, Read, below};
use crate::block::Block;
use crate::ids::{ColumnIndex, RowId, WordIndex};
use crate::refs::{Ptr, Span};
use crate::relation::{RelW, Relation, Spec};
use crate::rows::{ColRead, MIN_LOG, Rows};
use crate::structure;
use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering::{AcqRel, Acquire, Release}};

/// A state of a trace: the number of layers committed at it. `Frontier::EMPTY` is the trace before any layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Frontier(u32);

impl Frontier {
    pub const EMPTY: Frontier = Frontier(0);

    #[inline]
    pub const fn new(layers: u32) -> Frontier {
        Frontier(layers)
    }

    /// The layers committed at it.
    #[inline]
    pub const fn layers(self) -> u32 {
        self.0
    }
}

structure! {
    /// One committed layer: its relation, which continues the layer before's, and the id past its last row.
    pub struct LayerRec {
        pub rel: Ptr<Relation>,
        pub end: u32,
        pub pad: u32,
    }
}

structure! {
    /// A trace's directory, in its block.
    pub struct TraceDir {
        pub arity: u32,
        /// The layers its directory has room for.
        pub cap: u32,
        /// The layers committed: its latest frontier. Written last on a commit.
        pub nlayers: AtomicU32,
        /// 1 while a layer is open.
        pub open: AtomicU32,
        pub layers: Span<LayerRec>,
        /// Each row's death, by id: 0 while live, else the frontier from which it is dead.
        pub died: Ptr<Rows>,
    }
}

/// A trace where it lies: its block and its directory there. A handle, holding nothing of its own.
#[derive(Clone, Copy)]
pub struct Trace<'b> {
    block: &'b Block,
    dir: &'b TraceDir,
}

impl<'b> Trace<'b> {
    /// A trace of `arity` columns in `block`, with room for `cap` layers: its directory, layer records and death
    /// column, written into the block.
    pub fn create(block: &'b Block, arity: usize, cap: u32) -> Trace<'b> {
        assert!(cap > 0, "a trace has room for a layer");
        let dir: *mut TraceDir = block.alloc_t(1);
        let recs: *mut LayerRec = block.alloc_t(cap as usize);
        let died: *mut Rows = block.alloc_t(1);
        // SAFETY: each was just allocated in `block`, which is open, and no one else has them.
        unsafe {
            std::ptr::write_bytes(dir, 0, 1);
            std::ptr::write_bytes(recs, 0, cap as usize);
            std::ptr::write_bytes(died, 0, 1);
            (*dir).arity = u32::try_from(arity).expect("a trace's columns fit a u32");
            (*dir).cap = cap;
            Span::set(&raw const (*dir).layers, recs, cap as usize);
            Rows::init(died, block, &[4], MIN_LOG);
            Ptr::set(&raw const (*dir).died, died);
            Trace { block, dir: &*dir }
        }
    }

    /// The trace whose directory lies at `off` in `block` (`offset`).
    ///
    /// # Safety
    /// A trace's directory (`create`) lies at `off` in `block`.
    pub unsafe fn at_offset(block: &'b Block, off: usize) -> Trace<'b> {
        Trace { block, dir: unsafe { &*block.ptr::<TraceDir>(off) } }
    }

    /// Its directory's offset in its block, which a block's root records to find it again.
    pub fn offset(&self) -> usize {
        self.block.offset_of(std::ptr::from_ref(self.dir))
    }

    pub fn arity(&self) -> usize {
        self.dir.arity as usize
    }

    /// Its latest frontier: every layer committed.
    pub fn frontier(&self) -> Frontier {
        Frontier(self.dir.nlayers.load(Acquire))
    }

    /// The trace read at `f`.
    pub fn at(&self, f: Frontier) -> Version<'b> {
        assert!(f <= self.frontier(), "a frontier the trace has committed");
        Version { trace: *self, at: f }
    }

    /// The change from `a` to `b`: a cold run's from nothing when `a` is `None`.
    pub fn between(&self, a: Option<Frontier>, b: Frontier) -> Bistate<Version<'b>> {
        match a {
            Some(a) => {
                assert!(a <= b, "a change runs forward");
                Bistate::new(self.at(a), self.at(b))
            }
            None => Bistate::cold(self.at(b)),
        }
    }

    /// The committed layers' records, oldest first.
    fn layers(&self) -> &'b [LayerRec] {
        &self.dir.layers.as_slice()[..self.dir.nlayers.load(Acquire) as usize]
    }

    fn died(&self) -> &'b Rows {
        self.dir.died.get().expect("a trace's death column")
    }

    /// Opens its next layer, of relation `spec` (its columns' widths and what freeze would build): one at a time.
    pub fn open(&self, spec: &Spec) -> LayerW<'b> {
        assert_eq!(spec.widths.len(), self.arity(), "a layer of the trace's columns");
        assert_eq!(self.dir.open.swap(1, AcqRel), 0, "one layer open at a time");
        let n = self.dir.nlayers.load(Acquire);
        assert!(n < self.dir.cap, "room for another layer in the trace's directory");
        let rel = match self.layers().last() {
            None => Relation::create(self.block, spec, None),
            Some(prev) => {
                let base = prev.rel.get().expect("a committed layer's relation");
                Relation::try_continue(self.block, spec, base, prev.end).expect("room in the trace's block for a layer")
            }
        };
        LayerW { trace: *self, rel, layer: n, lock: Mutex::new(()), order: Mutex::new(()) }
    }
}

/// A trace's open layer: rows pushed, rows of any layer killed, then committed.
pub struct LayerW<'b> {
    trace: Trace<'b>,
    rel: &'b Relation,
    /// Its place: the layers committed before it.
    layer: u32,
    /// The rows' claims' lock (`Rows::claim`).
    lock: Mutex<()>,
    /// Keeps a row's id and its death's slot one claim apart.
    order: Mutex<()>,
}

impl<'b> LayerW<'b> {
    /// The frontier it will be once committed.
    pub fn frontier(&self) -> Frontier {
        Frontier(self.layer + 1)
    }

    /// The id past the last row pushed so far, of any layer.
    pub fn end(&self) -> RowId {
        RowId::new(self.rel.len_all())
    }

    /// Pushes a row: its id is the next one past every layer's rows.
    pub fn push(&self, row: &[i64]) -> RowId {
        assert_eq!(row.len(), self.trace.arity(), "a row of the trace's columns");
        let _g = self.order.lock().unwrap();
        let id = RelW { block: self.trace.block, rel: self.rel, lock: &self.lock }.push(row);
        let died = self.trace.died();
        let slot = died.claim(self.trace.block, &self.lock, 1);
        assert_eq!(slot, id, "a row's death lies at its id");
        // SAFETY: just claimed by this writer.
        unsafe { died.put(slot, 0, 0) };
        RowId::new(id)
    }

    /// Kills a live row of any layer: dead from this layer's frontier on, and recorded as this layer's kill.
    pub fn kill(&self, id: RowId) {
        assert!(id < self.end(), "a row pushed already");
        let died = self.trace.died();
        assert_eq!(died.get(id.get(), 0), 0, "row {id} is live");
        // SAFETY: row `id` was claimed (pushed); its death is written once, here, by the trace's one writer.
        unsafe { died.put(id.get(), 0, i64::from(self.layer + 1)) };
        RelW { block: self.trace.block, rel: self.rel, lock: &self.lock }.kill(id.get());
    }

    /// Commits the layer: its record written, then the trace's layer count, last. Returns its frontier.
    pub fn commit(self) -> Frontier {
        let dir = self.trace.dir;
        assert!(self.layer < dir.cap, "a layer within the directory");
        // SAFETY: the record is this layer's, in the directory's span, which no reader reads until the count below
        // covers it; its one writer is this layer's.
        unsafe {
            let rp = dir.layers.as_ptr().add(self.layer as usize).cast_mut();
            (*rp).end = self.rel.len_all();
            Ptr::set(&raw const (*rp).rel, self.rel);
        }
        dir.open.store(0, Release);
        dir.nlayers.store(self.layer + 1, Release);
        Frontier(self.layer + 1)
    }
}

/// A trace read at a frontier: the rows of its layers there, less those its layers there killed.
#[derive(Clone, Copy)]
pub struct Version<'b> {
    trace: Trace<'b>,
    at: Frontier,
}

impl<'b> Version<'b> {
    pub fn frontier(&self) -> Frontier {
        self.at
    }

    /// Its layers' records, oldest first.
    fn layers(&self) -> &'b [LayerRec] {
        &self.trace.layers()[..self.at.0 as usize]
    }
}

/// A version's column: the layer of the row last read, resolved once (`Rows::reader`), and found again by its ends
/// only when a read leaves it.
pub struct VersionColumn<'b> {
    layers: &'b [LayerRec],
    c: usize,
    last: Cell<Option<(u32, u32, ColRead)>>,
}

impl ColumnRead for VersionColumn<'_> {
    #[inline]
    fn get(&self, id: RowId) -> i64 {
        let id = id.get();
        if let Some((lo, hi, r)) = self.last.get()
            && (lo..hi).contains(&id)
        {
            return r.get(id - lo);
        }
        let k = self.layers.partition_point(|l| l.end <= id);
        let rel = self.layers[k].rel.get().expect("a committed layer's relation");
        let (lo, r) = (rel.base_rows, rel.rows.reader(self.c));
        self.last.set(Some((lo, self.layers[k].end, r)));
        r.get(id - lo)
    }
}

impl<'b> Read for Version<'b> {
    type Column<'s>
        = VersionColumn<'b>
    where
        Self: 's;

    fn arity(&self) -> usize {
        self.trace.arity()
    }

    fn end(&self) -> RowId {
        RowId::new(self.layers().last().map_or(0, |l| l.end))
    }

    /// A row is live when it lies below the frontier's end and its death, if any, is past the frontier.
    fn live_word(&self, w: WordIndex) -> u64 {
        let mask = below(self.end(), w);
        if mask == 0 {
            return 0;
        }
        let (died, first, at) = (self.trace.died().reader(0), w.first().get(), i64::from(self.at.0));
        let mut live = 0u64;
        for i in 0..64 - mask.leading_zeros() {
            let d = died.get(first + i);
            if d == 0 || d > at {
                live |= 1 << i;
            }
        }
        live
    }

    fn get(&self, id: RowId, c: ColumnIndex) -> i64 {
        let top = self.layers().last().expect("a row of a nonempty version").rel.get().expect("a committed layer's relation");
        top.get(id.get(), c.get())
    }

    fn column(&self, c: ColumnIndex) -> VersionColumn<'b> {
        VersionColumn { layers: self.layers(), c: c.get(), last: Cell::new(None) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bistate::check;
    use crate::block::kind;

    fn spec() -> Spec {
        Spec { widths: vec![8, 8], order: Vec::new(), indexes: Vec::new(), min_log: crate::rows::MIN_LOG }
    }

    /// Row `i`'s values in the tests' traces.
    fn row(i: u32) -> [i64; 2] {
        [i64::from(i), i64::from(i) * 7 + 1]
    }

    /// Three layers: ten rows; ten more, killing row 3 and its own row 12; two more, killing row 5.
    fn three_layers(block: &Block) -> (Trace<'_>, [Frontier; 3]) {
        let t = Trace::create(block, 2, 8);
        let spec = spec();
        let w = t.open(&spec);
        for i in 0..10 {
            assert_eq!(w.push(&row(i)), RowId::new(i));
        }
        let f1 = w.commit();
        let w = t.open(&spec);
        for i in 10..20 {
            w.push(&row(i));
        }
        w.kill(RowId::new(3));
        w.kill(RowId::new(12));
        let f2 = w.commit();
        let w = t.open(&spec);
        w.kill(RowId::new(5));
        for i in 20..22 {
            w.push(&row(i));
        }
        let f3 = w.commit();
        (t, [f1, f2, f3])
    }

    fn live(v: &Version<'_>) -> Vec<u32> {
        v.live_ids().map(RowId::get).collect()
    }

    /// Each frontier reads its layers' rows less its layers' kills, whatever came after it.
    #[test]
    fn a_frontier_reads_its_layers_rows_less_their_kills() {
        let block = Block::anon(kind::FACTS, 16 << 20);
        let (t, [f1, f2, f3]) = three_layers(&block);
        assert_eq!(t.frontier(), f3);
        assert_eq!(live(&t.at(Frontier::EMPTY)), Vec::<u32>::new());
        assert_eq!(live(&t.at(f1)), (0..10).collect::<Vec<_>>());
        assert_eq!(live(&t.at(f2)), (0..20).filter(|&i| i != 3 && i != 12).collect::<Vec<_>>());
        assert_eq!(live(&t.at(f3)), (0..22).filter(|&i| ![3, 5, 12].contains(&i)).collect::<Vec<_>>());
        for f in [f1, f2, f3] {
            let v = t.at(f);
            for id in v.live_ids() {
                assert_eq!([v.get(id, ColumnIndex::new(0)), v.get(id, ColumnIndex::new(1))], row(id.get()), "row {id} at {f:?}");
            }
        }
        // A kill in an open layer changes no committed frontier.
        let w = t.open(&spec());
        w.kill(RowId::new(0));
        assert!(t.at(f3).live(RowId::new(0)), "dead only from the open layer's frontier");
        let f4 = w.commit();
        assert!(!t.at(f4).live(RowId::new(0)) && t.at(f3).live(RowId::new(0)));
    }

    /// Two frontiers are a bistate (L1 by `check`), and the bistates of a chain of frontiers compose to the cold run's
    /// (L3): each row's liveness at the last frontier, folded from nothing through the steps' gone and added rows,
    /// is its liveness there. Every set is walked in id order, in place.
    #[test]
    fn a_chain_of_frontiers_composes_to_the_cold_run() {
        let block = Block::anon(kind::FACTS, 16 << 20);
        let (t, [f1, f2, f3]) = three_layers(&block);
        let chain = [None, Some(f1), Some(f2)].into_iter().zip([f1, f2, f3]).map(|(a, b)| t.between(a, b)).collect::<Vec<_>>();
        for s in &chain {
            check(s);
        }
        check(&t.between(Some(f1), f3));
        let cold = t.between(None, f3);
        check(&cold);
        let mut steps: Vec<_> = chain.iter().map(|s| (s.gone().peekable(), s.added().peekable())).collect();
        let mut cold_ids = cold.added().peekable();
        for id in (0..t.at(f3).end().get()).map(RowId::new) {
            let mut live = false;
            for (gone, added) in &mut steps {
                if gone.next_if_eq(&id).is_some() {
                    live = false;
                }
                if added.next_if_eq(&id).is_some() {
                    live = true;
                }
            }
            assert_eq!(live, cold_ids.next_if_eq(&id).is_some(), "row {id}: the chain's liveness is the cold run's");
        }
        let (gone, added) = t.between(Some(f1), f3).counts();
        assert_eq!((gone, added), (2, 11), "rows 3 and 5 gone; 10..22 less 12 added");
    }

    /// A column resolved once and a fill of several columns read what `get` reads, across layers' boundaries.
    #[test]
    fn columns_and_fills_read_what_cells_read() {
        let block = Block::anon(kind::FACTS, 16 << 20);
        let (t, [_, _, f3]) = three_layers(&block);
        let v = t.at(f3);
        let cols = [ColumnIndex::new(1), ColumnIndex::new(0)];
        let c1 = v.column(cols[0]);
        // Out of order, so the column leaves the layer it last read.
        for id in [15u32, 2, 21, 9, 10, 0, 19, 20].map(RowId::new) {
            assert_eq!(c1.get(id), v.get(id, cols[0]), "row {id}");
        }
        let mut out = [0i64; 2 * 14];
        v.fill(&cols, RowId::new(8), &mut out);
        for i in 0..14u32 {
            assert_eq!([out[i as usize + 14], out[i as usize]], row(8 + i), "row {} of the fill", 8 + i);
        }
    }

    /// The directory lies in the block: found again from its offset, it reads the same frontiers.
    #[test]
    fn a_trace_is_found_again_from_its_block() {
        let block = Block::anon(kind::FACTS, 16 << 20);
        let (t, [f1, _, f3]) = three_layers(&block);
        let again = unsafe { Trace::at_offset(&block, t.offset()) };
        assert_eq!(again.frontier(), f3);
        assert_eq!(live(&again.at(f1)), live(&t.at(f1)));
        assert_eq!(live(&again.at(f3)), live(&t.at(f3)));
    }
}
