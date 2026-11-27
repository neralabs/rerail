//! A trace's state, read where its layers lie (P§12's `Version`, "its View generalized from a base and one delta to n
//! layers"; moved from `rerail_blocks::layer`'s `View::chain`): its newest layer's block id, its layers reached through
//! the launch's descriptor table (`Reach`), each by a reference (a block id and an offset) to the one below.
//!
//! A row is live when it is an own row of a layer at or below the state's and no layer above that one, up to the
//! state's, kills it. Every read here is in place, and keeps its own state in fixed arrays bounded by `MAX_LAYERS`: a
//! commit's kill search reads at most `MAX_BELOW` layers below it (consolidate's kills phase's bound), so a trace has
//! at most one more, whatever its roll-up (the geometric one keeps far fewer: each layer at most half the one below it,
//! O(log n) of them); a chain past the bound is refused when its state is made (the bounds axis's "refuse"). Nothing
//! here allocates, so nothing reads a trace's rows into a structure of its own.

use crate::block::{BODY_AT, Id, Ref, refs};
use crate::col::Dec;
use crate::hash::key_hash;
use crate::ids::{ColumnIndex, RowId, WordIndex};
use crate::layer::{Layer, partition_point};
use crate::relation::Relation;
use crate::runs::MAX_COLS;
use crate::table::Reach;
use core::cmp::Ordering;
use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// The layers below it a commit's kill search reads at most: consolidate's kills phase's bound on an item's steps
/// (`Bound::Steps`), past which the commit fails.
pub const MAX_BELOW: usize = 64;

/// A trace's layers at most (`MAX_BELOW` and the newest): a state of more is refused (`ChainError::TooDeep`).
pub const MAX_LAYERS: usize = MAX_BELOW + 1;

/// What a state records of its change since its version (decision 0678 step 4): the version's end, and the
/// version's ids whose liveness may differ now, ascending, each once (a superset is allowed: each is checked). Ids
/// from `end` on are rows new since.
#[derive(Clone, Copy, Debug)]
pub struct Since<'s> {
    pub end: RowId,
    pub flips: &'s [u32],
}

/// A column's statistics (P§12 `stats`; P§11.2: "per column min, max and counts"): its live rows' bounds (a trace's
/// are an estimate: the bounds of every row added, gone ones included), and its sketch's reading (rows, distinct
/// values, the sum of each value's rows squared).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColStats {
    pub min: i64,
    pub max: i64,
    pub rows: u64,
    pub ndv: u64,
    pub sq: u64,
}

/// Why a chain of layers is not a state: each check `State::chain` makes once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainError {
    /// The launch's table does not reach the block.
    Unreachable(Id),
    /// The block is not a committed layer.
    NotALayer(Id, &'static str),
    /// Its relation is not where the layer above refers.
    Misplaced(Id),
    Arity { id: Id, has: usize, want: usize },
    /// It continues a layer it does not refer to (O§2: a reference is in the block's reference table).
    Unreferenced { id: Id, below: Id },
    /// More layers than `MAX_LAYERS`.
    TooDeep,
}

impl fmt::Display for ChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            ChainError::Unreachable(id) => write!(f, "layer #{id} is not reachable from the launch"),
            ChainError::NotALayer(id, e) => write!(f, "block #{id}: {e}"),
            ChainError::Misplaced(id) => write!(f, "layer #{id}'s relation is not where the layer above refers"),
            ChainError::Arity { id, has, want } => write!(f, "layer #{id} has {has} columns, not {want}"),
            ChainError::Unreferenced { id, below } => write!(f, "layer #{id} continues #{below} and does not refer to it"),
            ChainError::TooDeep => write!(f, "a trace of more than {MAX_LAYERS} layers"),
        }
    }
}

/// A trace's state: its layers, from `head` down, read through `reach`; restricted to a partition (`restrict`), its
/// reads counted (`counted`).
#[derive(Clone, Copy)]
pub struct State<'a> {
    reach: &'a dyn Reach,
    head: Option<Id>,
    arity: u32,
    part: Option<i64>,
    reads: Option<&'a AtomicU64>,
}

/// Block ids of a chain's layers, newest first: as many as it has, at most `MAX_LAYERS`.
#[derive(Clone, Copy)]
pub struct Ids {
    ids: [Id; MAX_LAYERS],
    n: usize,
}

impl Ids {
    pub fn as_slice(&self) -> &[Id] {
        &self.ids[..self.n]
    }
}

impl<'a> State<'a> {
    /// The state at layer `head` (`None`: the empty state, of `arity` columns), its layers reached through `reach`.
    /// Each layer is checked once: reached, committed, of `arity` columns, where the layer above refers to it, and
    /// referred to by it (O§2: a reference is a block id and an offset, the layer's header); at most `MAX_LAYERS`.
    pub fn chain(reach: &'a dyn Reach, head: Option<Id>, arity: usize) -> Result<State<'a>, ChainError> {
        let s = State { reach, head, arity: arity as u32, part: None, reads: None };
        let (mut at, mut depth) = (head.map(|h| (h, None)), 0usize);
        while let Some((id, off)) = at {
            depth += 1;
            if depth > MAX_LAYERS {
                return Err(ChainError::TooDeep);
            }
            let l = s.try_layer(id)?;
            if off.is_some_and(|o: usize| o != l.head.rows as usize) {
                return Err(ChainError::Misplaced(id));
            }
            if l.head.shape.arity() != arity || l.rows.arity as usize != arity {
                return Err(ChainError::Arity { id, has: l.head.shape.arity(), want: arity });
            }
            if let Some((b, _)) = l.below()
                && !refs(l.bytes).contains(&Ref { id: b, at: BODY_AT as u32 })
            {
                return Err(ChainError::Unreferenced { id, below: b });
            }
            at = l.below().map(|(b, o)| (b, Some(o)));
        }
        Ok(s)
    }

    fn try_layer(&self, id: Id) -> Result<Layer<'a>, ChainError> {
        let b = self.reach.block(id).ok_or(ChainError::Unreachable(id))?;
        Layer::new(b).map_err(|e| ChainError::NotALayer(id, e))
    }

    /// How its layers are reached.
    pub fn reach(&self) -> &'a dyn Reach {
        self.reach
    }

    /// Its top layer's block: `None` for the empty state.
    pub fn head(&self) -> Option<Id> {
        self.head
    }

    /// The partition it is restricted to.
    pub fn part(&self) -> Option<i64> {
        self.part
    }

    pub fn arity(&self) -> usize {
        self.arity as usize
    }

    /// The same state, its reads counted into `reads` (a search's steps, each a read of a row or a kill).
    pub fn counted(self, reads: &'a AtomicU64) -> State<'a> {
        State { reads: Some(reads), ..self }
    }

    /// Partition `p`'s rows alone, by each layer's partition index.
    pub fn restrict(self, p: i64) -> State<'a> {
        assert!(self.part.is_none_or(|q| q == p), "a restriction of another partition's");
        State { part: Some(p), ..self }
    }

    /// The empty state of the same trace.
    pub fn empty(self) -> State<'a> {
        State { head: None, ..self }
    }

    /// Whether two states of one trace are the same state.
    pub fn same(&self, other: &State<'_>) -> bool {
        self.head == other.head && self.part == other.part && self.arity == other.arity
    }

    #[inline]
    pub fn touch(&self, n: usize) {
        if let Some(r) = self.reads {
            r.fetch_add(n as u64, Relaxed);
        }
    }

    /// Layer `id`, checked when the state was made.
    pub fn layer(&self, id: Id) -> Layer<'a> {
        self.try_layer(id).expect("a chain's layers were checked when it was made")
    }

    /// Its layers, newest first, with their block ids.
    pub fn layers(&self) -> impl Iterator<Item = (Id, Layer<'a>)> + use<'a> {
        let s = *self;
        let mut id = s.head;
        core::iter::from_fn(move || {
            let i = id?;
            let l = s.layer(i);
            id = l.below().map(|(b, _)| b);
            Some((i, l))
        })
    }

    /// Its layers' block ids, newest first.
    pub fn ids(&self) -> Ids {
        let mut out = Ids { ids: [0; MAX_LAYERS], n: 0 };
        for (id, _) in self.layers() {
            out.ids[out.n] = id;
            out.n += 1;
        }
        out
    }

    /// The first id past the state's: its newest layer's end (0 for the empty state).
    pub fn end(&self) -> u32 {
        self.head.map_or(0, |h| self.layer(h).end())
    }

    /// The layers above the state `before` ends (all of them for `None`), newest first: `None` when `before` is not
    /// one of this state's.
    pub fn above(&self, before: Option<&State<'_>>) -> Option<Ids> {
        let stop = before.and_then(|b| b.head);
        let mut out = Ids { ids: [0; MAX_LAYERS], n: 0 };
        for (id, _) in self.layers() {
            if Some(id) == stop {
                return Some(out);
            }
            out.ids[out.n] = id;
            out.n += 1;
        }
        stop.is_none().then_some(out)
    }

    /// The layer holding id `id`.
    fn locate(&self, id: u32) -> Option<Layer<'a>> {
        self.layers().map(|(_, l)| l).find(|l| l.start() <= id && id < l.end())
    }

    /// Partition `p`'s own rows of layer `l`: all of them without a restriction.
    pub fn span(&self, l: &Layer<'a>) -> core::ops::Range<u32> {
        match self.part {
            None => l.start()..l.end(),
            Some(p) => l.part_rows(p),
        }
    }

    /// Whether row `id`, of layer `l`, is killed by a layer above `l`, up to this state's: a search of each one's
    /// kills (its partition's, when restricted: a row of one is killed only by a kill of it), each step a counted read.
    pub fn killed_above(&self, l: &Layer<'a>, id: u32) -> bool {
        let p = self.part.unwrap_or_else(|| l.part_of(id));
        self.layers().map(|(_, k)| k).take_while(|k| !core::ptr::eq(k.rows, l.rows)).any(|k| {
            let ks = k.part_kills(p);
            let at = partition_point(ks.len(), |i| {
                self.touch(1);
                ks[i] < id
            });
            ks.get(at) == Some(&id)
        })
    }

    pub fn get(&self, id: u32, c: usize) -> i64 {
        self.locate(id).unwrap_or_else(|| panic!("row {id} is not in the state")).get(id, c)
    }

    pub fn live(&self, id: u32) -> bool {
        self.locate(id).is_some_and(|l| self.span(&l).contains(&id) && !self.killed_above(&l, id))
    }

    /// The live bits of word `w`: each layer's own rows in the word (its partition's, when restricted), less every
    /// kill of one of them by a layer above it. A layer kills only rows below its own, so a kill names a live row of
    /// the state it was made over, and a row killed is never live again (a row added again is a new row): the kills
    /// are taken after every layer's rows are in.
    pub fn live_word(&self, w: WordIndex) -> u64 {
        self.touch(1);
        let (first, end) = (w.first().get(), self.end());
        let last = first.saturating_add(64).min(end);
        if first >= last {
            return 0;
        }
        let (mut live, mut killed) = (0u64, 0u64);
        // The word's rows, a run a layer (at most a few: a word is 64 ids), each with its partitions: a layer's own rows
        // lie in the canonical order, its partition first, so its rows in the word are a range of partitions (the
        // restriction's alone, when restricted).
        let mut runs: [(u32, u32, i64, i64); 4] = [(0, 0, 0, 0); 4];
        let mut nruns = 0;
        for (_, l) in self.layers() {
            if l.end() <= first {
                break;
            }
            let s = self.span(&l);
            let (lo, hi) = (s.start.max(first), s.end.min(last));
            if lo < hi {
                live |= if hi - lo == 64 { u64::MAX } else { ((1u64 << (hi - lo)) - 1) << (lo - first) };
                let (a, b) = self.part.map_or_else(|| (l.part_of(lo), l.part_of(hi - 1)), |p| (p, p));
                if nruns < runs.len() {
                    runs[nruns] = (lo, hi, a, b);
                } else {
                    // More layers in one word than room: the last run covers them all.
                    let r = &mut runs[runs.len() - 1];
                    *r = (lo.min(r.0), hi.max(r.1), a.min(r.2), b.max(r.3));
                }
                nruns = (nruns + 1).min(runs.len());
            }
        }
        // Each run's kills by the layers above its own (a layer kills only rows below it), by its partitions.
        for (_, k) in self.layers() {
            if k.start() <= first {
                break;
            }
            for &(lo, hi, a, b) in runs[..nruns].iter().filter(|r| r.0 < k.start()) {
                k.kills_within(a..=b, lo..hi, |x| killed |= 1u64 << (x - first));
            }
        }
        live & !killed
    }

    /// Its top layer's kills as one ascending run: restricted, its partition's; else when it changed one partition
    /// at most. Otherwise none (`flips_since` gives them).
    pub fn since(&self) -> Option<Since<'a>> {
        let l = self.layer(self.head?);
        let flips = match self.part {
            Some(p) => l.part_kills(p),
            None if l.parts.rows.len() <= 1 => l.kills(),
            None => return None,
        };
        Some(Since { end: RowId::new(l.start()), flips })
    }

    /// The ids the layers above `before` killed, ascending, each once (`ChainFlips`); `None` when `before` is not one
    /// of this state's.
    pub fn flips_since(&self, before: Option<&State<'_>>) -> Option<ChainFlips<'a>> {
        let above = self.above(before)?;
        let all = self.ids();
        let mut f = ChainFlips { spans: [(0, 0); MAX_LAYERS], n: all.n, up: [None; MAX_LAYERS], nup: 0, last: None };
        for (k, &id) in all.as_slice().iter().enumerate() {
            let l = self.layer(id);
            f.spans[k] = (l.start(), l.end());
            if k < above.n {
                let (kills, runs, restricted) = match self.part {
                    Some(p) => (l.part_kills(p), 1, true),
                    None => (l.kills(), l.parts.rows.len(), false),
                };
                f.up[f.nup] = Some(Upper { parts: l.parts, kills, runs, restricted, stop: k, lower: all.n, e: 0, cur: (0, 0) });
                f.nup += 1;
            }
        }
        Some(f)
    }

    /// The ids past `before`'s end live in some layer above it: a range a layer, ascending; `None` when `before` is not
    /// one of this state's.
    pub fn new_ranges(&self, before: Option<&State<'_>>) -> Option<Ranges> {
        let above = self.above(before)?;
        let mut r = Ranges { r: [(RowId::ZERO, RowId::ZERO); MAX_LAYERS], n: 0, at: 0 };
        for &id in above.as_slice().iter().rev() {
            let s = self.span(&self.layer(id));
            if !s.is_empty() {
                r.r[r.n] = (RowId::new(s.start), RowId::new(s.end));
                r.n += 1;
            }
        }
        Some(r)
    }

    /// Rows gone and added since `before`, when one layer lies above it (its header's counts, or its partition's).
    pub fn exact_counts_since(&self, before: Option<&State<'_>>) -> Option<(usize, usize)> {
        let above = self.above(before)?;
        let [id] = above.as_slice() else { return None };
        let l = self.layer(*id);
        match self.part {
            None => Some((l.head.ngone as usize, l.head.added as usize)),
            Some(p) => Some(l.part_entry(p).map_or((0, 0), |(_, added, gone)| (gone as usize, added as usize))),
        }
    }

    pub fn partition(&self) -> Option<ColumnIndex> {
        let l = self.layer(self.head?);
        l.head.shape.partition().map(|c| ColumnIndex::new(c as u16))
    }

    /// The whole state's statistics on column `c`, from its layers: the bounds of every row added (an estimate: rows
    /// since gone included), and its last layer's sketch's reading. `None` restricted or empty: a partition's are its
    /// rows'.
    pub fn stats(&self, c: ColumnIndex) -> Option<ColStats> {
        if self.part.is_some() {
            return None;
        }
        let head = self.head?;
        let (mut min, mut max) = (i64::MAX, i64::MIN);
        for (_, l) in self.layers() {
            if let Some(s) = l.rows.stats.as_slice().get(c.get()).filter(|_| l.n() > 0) {
                min = min.min(s.min);
                max = max.max(s.max);
            }
        }
        let r = self.layer(head).reading(c.get()).unwrap_or_default();
        Some(ColStats { min, max, rows: r.rows, ndv: r.ndv, sq: r.sq })
    }

    /// Its rows whose `cols` equal `key`, by each layer's indexes (`ProbeIter`).
    pub fn matching(&self, cols: &[ColumnIndex], key: &[i64]) -> ProbeIter<'a> {
        assert!(cols.len() <= MAX_COLS && cols.len() == key.len(), "a key of at most MAX_COLS columns");
        let cs: [u16; MAX_COLS] = core::array::from_fn(|i| cols.get(i).map_or(0, |c| c.get() as u16));
        let k: [i64; MAX_COLS] = core::array::from_fn(|i| key.get(i).copied().unwrap_or(0));
        ProbeIter { chain: *self, cs, k, n: cols.len(), inner: None, layer: self.head }
    }

    /// Its live rows in canonical order (`Scan`).
    pub fn scan(&self) -> Scan<'a> {
        Scan::new(self)
    }

    /// Gives `out` each partition changed since `before`: the layers above it, their `parts` merged on the partition,
    /// digests summed. False when `before` is not one of this state's.
    pub fn changed_parts_since(&self, before: Option<&State<'_>>, out: &mut dyn FnMut(i64)) -> bool {
        let Some(above) = self.above(before) else { return false };
        let n = above.n;
        let mut parts: [Option<&Relation>; MAX_LAYERS] = [None; MAX_LAYERS];
        for (k, &id) in above.as_slice().iter().enumerate() {
            parts[k] = Some(self.layer(id).parts);
        }
        let p = |k: usize| parts[k].expect("a layer above");
        let mut at = [0u32; MAX_LAYERS];
        loop {
            let key = (0..n).filter(|&k| at[k] < p(k).rows.len()).map(|k| p(k).own(at[k], 0)).min();
            let Some(key) = key else { return true };
            let (mut digest, mut moved) = (0u64, 0u64);
            for (k, a) in at.iter_mut().enumerate().take(n) {
                let r = p(k);
                if *a < r.rows.len() && r.own(*a, 0) == key {
                    digest = digest.wrapping_add(r.own(*a, 1) as u64);
                    moved += (r.own(*a, 2) + r.own(*a, 3)) as u64;
                    *a += 1;
                }
            }
            // One layer's partition changed when it holds a row of it; across layers, when their digests do not
            // cancel (an add and its undo sum to zero).
            if ((n == 1 && moved > 0) || digest != 0) && self.part.is_none_or(|q| q == key) {
                out(key);
            }
        }
    }
}

/// The new ids of a state since another (`State::new_ranges`): a range a layer above, ascending.
#[derive(Clone, Copy)]
pub struct Ranges {
    r: [(RowId, RowId); MAX_LAYERS],
    n: usize,
    at: usize,
}

impl Iterator for Ranges {
    type Item = (RowId, RowId);

    fn next(&mut self) -> Option<(RowId, RowId)> {
        let r = self.r[..self.n].get(self.at).copied()?;
        self.at += 1;
        Some(r)
    }
}

/// One layer above a state's version, as its kills are read in id order: its kills by partition (`gone`'s id column,
/// or its restriction's run), read a layer below at a time, oldest first, each of its partitions' kills of that layer's
/// rows in turn. A layer's own rows lie in its canonical order, the partition first, and the layers below lie in id
/// order, oldest first: so (lower layer, partition, id) is id order, with a cursor of a few words.
#[derive(Clone, Copy)]
struct Upper<'a> {
    parts: &'a Relation,
    kills: &'a [u32],
    /// Its partition entries (one, its run, when restricted).
    runs: u32,
    restricted: bool,
    /// Its own place in the chain (newest first): the layers below it are those past it.
    stop: usize,
    /// The chain place of the layer below being read (from the oldest, `n - 1`, toward `stop + 1`); `n` before the
    /// first.
    lower: usize,
    /// The next entry to read of the layer below.
    e: u32,
    /// The kills being read: `kills[cur.0..cur.1]`.
    cur: (usize, usize),
}

impl Upper<'_> {
    /// Its next kill, in id order, moved to it.
    fn peek(&mut self, spans: &[(u32, u32)]) -> Option<u32> {
        loop {
            if self.cur.0 < self.cur.1 {
                return Some(self.kills[self.cur.0]);
            }
            // The oldest layer below first; then, its entries read, the next layer below, toward this one.
            if self.lower == spans.len() {
                self.lower = spans.len().checked_sub(1)?;
                self.e = 0;
            } else if self.e >= self.runs {
                self.lower = self.lower.checked_sub(1)?;
                self.e = 0;
            }
            if self.lower <= self.stop {
                return None;
            }
            if self.e >= self.runs {
                continue;
            }
            let (at, n) = if self.restricted {
                (0, self.kills.len())
            } else {
                let base = self.parts.base_rows;
                (self.parts.own(base + self.e, 5) as usize, self.parts.own(base + self.e, 3) as usize)
            };
            self.e += 1;
            let run = &self.kills[at..at + n];
            let (lo, hi) = spans[self.lower];
            self.cur = (at + partition_point(n, |i| run[i] < lo), at + partition_point(n, |i| run[i] < hi));
        }
    }
}

/// A state's recorded change over the layers above its version (`State::flips_since`): each one's kills, merged
/// ascending, each once. The ids are read where they lie; it holds a cursor a layer above, and each layer's span.
#[derive(Clone, Copy)]
pub struct ChainFlips<'a> {
    spans: [(u32, u32); MAX_LAYERS],
    n: usize,
    up: [Option<Upper<'a>>; MAX_LAYERS],
    nup: usize,
    last: Option<u32>,
}

impl ChainFlips<'_> {
    /// Whether no id flipped.
    pub fn is_empty(&self) -> bool {
        let mut f = *self;
        f.next().is_none()
    }
}

impl Iterator for ChainFlips<'_> {
    type Item = RowId;

    fn next(&mut self) -> Option<RowId> {
        let spans = &self.spans[..self.n];
        loop {
            let mut best: Option<(u32, usize)> = None;
            for (k, u) in self.up[..self.nup].iter_mut().enumerate() {
                if let Some(id) = u.as_mut().and_then(|u| u.peek(spans))
                    && best.is_none_or(|(b, _)| id < b)
                {
                    best = Some((id, k));
                }
            }
            let (id, k) = best?;
            self.up[k].as_mut().expect("a layer above").cur.0 += 1;
            if self.last != Some(id) {
                self.last = Some(id);
                return Some(RowId::new(id));
            }
        }
    }
}

/// A probe of a trace's state: each layer's matches in turn, newest first, by the layer's indexes: a range of the
/// canonical order when the key is a prefix of it, a sorted order (`Order`), a packed layer's permutation index, a
/// hashed index (`Hash`, or the row index), else its rows; a Bloom index skips a layer that cannot hold the key.
pub struct ProbeIter<'a> {
    chain: State<'a>,
    cs: [u16; MAX_COLS],
    k: [i64; MAX_COLS],
    n: usize,
    inner: Option<(Layer<'a>, Candidates<'a>)>,
    layer: Option<Id>,
}

/// A layer's candidates for a probe: a range of ids, or a list of them.
enum Candidates<'a> {
    Range(core::ops::Range<u32>),
    Listed(core::slice::Iter<'a, u32>),
    /// A packed layer's permutation index: positions `at` of its `perm`, each an own row from `start`.
    Perm { perm: Dec<'a>, at: core::ops::Range<usize>, start: u32 },
}

impl Iterator for Candidates<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        match self {
            Candidates::Range(r) => r.next(),
            Candidates::Perm { perm, at, start } => at.next().map(|i| *start + perm.get(i) as u32),
            Candidates::Listed(s) => s.next().copied(),
        }
    }
}

impl Iterator for ProbeIter<'_> {
    type Item = RowId;

    fn next(&mut self) -> Option<RowId> {
        loop {
            if let Some((l, ids)) = &mut self.inner {
                let (cs, ks) = (&self.cs[..self.n], &self.k[..self.n]);
                let span = self.chain.span(l);
                for id in ids.by_ref() {
                    self.chain.touch(1);
                    if span.contains(&id) && l.cmp_key(id, cs, ks) == Ordering::Equal && !self.chain.killed_above(l, id) {
                        return Some(RowId::new(id));
                    }
                }
                self.inner = None;
            }
            let l = self.chain.layer(self.layer?);
            self.layer = l.below().map(|(b, _)| b);
            let (cs, ks) = (&self.cs[..self.n], &self.k[..self.n]);
            if !l.may_hold(cs, ks) {
                continue;
            }
            let chain = self.chain;
            let ids = if let Some(r) = l.prefix_range_counted(cs, ks, chain.span(&l), &|| chain.touch(1)) {
                Candidates::Range(r)
            } else if let Some(p) = l.order_range(cs, ks) {
                Candidates::Listed(p.iter())
            } else if let Some((perm, at)) = l.perm_range(cs, ks) {
                Candidates::Perm { perm, at, start: l.start() }
            } else if let Some(x) = l.rows.find_index(cs, l.n() as u32) {
                Candidates::Listed(l.rows.group(x, key_hash(ks)).iter())
            } else {
                Candidates::Range(self.chain.span(&l))
            };
            self.inner = Some((l, ids));
        }
    }
}

/// Layer k's cursor into the kills of layer j above it: the partition it reads (of k's last row asked), that
/// partition's kills in layer j (up to `to` of j's kills by partition), and its place in them.
#[derive(Clone, Copy)]
struct KillCursor {
    set: bool,
    p: i64,
    to: u32,
    at: u32,
}

/// The cursors a scan keeps, one a pair of layers (j above k): `MAX_LAYERS` choose 2.
const PAIRS: usize = MAX_LAYERS * (MAX_LAYERS - 1) / 2;

/// A trace's state's live rows in canonical order: a merge of its layers' runs (each layer lies in the order), as many
/// as it has. A layer's ids come in ascending order, its partitions too (the canonical order leads with the
/// partition), so whether one is killed above is a merge too: a cursor a layer above into the kills of the row's
/// partition (`gone(part, id)`, ascending within a partition), each moving forward only, and placed again by one
/// search when the layer's rows reach the next partition. A scan reads its rows and the kills above them once each.
pub struct Scan<'a> {
    chain: State<'a>,
    /// A cursor a layer, newest first: its handle, its next id and its end.
    heads: [Option<(Layer<'a>, u32, u32)>; MAX_LAYERS],
    n: usize,
    at: [KillCursor; PAIRS],
}

/// Pair `(k, j)`'s cursor's place, `j < k`.
fn pair(k: usize, j: usize) -> usize {
    k * (k - 1) / 2 + j
}

impl<'a> Scan<'a> {
    fn new(chain: &State<'a>) -> Scan<'a> {
        let mut s = Scan { chain: *chain, heads: [None; MAX_LAYERS], n: 0, at: [KillCursor { set: false, p: 0, to: 0, at: 0 }; PAIRS] };
        for (_, l) in chain.layers() {
            let r = chain.span(&l);
            s.heads[s.n] = Some((l, r.start, r.end));
            s.n += 1;
        }
        s
    }

    fn head(&self, k: usize) -> (Layer<'a>, u32, u32) {
        self.heads[k].expect("a layer's cursor")
    }

    /// Whether layer `k`'s row `id` is killed above it, its cursors moved to `id` (placed again, by a search, in the
    /// kills of `id`'s partition when it is not the last row's).
    fn killed(&mut self, k: usize, id: u32) -> bool {
        let p = self.chain.part.unwrap_or_else(|| self.head(k).0.part_of(id));
        let mut gone = false;
        for j in 0..k {
            let x = pair(k, j);
            let all = self.head(j).0.kills();
            if !self.at[x].set || self.at[x].p != p {
                let ks = self.head(j).0.part_kills(p);
                // `part_kills` is a slice of the layer's kills by partition: its place there.
                let from = if ks.is_empty() { 0 } else { (ks.as_ptr() as usize - all.as_ptr() as usize) / size_of::<u32>() };
                let chain = &self.chain;
                let at = partition_point(ks.len(), |i| {
                    chain.touch(1);
                    ks[i] < id
                });
                self.at[x] = KillCursor { set: true, p, to: (from + ks.len()) as u32, at: (from + at) as u32 };
            }
            let c = &mut self.at[x];
            while c.at < c.to && all[c.at as usize] < id {
                self.chain.touch(1);
                c.at += 1;
            }
            gone |= c.at < c.to && all[c.at as usize] == id;
        }
        gone
    }
}

impl Iterator for Scan<'_> {
    type Item = RowId;

    fn next(&mut self) -> Option<RowId> {
        loop {
            let mut best: Option<usize> = None;
            for k in 0..self.n {
                let (lk, ik, hi) = self.head(k);
                if ik >= hi {
                    continue;
                }
                best = match best {
                    Some(b) => {
                        let (lb, ib, _) = self.head(b);
                        let o = lb.head.shape.order().iter().map(|&c| lk.get(ik, c as usize).cmp(&lb.get(ib, c as usize))).find(|o| o.is_ne());
                        if o == Some(Ordering::Less) { Some(k) } else { Some(b) }
                    }
                    None => Some(k),
                };
            }
            let k = best?;
            let h = self.heads[k].as_mut().expect("a layer's cursor");
            let id = h.1;
            h.1 += 1;
            self.chain.touch(1);
            if !self.killed(k, id) {
                return Some(RowId::new(id));
            }
        }
    }
}
