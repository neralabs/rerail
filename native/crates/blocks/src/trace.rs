//! A trace: one relation's history, its states at frontiers `EMPTY ..= frontier()`, each a `Read` of the same ids
//! (decision 0678 step 2). Any two of its states are a `Bistate` (`Trace::between`).
//!
//! `Trace` is the abstraction, and each store of layers is one:
//!
//! - **`BlockTrace`**: layers written as a trace, all of its state in one block (below).
//! - **`View`** (a base and its delta, `FrozenFacts::view`): two frontiers, the base alone and the view.
//! - **A continuation chain** (`chain`, `Relation::try_continue`): a frontier per relation of the chain, each read
//!   up to where the next continues it.
//! - **A kept key's files** (`KeptChain::trace`, in `kept`): a frontier per file. A file's rows for the key take ids
//!   of their own past the older files', and the newest entry's are the live ones: a file's entry kills the older
//!   rows and adds its own, and a reads-only entry changes nothing.
//!
//! Each reads what its store holds; none writes anything. `Upto` reads a state below an end, which is how an
//! adapter reads its earlier frontiers and its empty one.
//!
//! A `BlockTrace`'s state lies in its block, so the block alone says what each state of the relation is:
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
//! killed. The rows of an earlier frontier are never written again, so a version at it reads the same rows
//! however far the trace has gone on. A `Bistate` is two versions of one trace.
//!
//! Writing: `open` makes the next layer (`LayerW`), which pushes rows and kills rows of any layer; `commit` writes
//! the layer's end, then the directory's layer count, last. A kill in the open layer marks the row dead from the next
//! frontier, so no committed frontier reads it differently.

use crate::bistate::{Bistate, ColumnRead, Read, below};
use crate::block::Block;
use crate::ids::{ColumnIndex, RowId, WordIndex};
use crate::refs::{Ptr, Span};
use crate::relation::{RelW, Relation, Spec, View};
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

/// A relation's history: its states at frontiers `EMPTY ..= frontier()`. Ids below a frontier's end are the same rows
/// at every later frontier, so any two of its states are a `Bistate`.
pub trait Trace {
    /// Its state at a frontier.
    type Version<'s>: Read
    where
        Self: 's;

    /// Its latest frontier.
    fn frontier(&self) -> Frontier;

    /// Its state at `f`, at most `frontier()`. `EMPTY` reads no row.
    fn at(&self, f: Frontier) -> Self::Version<'_>;

    /// The change from `a` to `b`: a cold run's from nothing when `a` is `None`.
    fn between(&self, a: Option<Frontier>, b: Frontier) -> Bistate<Self::Version<'_>> {
        match a {
            Some(a) => {
                assert!(a <= b, "a change runs forward");
                Bistate::new(self.at(a), self.at(b))
            }
            None => Bistate::cold(self.at(b)),
        }
    }
}

/// A state read below `end`: its rows from `end` on are not read. How an adapter reads an earlier frontier of a store
/// that reads its latest (a view's base, a chain's ancestor, a kept key's older files), and its empty one.
#[derive(Clone, Copy)]
pub struct Upto<S> {
    state: S,
    end: RowId,
}

impl<S: Read> Upto<S> {
    pub fn new(state: S, end: RowId) -> Upto<S> {
        assert!(end <= state.end(), "an end within the state's rows");
        Upto { state, end }
    }

    /// The state read whole.
    pub fn whole(state: S) -> Upto<S> {
        let end = state.end();
        Upto { state, end }
    }
}

impl<S: Read> Read for Upto<S> {
    type Column<'s>
        = S::Column<'s>
    where
        Self: 's;

    fn arity(&self) -> usize {
        self.state.arity()
    }

    fn end(&self) -> RowId {
        self.end
    }

    fn live_word(&self, w: WordIndex) -> u64 {
        let mask = below(self.end, w);
        if mask == 0 { 0 } else { self.state.live_word(w) & mask }
    }

    fn get(&self, id: RowId, c: ColumnIndex) -> i64 {
        self.state.get(id, c)
    }

    fn column(&self, c: ColumnIndex) -> S::Column<'_> {
        self.state.column(c)
    }

    fn fill(&self, cols: &[ColumnIndex], from: RowId, out: &mut [i64]) {
        crate::bistate::fill_rows(cols, from, out, self.end);
        self.state.fill(cols, from, out)
    }
}

/// A view's history: the base alone (frontier 1), then the base and its delta (frontier 2). A full relation's view
/// has one frontier.
impl<'a> Trace for View<'a> {
    type Version<'s>
        = Upto<View<'a>>
    where
        Self: 's;

    fn frontier(&self) -> Frontier {
        Frontier(1 + u32::from(self.base.is_some()))
    }

    fn at(&self, f: Frontier) -> Upto<View<'a>> {
        assert!(f <= self.frontier(), "a frontier the view has");
        match (f.0, self.base) {
            (0, base) => Upto::new(View::full(base.unwrap_or(self.top)), RowId::ZERO),
            (1, Some(base)) => Upto::new(View::full(base), RowId::new(self.top.base_rows)),
            _ => Upto::whole(*self),
        }
    }
}

/// A relation read down its chain of continuations, as `Relation::get` reads it: ids `0 .. len_all`, its dead bits
/// marking the dead.
#[derive(Clone, Copy)]
pub struct ChainRead<'a>(pub &'a Relation);

/// A chain's column: its own rows resolved once, the ids below them read down the chain.
pub struct ChainColumn<'a> {
    rel: &'a Relation,
    own: ColRead,
    base_rows: u32,
    col: usize,
}

impl ColumnRead for ChainColumn<'_> {
    #[inline]
    fn get(&self, id: RowId) -> i64 {
        let id = id.get();
        if id >= self.base_rows { self.own.get(id - self.base_rows) } else { self.rel.get(id, self.col) }
    }
}

impl<'a> Read for ChainRead<'a> {
    type Column<'s>
        = ChainColumn<'a>
    where
        Self: 's;

    fn arity(&self) -> usize {
        self.0.arity as usize
    }

    fn end(&self) -> RowId {
        RowId::new(self.0.len_all())
    }

    fn live_word(&self, w: WordIndex) -> u64 {
        below(Read::end(self), w) & !self.0.dead.as_slice().get(w.get()).copied().unwrap_or(0)
    }

    fn get(&self, id: RowId, c: ColumnIndex) -> i64 {
        self.0.get(id.get(), c.get())
    }

    fn column(&self, c: ColumnIndex) -> ChainColumn<'a> {
        let base_rows = if self.0.continues() { self.0.base_rows } else { 0 };
        ChainColumn { rel: self.0, own: self.0.rows.reader(c.get()), base_rows, col: c.get() }
    }
}

/// A continuation chain's history: a frontier per relation of the chain, oldest first, each read up to the id where
/// the next one continues it, the last whole.
#[derive(Clone, Copy)]
pub struct Chain<'a> {
    top: &'a Relation,
}

/// The chain `top` ends: its relations continue one another in one block, down to a relation of its own.
pub fn chain(top: &Relation) -> Chain<'_> {
    let mut r = top;
    while let Some(b) = r.base() {
        r = b;
    }
    assert!(!r.continues() && !r.is_delta(), "a chain in one block, down to a relation of its own");
    Chain { top }
}

impl<'a> Trace for Chain<'a> {
    type Version<'s>
        = Upto<ChainRead<'a>>
    where
        Self: 's;

    fn frontier(&self) -> Frontier {
        Frontier(self.top.depth + 1)
    }

    fn at(&self, f: Frontier) -> Upto<ChainRead<'a>> {
        assert!(f <= self.frontier(), "a frontier the chain has");
        // Down from the top to the relation at depth `f - 1`, the end the one above it continues from.
        let want = f.0.max(1) - 1;
        let (mut r, mut end) = (self.top, self.top.len_all());
        while r.depth > want {
            end = r.base_rows;
            r = r.base().expect("a continuation's base in its block");
        }
        Upto::new(ChainRead(r), RowId::new(if f == Frontier::EMPTY { 0 } else { end }))
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
pub struct BlockTrace<'b> {
    block: &'b Block,
    dir: &'b TraceDir,
}

impl<'b> BlockTrace<'b> {
    /// A trace of `arity` columns in `block`, with room for `cap` layers: its directory, layer records and death
    /// column, written into the block.
    pub fn create(block: &'b Block, arity: usize, cap: u32) -> BlockTrace<'b> {
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
            BlockTrace { block, dir: &*dir }
        }
    }

    /// The trace whose directory lies at `off` in `block` (`offset`).
    ///
    /// # Safety
    /// A trace's directory (`create`) lies at `off` in `block`.
    pub unsafe fn at_offset(block: &'b Block, off: usize) -> BlockTrace<'b> {
        BlockTrace { block, dir: unsafe { &*block.ptr::<TraceDir>(off) } }
    }

    /// Its directory's offset in its block, which a block's root records to find it again.
    pub fn offset(&self) -> usize {
        self.block.offset_of(std::ptr::from_ref(self.dir))
    }

    pub fn arity(&self) -> usize {
        self.dir.arity as usize
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

impl<'b> Trace for BlockTrace<'b> {
    type Version<'s>
        = BlockVersion<'b>
    where
        Self: 's;

    /// Every layer committed.
    fn frontier(&self) -> Frontier {
        Frontier(self.dir.nlayers.load(Acquire))
    }

    fn at(&self, f: Frontier) -> BlockVersion<'b> {
        assert!(f <= self.frontier(), "a frontier the trace has committed");
        BlockVersion { trace: *self, at: f }
    }
}

/// A trace's open layer: rows pushed, rows of any layer killed, then committed.
pub struct LayerW<'b> {
    trace: BlockTrace<'b>,
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
pub struct BlockVersion<'b> {
    trace: BlockTrace<'b>,
    at: Frontier,
}

impl<'b> BlockVersion<'b> {
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
pub struct BlockColumn<'b> {
    layers: &'b [LayerRec],
    c: usize,
    last: Cell<Option<(u32, u32, ColRead)>>,
}

impl ColumnRead for BlockColumn<'_> {
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

impl<'b> Read for BlockVersion<'b> {
    type Column<'s>
        = BlockColumn<'b>
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

    fn column(&self, c: ColumnIndex) -> BlockColumn<'b> {
        BlockColumn { layers: self.layers(), c: c.get(), last: Cell::new(None) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bistate::check;
    use crate::block::kind;
    use std::sync::Mutex;

    fn spec() -> Spec {
        Spec { widths: vec![8, 8], order: Vec::new(), indexes: Vec::new(), min_log: crate::rows::MIN_LOG }
    }

    /// Row `i`'s values in the tests' traces.
    fn row(i: u32) -> [i64; 2] {
        [i64::from(i), i64::from(i) * 7 + 1]
    }

    /// Three layers: ten rows; ten more, killing row 3 and its own row 12; two more, killing row 5.
    fn three_layers(block: &Block) -> (BlockTrace<'_>, [Frontier; 3]) {
        let t = BlockTrace::create(block, 2, 8);
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

    fn live(v: &BlockVersion<'_>) -> Vec<u32> {
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
        check_every_pair(&t);
        let cold = t.between(None, f3);
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

    /// Every pair of a trace's frontiers is a bistate that keeps L1, and a cold run of each.
    fn check_every_pair<T: Trace>(t: &T) {
        let last = t.frontier().layers();
        for b in 0..=last {
            check(&t.between(None, Frontier::new(b)));
            for a in 0..=b {
                check(&t.between(Some(Frontier::new(a)), Frontier::new(b)));
            }
        }
    }

    /// A base and its delta are a trace of two frontiers: the base alone, then the view; its change between them is
    /// `View::bistate`'s.
    #[test]
    fn a_view_is_a_trace_of_two_frontiers() {
        let block = Block::anon(kind::FACTS, 16 << 20);
        let (spec, lock) = (Spec { order: vec![0], indexes: vec![vec![0]], ..spec() }, Mutex::new(()));
        let b = Relation::create(&block, &spec, None);
        let w = RelW { block: &block, rel: b, lock: &lock };
        for i in 0..10 {
            w.push(&row(i));
        }
        crate::relation::freeze(&block, b, &spec, None);
        let d = Relation::create(&block, &spec, Some(b));
        let w = RelW { block: &block, rel: d, lock: &lock };
        for i in 10..14 {
            w.push(&row(i));
        }
        w.kill(2);
        w.kill(5);
        crate::relation::freeze(&block, d, &spec, Some(b));
        let v = View { base: Some(b), top: d };
        assert_eq!(v.frontier(), Frontier::new(2));
        assert_eq!(v.at(Frontier::EMPTY).live_ids().count(), 0);
        assert_eq!(v.at(Frontier::new(1)).live_ids().count(), 10);
        check_every_pair(&v);
        let (s, old) = (v.between(Some(Frontier::new(1)), Frontier::new(2)), v.bistate());
        assert!(s.gone().eq(old.gone()) && s.added().eq(old.added()), "the trace's change is the view's bistate");
        assert_eq!(s.counts(), (2, 4));
        assert_eq!(View::full(b).frontier(), Frontier::new(1), "a full relation's view has one frontier");
    }

    /// A continuation chain is a trace of a frontier per relation, each read up to where the next continues it: a
    /// fork from row 6 of ten reads its own rows at ids 6 on, so the root's frontier stops at 6.
    #[test]
    fn a_continuation_chain_is_a_trace() {
        let block = Block::anon(kind::FACTS, 16 << 20);
        let (spec, lock) = (spec(), Mutex::new(()));
        let root = Relation::create(&block, &spec, None);
        let w = RelW { block: &block, rel: root, lock: &lock };
        for i in 0..10 {
            w.push(&row(i));
        }
        let fork = Relation::try_continue(&block, &spec, root, 6).expect("room");
        let w = RelW { block: &block, rel: fork, lock: &lock };
        for i in 100..104 {
            w.push(&row(i));
        }
        let fork2 = Relation::try_continue(&block, &spec, fork, 10).expect("room");
        let w = RelW { block: &block, rel: fork2, lock: &lock };
        for i in 200..203 {
            w.push(&row(i));
        }
        let t = chain(fork2);
        assert_eq!(t.frontier(), Frontier::new(3));
        let ends = (0..=3).map(|f| t.at(Frontier::new(f)).end().get()).collect::<Vec<_>>();
        assert_eq!(ends, [0, 6, 10, 13]);
        let want = |id: u32| match id {
            0..6 => row(id),
            6..10 => row(94 + id),
            _ => row(190 + id),
        };
        for f in 1..=3 {
            let v = t.at(Frontier::new(f));
            let col = v.column(ColumnIndex::new(1));
            for id in v.live_ids() {
                assert_eq!([v.get(id, ColumnIndex::new(0)), col.get(id)], want(id.get()), "row {id} at frontier {f}");
            }
        }
        check_every_pair(&t);
        assert_eq!(t.between(Some(Frontier::new(1)), Frontier::new(3)).counts(), (0, 7));
        assert_eq!(chain(root).frontier(), Frontier::new(1));
    }

    /// The directory lies in the block: found again from its offset, it reads the same frontiers.
    #[test]
    fn a_trace_is_found_again_from_its_block() {
        let block = Block::anon(kind::FACTS, 16 << 20);
        let (t, [f1, _, f3]) = three_layers(&block);
        let again = unsafe { BlockTrace::at_offset(&block, t.offset()) };
        assert_eq!(again.frontier(), f3);
        assert_eq!(live(&again.at(f1)), live(&t.at(f1)));
        assert_eq!(live(&again.at(f3)), live(&t.at(f3)));
    }
}
