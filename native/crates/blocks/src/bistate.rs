//! A bistate: one relation's rows at two states, before and after, and so the change between them (decision 0678).
//!
//! Both states are reads of the same rows where they lie (decision 0353). The ids `..before.end()` mean the same
//! rows in both, and the after state's ids past them are rows added since. Nothing is built: the change is read
//! from the two states' liveness, a 64-bit word of ids at a time.
//!
//! Every form the engine keeps a change in is such a pair:
//!
//! - a base and its delta (`relation::View`): before is the base alone, after the view of both;
//! - a relation started from its kept rows (setfix's `InRel` version, decision 0364 stage 2): before is its first
//!   rows with the dead bits they had at the start, after is all its rows with its dead bits now;
//! - a cold run: no before, every live row of after added.
//!
//! Both states are one type: they read the same rows, so they are read the same way.
//!
//! Nothing reads more than two states (the user, 2026-11-24): one version and one delta of it, in a linear history
//! with no forks. A row of the before state is, across the change, always alive, alive then dead (gone), dead then
//! alive (resurrected rather than inserted again: added), or always dead. A row past the before state's end is new:
//! alive (added) or dead.
//!
//! `Read` is what a state gives. `Bistate` reads the change. A row is no value of its own: it is a `RowId` into
//! its relation's block, and its cells are read where they lie (`Read::get`). `check` is the law a bistate keeps,
//! read in place too, built for tests (`test-hooks`) only.
//!
//! Cost: a state that records its change since its version (`Read::since`: a frozen delta's `changed` ids, an
//! `InRel`'s kills since its start) is read in the change's size: `added`, `gone`, `counts` and `is_identity` read
//! the ids it names and its new rows' words, never the version's liveness (0678 step 4). A state that records none
//! is read a liveness word at a time over every id: `after.end() / 64` words.

pub use crate::ids::{ColumnIndex, RowId, WordIndex};
use crate::relation::View;

/// One column of a state, resolved once (`Read::column`): a row's value is the column's own read, with no lookup of
/// the column again.
pub trait ColumnRead {
    fn get(&self, id: RowId) -> i64;
}

/// What a state records of its change since its version (decision 0678 step 4): the version's end, and the
/// version's ids whose liveness may differ now, ascending, each once (a superset is allowed: each is checked). Ids
/// from `end` on are rows new since.
#[derive(Clone, Copy, Debug)]
pub struct Since<'s> {
    pub end: RowId,
    pub flips: &'s [u32],
}

/// A relation at one state, read where it lies.
///
/// Three grains of read: a cell (`get`), a column resolved once and read for many rows (`column`), and a chunk of
/// rows for a set of columns, a column at a time (`fill`). Readers use the last two; `get` is what they are built
/// from by default, and each state gives its own faster form.
pub trait Read {
    /// A column of it resolved for reads.
    type Column<'s>: ColumnRead
    where
        Self: 's;

    /// Its columns.
    fn arity(&self) -> usize;
    /// The id past its last: its ids, live or not, are `0 .. end`.
    fn end(&self) -> RowId;
    /// The liveness of the rows of word `w`: bit `i` set when row `w.first() + i` is live. Rows at or past `end`
    /// read as not live.
    fn live_word(&self, w: WordIndex) -> u64;
    /// Row `id`'s value in column `c`.
    fn get(&self, id: RowId, c: ColumnIndex) -> i64;
    /// Column `c`, resolved once.
    fn column(&self, c: ColumnIndex) -> Self::Column<'_>;

    /// Rows `from .. from + n` of columns `cols`, into `out` (`n` = `out.len() / cols.len()`), a column at a time:
    /// `out[j * n + i]` is row `from + i`'s value in `cols[j]`. Dead rows are filled too (`live` says which). Rows
    /// past `end` are refused. `out` is a chunk the reader reuses as it goes (a cache line's worth to `FILL`'s), not
    /// a second store of the relation.
    fn fill(&self, cols: &[ColumnIndex], from: RowId, out: &mut [i64]) {
        let n = fill_rows(cols, from, out, self.end());
        for (j, &c) in cols.iter().enumerate() {
            let col = self.column(c);
            for (i, o) in out[j * n..(j + 1) * n].iter_mut().enumerate() {
                *o = col.get(RowId::new(from.get() + i as u32));
            }
        }
    }

    #[inline]
    fn live(&self, id: RowId) -> bool {
        self.live_word(id.word()) >> id.bit() & 1 != 0
    }

    /// Its change since its version, where it records one: none by default.
    fn since(&self) -> Option<Since<'_>> {
        None
    }

    /// The ids below `end` whose liveness may differ from the state that ended at `end`, ascending, each once: its
    /// recorded change, over as many of its own versions as lie between (a trace's layers above `end`, however
    /// many). `None` when it records none: the change is then read word by word.
    fn flips_from(&self, end: RowId) -> Option<Flips<'_>> {
        self.since().filter(|s| s.end == end).map(|s| Flips::one(s.flips))
    }

    /// Its recorded change since `before` (an earlier state of the same rows; `None`: the empty state): by default its
    /// change since the state that ended where `before` ends (`flips_from`). A trace's state names the layers above
    /// `before` itself, since two states can end at one id (a layer that only kills).
    fn flips_since(&self, before: Option<&Self>) -> Option<Flips<'_>> {
        self.flips_from(before.map_or(RowId::ZERO, Read::end))
    }

    /// The ids from `before`'s end on that may be live, as ranges, ascending and apart (a trace's layers' own rows
    /// above `before`, a partition's alone when restricted): only these words are read for rows added. `None`: every
    /// id from `before`'s end to its own.
    fn new_ranges(&self, _before: Option<&Self>) -> Option<Vec<(RowId, RowId)>> {
        None
    }

    /// The rows gone and added since `before`, when it keeps them counted: a layer's counts.
    fn exact_counts_since(&self, _before: Option<&Self>) -> Option<(usize, usize)> {
        None
    }

    /// Its live rows in its canonical order (P§12): by default, its ids' order.
    fn scan(&self) -> impl Iterator<Item = RowId> + '_
    where
        Self: Sized,
    {
        self.live_ids()
    }

    /// Its live rows whose columns `cols` hold `key`, by its indexes where it has them: by default, a scan. (P§12's
    /// `probe`; `table::Table::probe` is the same read given to a callback.)
    fn matching<'s>(&'s self, cols: &'s [ColumnIndex], key: &'s [i64]) -> impl Iterator<Item = RowId> + 's
    where
        Self: Sized,
    {
        self.live_ids().filter(move |&id| cols.iter().zip(key).all(|(&c, &k)| self.get(id, c) == k))
    }

    /// The column its rows are partitioned by: none by default.
    fn partition(&self) -> Option<ColumnIndex> {
        None
    }

    /// Partition `p`'s live rows: by default, those whose partition column holds `p` (none without one).
    fn part(&self, p: i64) -> impl Iterator<Item = RowId> + '_
    where
        Self: Sized,
    {
        let c = self.partition();
        self.live_ids().filter(move |&id| c.is_some_and(|c| self.get(id, c) == p))
    }

    /// Column `c`'s statistics: by default, read from every live row (`stats_of`).
    fn stats(&self, c: ColumnIndex) -> ColStats
    where
        Self: Sized,
    {
        stats_of(self, c)
    }

    /// Its live ids, ascending.
    fn live_ids(&self) -> Bits<impl Fn(WordIndex) -> u64 + '_>
    where
        Self: Sized,
    {
        Bits::new(self.end(), move |w| self.live_word(w))
    }
}

/// The rows a `fill` of `out` for `cols` from `from` reads, checked against the state's `end`.
#[inline]
pub fn fill_rows(cols: &[ColumnIndex], from: RowId, out: &[i64], end: RowId) -> usize {
    if cols.is_empty() {
        return 0;
    }
    let n = out.len() / cols.len();
    assert_eq!(n * cols.len(), out.len(), "a fill's buffer holds whole rows of its columns");
    assert!(u64::from(from.get()) + n as u64 <= u64::from(end.get()), "a fill within the state's rows");
    n
}

/// A column read a cell at a time through its state's `get`: the default column of a state with no faster one.
pub struct Cells<'s, R: ?Sized> {
    state: &'s R,
    c: ColumnIndex,
}

impl<'s, R: ?Sized> Cells<'s, R> {
    pub fn new(state: &'s R, c: ColumnIndex) -> Self {
        Cells { state, c }
    }
}

impl<R: Read + ?Sized> ColumnRead for Cells<'_, R> {
    #[inline]
    fn get(&self, id: RowId) -> i64 {
        self.state.get(id, self.c)
    }
}

/// The bits of word `w` whose rows lie below `end`.
#[inline]
pub fn below(end: RowId, w: WordIndex) -> u64 {
    let (lo, end) = (u64::from(w.first().get()), u64::from(end.get()));
    if lo >= end {
        0
    } else if end - lo >= 64 {
        u64::MAX
    } else {
        (1u64 << (end - lo)) - 1
    }
}

/// The rows whose bits a word function sets, ascending, over `0 .. end`.
pub struct Bits<F: Fn(WordIndex) -> u64> {
    word: F,
    words: std::ops::Range<u32>,
    at: WordIndex,
    cur: u64,
}

impl<F: Fn(WordIndex) -> u64> Bits<F> {
    fn new(end: RowId, word: F) -> Self {
        Bits { word, words: 0..end.get().div_ceil(64), at: WordIndex::default(), cur: 0 }
    }
}

/// The rows from `from` to `end` whose bits a word function sets, ascending: only the words that hold them read.
fn bits_from(from: RowId, end: RowId, word: impl Fn(WordIndex) -> u64) -> Bits<impl Fn(WordIndex) -> u64> {
    let first = from.word();
    let words = first.get() as u32..end.get().div_ceil(64).max(first.get() as u32);
    Bits { word: move |w: WordIndex| word(w) & !below(from, w) & below(end, w), words, at: WordIndex::default(), cur: 0 }
}

/// One of two iterators of ids.
enum Either<A, B> {
    L(A),
    R(B),
}

impl<A: Iterator<Item = RowId>, B: Iterator<Item = RowId>> Iterator for Either<A, B> {
    type Item = RowId;

    #[inline]
    fn next(&mut self) -> Option<RowId> {
        match self {
            Either::L(a) => a.next(),
            Either::R(b) => b.next(),
        }
    }
}

impl<F: Fn(WordIndex) -> u64> Iterator for Bits<F> {
    type Item = RowId;

    #[inline]
    fn next(&mut self) -> Option<RowId> {
        while self.cur == 0 {
            self.at = WordIndex::new(self.words.next()?);
            self.cur = (self.word)(self.at);
        }
        let id = RowId::new(self.at.first().get() + self.cur.trailing_zeros());
        self.cur &= self.cur - 1;
        Some(id)
    }
}

/// A column's statistics (P§12 `stats`; P§11.2: "per column min, max and counts"): its live rows' bounds (a trace's
/// are an estimate: the bounds of every row added, gone ones included), and its `blocks::sketch` reading (rows,
/// distinct values, the sum of each value's rows squared).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColStats {
    pub min: i64,
    pub max: i64,
    pub rows: u64,
    pub ndv: u64,
    pub sq: u64,
}

/// Column `c`'s statistics read from every live row of `state`: its bounds, and a sketch of its values
/// (`blocks::sketch`, the one sketch), made for this read and let go.
pub fn stats_of<R: Read>(state: &R, c: ColumnIndex) -> ColStats {
    let mut s = crate::sketch::Sketch::new_in(crate::layer::sketch_room());
    let (mut min, mut max) = (i64::MAX, i64::MIN);
    for id in state.live_ids() {
        let v = state.get(id, c);
        s.add(v);
        min = min.min(v);
        max = max.max(v);
    }
    let r = s.reading();
    ColStats { min, max, rows: r.rows, ndv: r.ndv, sq: r.sq }
}

/// A state's recorded change over several of its versions (`Read::flips_from`): each version's flipped ids, merged
/// ascending, each once, over as many versions as it has. The ids are read where they lie; it holds only a handle and
/// a cursor a version.
#[derive(Clone, Default)]
pub struct Flips<'s> {
    // copy: not block data (a handle and a cursor a version)
    runs: Vec<(&'s [u32], u32)>,
    last: Option<u32>,
}

impl<'s> Flips<'s> {
    pub fn one(run: &'s [u32]) -> Flips<'s> {
        let mut f = Flips::default();
        f.push(run);
        f
    }

    /// Adds one version's flips.
    pub fn push(&mut self, run: &'s [u32]) {
        self.runs.push((run, 0));
    }

    /// Whether no id flipped.
    pub fn is_empty(&self) -> bool {
        self.runs.iter().all(|(r, at)| r.len() == *at as usize)
    }
}

impl Iterator for Flips<'_> {
    type Item = RowId;

    fn next(&mut self) -> Option<RowId> {
        loop {
            let k = (0..self.runs.len()).filter(|&k| (self.runs[k].1 as usize) < self.runs[k].0.len()).min_by_key(|&k| self.runs[k].0[self.runs[k].1 as usize])?;
            let id = self.runs[k].0[self.runs[k].1 as usize];
            self.runs[k].1 += 1;
            if self.last != Some(id) {
                self.last = Some(id);
                return Some(RowId::new(id));
            }
        }
    }
}

/// States that can be inverted, composed and restricted to a partition (P§1): a trace's (`layer::Chain`).
pub trait States: Read + Copy {
    /// The empty state of the same relation.
    fn empty(&self) -> Self;
    /// Whether two states are the same state.
    fn same(&self, other: &Self) -> bool;
    /// Partition `p`'s rows alone, by the partition's index.
    fn restrict(&self, p: i64) -> Self;
    /// Gives `out` each partition changed since `before` (`None`: the empty state), by partition digests, in the
    /// change's size: false when it cannot (the change is then not recorded by partition).
    fn changed_parts_since(&self, before: Option<&Self>, out: &mut dyn FnMut(i64)) -> bool;
}

/// One relation's rows at two states (P§1): `lo` the earlier, `hi` the later, of the same rows; `inverted` when the
/// change runs from `hi` back to `lo` (`invert`), so that before is `hi` and after `lo`. `lo` `None`: a cold run.
pub struct Bistate<S> {
    lo: Option<S>,
    hi: S,
    inverted: bool,
}

impl<S: Read> Bistate<S> {
    /// A cold run's: nothing before, every live row of `after` added.
    pub fn cold(after: S) -> Self {
        Bistate { lo: None, hi: after, inverted: false }
    }

    /// `before` and `after` are states of the same rows: ids below `before.end()` are the same rows in both.
    pub fn new(before: S, after: S) -> Self {
        assert_eq!(before.arity(), after.arity(), "two states of one relation");
        assert!(before.end() <= after.end(), "a later state keeps every id of the earlier one");
        Bistate { lo: Some(before), hi: after, inverted: false }
    }

    pub fn before(&self) -> Option<&S> {
        if self.inverted { Some(&self.hi) } else { self.lo.as_ref() }
    }

    pub fn after(&self) -> &S {
        if self.inverted { self.lo.as_ref().expect("an inverted bistate holds its after state") } else { &self.hi }
    }

    /// Whether it runs from its later state back to its earlier one.
    pub fn is_inverted(&self) -> bool {
        self.inverted
    }

    #[inline]
    fn lo_word(&self, w: WordIndex) -> u64 {
        self.lo.as_ref().map_or(0, |b| b.live_word(w))
    }

    fn lo_end(&self) -> RowId {
        self.lo.as_ref().map_or(RowId::ZERO, Read::end)
    }

    /// The later state's record of its change since the earlier one, when it keeps one.
    fn listed(&self) -> Option<(Option<&S>, Flips<'_>)> {
        let flips = self.hi.flips_since(self.lo.as_ref())?;
        Some((self.lo.as_ref(), flips))
    }

    /// The ids past `lo`'s end live in `hi`, ascending: only the words of the ranges `hi` names, when it names them.
    fn new_ids(&self) -> impl Iterator<Item = RowId> + '_ {
        let hi = &self.hi;
        let ranges = hi.new_ranges(self.lo.as_ref()).unwrap_or_else(|| vec![(self.lo_end(), hi.end())]);
        ranges.into_iter().flat_map(move |(a, b)| bits_from(a, b, move |w| hi.live_word(w)))
    }

    /// The ids live in `hi` and not `lo`, ascending.
    fn rise(&self) -> impl Iterator<Item = RowId> + '_ {
        match self.listed() {
            Some((lo, flips)) => {
                let hi = &self.hi;
                let back = flips.filter(move |&id| !lo.is_some_and(|l| l.live(id)) && hi.live(id));
                Either::L(back.chain(self.new_ids()))
            }
            None => Either::R(Bits::new(self.hi.end(), move |w| self.hi.live_word(w) & !self.lo_word(w))),
        }
    }

    /// The ids live in `lo` and not `hi`, ascending.
    fn fall(&self) -> impl Iterator<Item = RowId> + '_ {
        match self.listed() {
            Some((lo, flips)) => {
                let hi = &self.hi;
                Either::L(flips.filter(move |&id| lo.is_some_and(|l| l.live(id)) && !hi.live(id)))
            }
            None => Either::R(Bits::new(self.lo_end(), move |w| self.lo_word(w) & !self.hi.live_word(w))),
        }
    }

    /// The ids live after and not before, ascending: rows added since, and rows of before resurrected.
    pub fn added(&self) -> impl Iterator<Item = RowId> + '_ {
        if self.inverted { Either::R(self.fall()) } else { Either::L(self.rise()) }
    }

    /// The ids live before and not after, ascending: rows killed since.
    pub fn gone(&self) -> impl Iterator<Item = RowId> + '_ {
        if self.inverted { Either::L(self.rise()) } else { Either::R(self.fall()) }
    }

    /// `hi`'s rows that fell and rose since `lo`, counted.
    fn raw_counts(&self) -> (usize, usize) {
        if let Some(c) = self.hi.exact_counts_since(self.lo.as_ref()) {
            return c;
        }
        if let Some((lo, flips)) = self.listed() {
            let (mut fell, mut rose) = (0, 0);
            for id in flips {
                let (b, a) = (lo.is_some_and(|l| l.live(id)), self.hi.live(id));
                fell += usize::from(b && !a);
                rose += usize::from(a && !b);
            }
            rose += self.new_ids().count();
            return (fell, rose);
        }
        let (mut fell, mut rose) = (0, 0);
        for w in WordIndex::upto(self.hi.end()) {
            let (b, a) = (self.lo_word(w), self.hi.live_word(w));
            fell += (b & !a).count_ones() as usize;
            rose += (a & !b).count_ones() as usize;
        }
        (fell, rose)
    }

    /// Rows gone and rows added, counted.
    pub fn counts(&self) -> (usize, usize) {
        let (fell, rose) = self.raw_counts();
        if self.inverted { (rose, fell) } else { (fell, rose) }
    }

    /// Whether nothing changed: no row gone or added. A layer's counts say so in O(1) (P§1); a change over several
    /// layers is read in its size; one not recorded, word by word.
    pub fn is_identity(&self) -> bool {
        if let Some(c) = self.hi.exact_counts_since(self.lo.as_ref()) {
            return c == (0, 0);
        }
        if let Some((_, flips)) = self.listed() {
            if flips.is_empty() && self.hi.end() == self.lo_end() {
                return true;
            }
            return self.raw_counts() == (0, 0);
        }
        WordIndex::upto(self.hi.end()).all(|w| self.lo_word(w) == self.hi.live_word(w))
    }
}

impl<S: States> Bistate<S> {
    /// The change run backward (P§1): after becomes before, so gone and added swap roles. O(1): nothing is read.
    pub fn invert(self) -> Self {
        let lo = self.lo.unwrap_or_else(|| self.hi.empty());
        Bistate { lo: Some(lo), hi: self.hi, inverted: !self.inverted }
    }

    /// This change, then `next` (P§1): `next` starts where this ends. Two edits in a burst as one; an edit and its
    /// undo compose to the identity. O(1): the result reads both states where they lie.
    pub fn compose(self, next: Bistate<S>) -> Result<Self, String> {
        let empty = self.hi.empty();
        let (x, mid) = (self.before().copied().unwrap_or(empty), *self.after());
        let (mid2, z) = (next.before().copied().unwrap_or(empty), *next.after());
        if !mid.same(&mid2) {
            return Err("a composed change starts where the first one ends".into());
        }
        Ok(if x.end() <= z.end() { Bistate { lo: Some(x), hi: z, inverted: false } } else { Bistate { lo: Some(z), hi: x, inverted: true } })
    }

    /// The bistate of partition `p` alone (P§1): each state read through the partition's index, nothing built.
    pub fn restrict(&self, p: i64) -> Self {
        Bistate { lo: self.lo.map(|l| l.restrict(p)), hi: self.hi.restrict(p), inverted: self.inverted }
    }

    /// The partitions whose restriction is not the identity (P§1), by the partitions' digests, in the change's size
    /// where the states record it by partition; otherwise read from the rows gone and added.
    pub fn changed_parts(&self, out: &mut dyn FnMut(i64)) {
        if self.hi.changed_parts_since(self.lo.as_ref(), out) {
            return;
        }
        let c = self.hi.partition().expect("a partitioned relation's changed partitions");
        let mut last = None;
        let mut keys: Vec<i64> = self.rise().map(|id| self.hi.get(id, c)).chain(self.fall().map(|id| self.lo.as_ref().expect("a gone row's state").get(id, c))).collect(); // copy: derived (the changed keys, the answer)
        keys.sort_unstable();
        for k in keys {
            if last != Some(k) {
                out(k);
                last = Some(k);
            }
        }
    }
}

/// Asserts law L1 for one relation (decision 0678), reading both states where they lie: nothing is copied. Every id
/// below the after state's end is gone exactly when it is live before only, and added exactly when it is live after
/// only; a row live in both has the same values in both (the two states are the same rows). So the rows before, less
/// those gone, plus those added, are the rows after. The change need not be the least one (`is_least`): a row killed
/// and derived again is gone and added (a level reset, 0364 R4).
#[cfg(any(test, feature = "test-hooks"))]
pub fn check<S: Read>(s: &Bistate<S>) {
    let after = s.after();
    let (mut gone, mut added) = (s.gone().peekable(), s.added().peekable());
    let (mut gone_n, mut added_n) = (0usize, 0usize);
    for id in (0..s.hi.end().get()).map(RowId::new) {
        let b = s.before().is_some_and(|b| b.live(id));
        let a = after.live(id);
        let is_gone = gone.next_if_eq(&id).is_some();
        let is_added = added.next_if_eq(&id).is_some();
        assert_eq!(is_gone, b && !a, "row {id}: gone exactly when live before only");
        assert_eq!(is_added, a && !b, "row {id}: added exactly when live after only");
        if let Some(before) = s.before().filter(|_| b && a) {
            for c in ColumnIndex::all(after.arity()) {
                assert_eq!(before.get(id, c), after.get(id, c), "row {id} is the same row in both states");
            }
        }
        gone_n += usize::from(is_gone);
        added_n += usize::from(is_added);
    }
    assert!(gone.next().is_none() && added.next().is_none(), "no id past the after state's end");
    assert_eq!(s.counts(), (gone_n, added_n), "its counts are its ids'");
    assert_eq!(s.is_identity(), gone_n == 0 && added_n == 0, "identity when no id changed");
}

/// Whether the change is the least one, by value: no row gone has the values of a row added. Read in place, every
/// pair compared: for tests' small relations only.
#[cfg(any(test, feature = "test-hooks"))]
pub fn is_least<S: Read>(s: &Bistate<S>) -> bool {
    let Some(before) = s.before() else { return true };
    let cols = before.arity();
    s.gone().all(|g| s.added().all(|a| ColumnIndex::all(cols).any(|c| before.get(g, c) != s.after().get(a, c))))
}

/// A view's column: the base's and the delta's, each resolved once (`Rows::reader`).
#[derive(Clone, Copy)]
pub struct ViewColumn {
    base: Option<crate::rows::ColRead>,
    top: crate::rows::ColRead,
    base_rows: u32,
}

impl ColumnRead for ViewColumn {
    #[inline]
    fn get(&self, id: RowId) -> i64 {
        let id = id.get();
        if id < self.base_rows { self.base.expect("a delta's base").get(id) } else { self.top.get(id - self.base_rows) }
    }
}

/// A relation read alone: its own rows, less its dead bits.
impl Read for View<'_> {
    type Column<'s>
        = ViewColumn
    where
        Self: 's;

    fn arity(&self) -> usize {
        self.top.arity as usize
    }

    fn end(&self) -> RowId {
        RowId::new(View::total(self))
    }

    /// `View::live`, a word at a time: the delta's dead bits name its base's rows and its own, the base's its own.
    fn live_word(&self, w: WordIndex) -> u64 {
        let dead = |r: &crate::relation::Relation| r.dead.as_slice().get(w.get()).copied().unwrap_or(0);
        let mut live = below(Read::end(self), w) & !dead(self.top);
        if let Some(b) = self.base {
            live &= !(dead(b) & below(RowId::new(self.top.base_rows), w));
        }
        live
    }

    fn get(&self, id: RowId, c: ColumnIndex) -> i64 {
        View::get(self, id.get(), c.get())
    }

    /// A frozen delta's change: its base's ids it killed (`Relation::changed`), and its own rows after them.
    fn since(&self) -> Option<Since<'_>> {
        self.base?;
        self.top.is_frozen().then(|| Since { end: RowId::new(self.top.base_rows), flips: self.top.changed.as_slice() })
    }

    fn column(&self, c: ColumnIndex) -> ViewColumn {
        ViewColumn { base: self.base.map(|b| b.rows.reader(c.get())), top: self.top.rows.reader(c.get()), base_rows: self.top.base_rows }
    }
}

impl<'a> View<'a> {
    /// The view as a change of its base (decision 0678): before, the base alone; after, the base and its delta. A
    /// full relation's view is a cold run's.
    pub fn bistate(&self) -> Bistate<View<'a>> {
        match self.base {
            Some(b) => Bistate::new(View::full(b), *self),
            None => Bistate::cold(*self),
        }
    }
}

/// A kept relation's column: its packed parts, resolved once (`packed::ColDec`).
#[derive(Clone, Copy)]
pub struct KeptColumn<'b>(crate::packed::ColDec<'b>);

impl ColumnRead for KeptColumn<'_> {
    #[inline]
    fn get(&self, id: RowId) -> i64 {
        self.0.get(id.get() as usize)
    }
}

/// A kept relation: its packed rows, every one live.
impl<'b> Read for crate::kept::Kept<'b> {
    type Column<'s>
        = KeptColumn<'b>
    where
        Self: 's;

    fn arity(&self) -> usize {
        self.rows.arity as usize
    }

    fn end(&self) -> RowId {
        RowId::new(u32::try_from(self.rows.len()).expect("a kept relation's rows fit a row id"))
    }

    fn live_word(&self, w: WordIndex) -> u64 {
        below(Read::end(self), w)
    }

    fn get(&self, id: RowId, c: ColumnIndex) -> i64 {
        self.rows.get(id.get() as usize, c.get())
    }

    fn column(&self, c: ColumnIndex) -> KeptColumn<'b> {
        KeptColumn(self.rows.col(c.get()).dec())
    }

    /// Each column's parts decoded a run of `FILL` rows at a time (`ColDec::fill`), as `Kept::each_row` reads them.
    fn fill(&self, cols: &[ColumnIndex], from: RowId, out: &mut [i64]) {
        use crate::packed::FILL;
        let n = fill_rows(cols, from, out, Read::end(self));
        for (j, &c) in cols.iter().enumerate() {
            let dec = self.rows.col(c.get()).dec();
            let col = &mut out[j * n..(j + 1) * n];
            for at in (0..n).step_by(FILL) {
                let m = (n - at).min(FILL);
                dec.fill(from.get() as usize + at, &mut col[at..at + m]);
            }
        }
    }
}

/// A run of rows at one state, read through a function: its ids `0 .. end`, less those `dead` marks (bit `id`). The
/// form a relation that keeps its rows and dead bits in place has at its start and now (setfix's `InRel`).
pub struct Marked<'a, G: Fn(RowId, ColumnIndex) -> i64> {
    pub arity: usize,
    pub end: RowId,
    pub dead: &'a [u64],
    pub get: G,
    /// Its change since its version, where its holder records one (an `InRel`'s kills since its start).
    pub since: Option<Since<'a>>,
}

impl<G: Fn(RowId, ColumnIndex) -> i64> Read for Marked<'_, G> {
    type Column<'s>
        = Cells<'s, Self>
    where
        Self: 's;

    fn arity(&self) -> usize {
        self.arity
    }

    fn end(&self) -> RowId {
        self.end
    }

    #[inline]
    fn live_word(&self, w: WordIndex) -> u64 {
        below(self.end, w) & !self.dead.get(w.get()).copied().unwrap_or(0)
    }

    fn get(&self, id: RowId, c: ColumnIndex) -> i64 {
        (self.get)(id, c)
    }

    fn since(&self) -> Option<Since<'_>> {
        self.since
    }

    /// Its cells come through `get`, a cell at a time: it has no column of its own to resolve.
    fn column(&self, c: ColumnIndex) -> Cells<'_, Self> {
        Cells::new(self, c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Block, kind};
    use crate::relation::{self, Relation, RelW, Spec};
    use std::sync::Mutex;

    fn spec() -> Spec {
        Spec { widths: vec![8, 8], sort: Some(0), indexes: vec![vec![0]], min_log: crate::rows::MIN_LOG }
    }

    /// A base relation and its delta in one block: the base's rows `base`, then the delta's `added` and its kills
    /// of the base's rows `killed` (ids in the base's canonical order).
    fn with_delta(base: &[[i64; 2]], added: &[[i64; 2]], killed: &[u32], f: impl FnOnce(View<'_>)) {
        let block = Block::anon(kind::FACTS, 16 << 20);
        let (spec, lock) = (spec(), Mutex::new(()));
        let b = Relation::create(&block, &spec, None);
        let w = RelW { block: &block, rel: b, lock: &lock };
        for r in base {
            w.push(r);
        }
        relation::freeze(&block, b, &spec, None);
        let d = Relation::create(&block, &spec, Some(b));
        let w = RelW { block: &block, rel: d, lock: &lock };
        for r in added {
            w.push(r);
        }
        for &id in killed {
            w.kill(id);
        }
        relation::freeze(&block, d, &spec, Some(b));
        f(View { base: Some(b), top: d });
    }

    /// A row's two columns, read through `Read`.
    fn pair<R: Read>(r: &R, id: RowId) -> [i64; 2] {
        [r.get(id, ColumnIndex::new(0)), r.get(id, ColumnIndex::new(1))]
    }

    /// A base and its delta are a bistate: what the delta kills is gone, what it adds (less what the base holds
    /// already) is added, and the change is the least one.
    #[test]
    fn a_base_and_its_delta_are_a_bistate() {
        let base = [[1, 10], [2, 20], [3, 30], [4, 40]];
        with_delta(&base, &[[5, 50], [2, 20], [6, 60]], &[0, 2], |v| {
            let s = v.bistate();
            check(&s);
            let before = s.before().unwrap();
            assert_eq!(s.gone().map(|id| pair(before, id)).collect::<Vec<_>>(), vec![[1, 10], [3, 30]]);
            assert_eq!(s.added().map(|id| pair(s.after(), id)).collect::<Vec<_>>(), vec![[5, 50], [6, 60]], "a row the base holds live is not added");
            assert_eq!(s.counts(), (2, 2));
            assert!(!s.is_identity());
            // Freeze drops a delta's rows its base holds live, so its change is the least one.
            assert!(is_least(&s));
        });
    }

    /// A view's columns resolved once and its fills read what its cells read, on both sides of the base's end.
    #[test]
    fn a_views_columns_and_fills_read_what_cells_read() {
        let base: Vec<[i64; 2]> = (0..40).map(|i| [i * 2, 100 - i]).collect();
        let added: Vec<[i64; 2]> = (0..30).map(|i| [i * 2 + 1, -i]).collect();
        with_delta(&base, &added, &[3, 7], |v| {
            let cols = [ColumnIndex::new(1), ColumnIndex::new(0)];
            let end = Read::end(&v).get();
            let c = v.column(cols[0]);
            for id in (0..end).rev().map(RowId::new) {
                assert_eq!(c.get(id), Read::get(&v, id, cols[0]), "row {id}");
            }
            let (from, n) = (5u32, end as usize - 5);
            let mut out = vec![0i64; 2 * n];
            v.fill(&cols, RowId::new(from), &mut out);
            for i in 0..n {
                let id = RowId::new(from + i as u32);
                assert_eq!([out[i], out[n + i]], [Read::get(&v, id, cols[0]), Read::get(&v, id, cols[1])], "row {id}");
            }
        });
    }

    /// A frozen delta records its change: the base rows it killed, read without the base's liveness. A base row
    /// killed and pushed again is brought back, not added again: its kill is cancelled, and it is in no change.
    #[test]
    fn a_delta_records_its_kills_and_brings_back_a_row_pushed_again() {
        let base = [[1, 10], [2, 20], [3, 30], [4, 40]];
        with_delta(&base, &[[5, 50], [3, 30]], &[0, 2, 0], |v| {
            assert_eq!(v.top.changed.as_slice(), &[0], "id 2 pushed again: its kill cancelled; id 0 once");
            assert!(View::live(&v, 2), "brought back where it was");
            let s = v.bistate();
            assert!(s.listed().is_some());
            check(&s);
            assert!(is_least(&s));
            assert_eq!(s.gone().map(RowId::get).collect::<Vec<_>>(), vec![0]);
            assert_eq!(s.added().map(|id| pair(s.after(), id)).collect::<Vec<_>>(), vec![[5, 50]]);
            assert_eq!(s.counts(), (1, 1));
        });
        with_delta(&base, &[[3, 30]], &[2], |v| {
            assert!(v.top.changed.as_slice().is_empty());
            let s = v.bistate();
            check(&s);
            assert!(s.is_identity(), "killed and pushed again: no change");
        });
    }

    /// A delta that adds only rows its base holds and kills none is the identity.
    #[test]
    fn a_delta_of_nothing_new_is_the_identity() {
        with_delta(&[[1, 10], [2, 20]], &[[2, 20]], &[], |v| {
            let s = v.bistate();
            check(&s);
            assert!(s.is_identity());
            assert_eq!(s.counts(), (0, 0));
        });
    }

    /// A full relation's view is a cold run's: every live row added, nothing gone.
    #[test]
    fn a_full_relation_is_a_cold_run() {
        with_delta(&[[7, 1], [8, 2]], &[], &[], |v| {
            let s = View::full(v.base.unwrap()).bistate();
            check(&s);
            assert!(s.before().is_none());
            assert_eq!(s.counts(), (0, 2));
        });
    }

    /// Two states of one run of rows kept in place (setfix's `InRel` versions): rows dead at the start stay out of
    /// both, rows killed since are gone, rows past the start's end added, a row resurrected added.
    #[test]
    fn marked_states_read_their_change_by_words() {
        let n = 200u32;
        let get = |id: RowId, c: ColumnIndex| i64::from(id.get()) * 10 + c.get() as i64;
        let mut dead0 = vec![0u64; 2];
        dead0[0] |= 1 << 5;
        dead0[1] |= 1 << 3; // id 67, dead at the start and resurrected
        let mut dead = dead0.clone();
        dead[1] &= !(1 << 3);
        dead[0] |= 1 << 9;
        dead[1] |= 1 << 20; // id 84
        let mut now = dead.clone();
        now.resize(4, 0);
        now[2] |= 1 << 1; // id 129, added and dead: in neither state
        // Read by words, and from the ids the after state records (a superset: id 5 dead in both, id 40 live in
        // both): the same change.
        for since in [None, Some(Since { end: RowId::new(100), flips: &[5, 9, 40, 67, 84] })] {
            let before = Marked { arity: 2, end: RowId::new(100), dead: &dead0, get, since: None };
            let after = Marked { arity: 2, end: RowId::new(n), dead: &now, get, since };
            let s = Bistate::new(before, after);
            assert_eq!(s.listed().is_some(), since.is_some());
            check(&s);
            assert_eq!(s.gone().map(RowId::get).collect::<Vec<_>>(), vec![9, 84]);
            let added: Vec<u32> = s.added().map(RowId::get).collect();
            assert_eq!(added[0], 67, "a row resurrected is added");
            assert_eq!(added[1..], (100..n).filter(|&id| id != 129).collect::<Vec<_>>()[..]);
            assert_eq!(s.counts(), (2, 1 + 99));
        }
    }

    /// A state that counts its liveness reads.
    struct Counted<'c, R> {
        r: R,
        words: &'c std::cell::Cell<usize>,
    }

    impl<R: Read> Read for Counted<'_, R> {
        type Column<'s>
            = R::Column<'s>
        where
            Self: 's;

        fn arity(&self) -> usize {
            self.r.arity()
        }

        fn end(&self) -> RowId {
            self.r.end()
        }

        fn live_word(&self, w: WordIndex) -> u64 {
            self.words.set(self.words.get() + 1);
            self.r.live_word(w)
        }

        fn get(&self, id: RowId, c: ColumnIndex) -> i64 {
            self.r.get(id, c)
        }

        fn column(&self, c: ColumnIndex) -> R::Column<'_> {
            self.r.column(c)
        }

        fn since(&self) -> Option<Since<'_>> {
            self.r.since()
        }
    }

    /// A change recorded by its state costs its size: a version 64 times as large, with the same kills and rows
    /// added, takes the same liveness reads; read by words, it takes 64 times as many.
    #[test]
    fn a_recorded_change_costs_its_size() {
        let reads = |n0: u32, listed: bool| -> (usize, (usize, usize), Vec<u32>, Vec<u32>) {
            let get = |id: RowId, _: ColumnIndex| i64::from(id.get());
            let words = std::cell::Cell::new(0);
            let mut now = vec![0u64; (n0 as usize + 100).div_ceil(64)];
            for id in [3u32, 7, n0 - 1, n0 + 5] {
                now[id as usize >> 6] |= 1 << (id & 63);
            }
            let flips = [3u32, 7, n0 - 1];
            let since = listed.then_some(Since { end: RowId::new(n0), flips: &flips });
            let before = Counted { r: Marked { arity: 1, end: RowId::new(n0), dead: &[], get, since: None }, words: &words };
            let after = Counted { r: Marked { arity: 1, end: RowId::new(n0 + 100), dead: &now, get, since }, words: &words };
            let s = Bistate::new(before, after);
            words.set(0);
            let counts = s.counts();
            let gone: Vec<u32> = s.gone().map(RowId::get).collect();
            let added: Vec<u32> = s.added().map(|id| id.get() - n0).collect();
            (words.get(), counts, gone.iter().map(|&g| if g == n0 - 1 { u32::MAX } else { g }).collect(), added)
        };
        let (small, c1, g1, a1) = reads(1 << 10, true);
        let (large, c2, g2, a2) = reads(1 << 16, true);
        assert_eq!((c1, &g1, &a1), (c2, &g2, &a2), "the same change");
        assert_eq!(c1, (3, 99));
        assert_eq!(small, large, "a recorded change's reads do not grow with its version");
        let (by_words_small, ..) = reads(1 << 10, false);
        let (by_words_large, ..) = reads(1 << 16, false);
        assert!(by_words_large >= 32 * by_words_small, "{by_words_small} {by_words_large}");
    }

    /// Word boundaries: ends of 0, 1, 63, 64, 65 and 128 ids.
    #[test]
    fn ends_at_word_boundaries() {
        for end in [0u32, 1, 63, 64, 65, 128] {
            let state = || Marked { arity: 1, end: RowId::new(end), dead: &[], get: |id: RowId, _| i64::from(id.get()), since: None };
            assert_eq!(state().live_ids().count(), end as usize, "{end}");
            check(&Bistate::cold(state()));
        }
    }
}
