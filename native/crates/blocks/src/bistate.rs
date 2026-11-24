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
//! Both states are one type: they read the same rows, so they are read the same way. (Step 2 makes them two
//! frontiers of one trace.)
//!
//! `Read` is what a state gives. `Bistate` reads the change. `reference` is the plain definition (the rows after,
//! less the rows before, compared by value): the oracle every bistate is tested against, built for tests
//! (`test-hooks`) only.
//!
//! Cost: `added`, `gone`, `counts` and `is_identity` read the two states' liveness words, so they cost
//! `after.end() / 64` word reads, not the change's size. A state whose layer records its own kills (a delta's
//! `killed` rows) will read them in O(change) (0678, step 3).

pub use crate::ids::{ColumnIndex, RowId, WordIndex};
use crate::relation::View;
#[cfg(any(test, feature = "test-hooks"))]
use std::collections::BTreeMap;

/// A relation at one state, read where it lies.
pub trait Read {
    /// Its columns.
    fn arity(&self) -> usize;
    /// The id past its last: its ids, live or not, are `0 .. end`.
    fn end(&self) -> RowId;
    /// The liveness of the rows of word `w`: bit `i` set when row `w.first() + i` is live. Rows at or past `end`
    /// read as not live.
    fn live_word(&self, w: WordIndex) -> u64;
    /// Row `id`'s value in column `c`.
    fn get(&self, id: RowId, c: ColumnIndex) -> i64;

    #[inline]
    fn live(&self, id: RowId) -> bool {
        self.live_word(id.word()) >> id.bit() & 1 != 0
    }

    /// Its live ids, ascending.
    fn live_ids(&self) -> Bits<impl Fn(WordIndex) -> u64 + '_>
    where
        Self: Sized,
    {
        Bits::new(self.end(), move |w| self.live_word(w))
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

/// One relation's rows before and after: `before` `None` for a cold run (nothing before).
pub struct Bistate<S> {
    before: Option<S>,
    after: S,
}

impl<S: Read> Bistate<S> {
    /// A cold run's: nothing before, every live row of `after` added.
    pub fn cold(after: S) -> Self {
        Bistate { before: None, after }
    }

    /// `before` and `after` are states of the same rows: ids below `before.end()` are the same rows in both.
    pub fn new(before: S, after: S) -> Self {
        assert_eq!(before.arity(), after.arity(), "two states of one relation");
        assert!(before.end() <= after.end(), "a later state keeps every id of the earlier one");
        Bistate { before: Some(before), after }
    }

    pub fn before(&self) -> Option<&S> {
        self.before.as_ref()
    }

    pub fn after(&self) -> &S {
        &self.after
    }

    #[inline]
    fn before_word(&self, w: WordIndex) -> u64 {
        self.before.as_ref().map_or(0, |b| b.live_word(w))
    }

    fn before_end(&self) -> RowId {
        self.before.as_ref().map_or(RowId::ZERO, Read::end)
    }

    /// The ids live after and not before, ascending: rows added since, and rows of before brought back.
    pub fn added(&self) -> Bits<impl Fn(WordIndex) -> u64 + '_> {
        Bits::new(self.after.end(), move |w| self.after.live_word(w) & !self.before_word(w))
    }

    /// The ids live before and not after, ascending: rows killed since.
    pub fn gone(&self) -> Bits<impl Fn(WordIndex) -> u64 + '_> {
        Bits::new(self.before_end(), move |w| self.before_word(w) & !self.after.live_word(w))
    }

    /// Rows gone and rows added, counted.
    pub fn counts(&self) -> (usize, usize) {
        let (mut gone, mut added) = (0, 0);
        for w in WordIndex::upto(self.after.end()) {
            let (b, a) = (self.before_word(w), self.after.live_word(w));
            gone += (b & !a).count_ones() as usize;
            added += (a & !b).count_ones() as usize;
        }
        (gone, added)
    }

    /// Whether nothing changed: no row gone or added.
    pub fn is_identity(&self) -> bool {
        WordIndex::upto(self.after.end()).all(|w| self.before_word(w) == self.after.live_word(w))
    }
}

/// A row's values, for the oracle's comparisons.
#[cfg(any(test, feature = "test-hooks"))]
fn values<R: Read + ?Sized>(r: &R, id: RowId) -> Vec<i64> {
    ColumnIndex::all(r.arity()).map(|c| r.get(id, c)).collect()
}

/// A state's live rows as a multiset of values: what `reference` compares.
#[cfg(any(test, feature = "test-hooks"))]
pub fn rows_of<R: Read + ?Sized>(r: &R) -> BTreeMap<Vec<i64>, usize> {
    let mut out = BTreeMap::new();
    for id in Bits::new(r.end(), |w| r.live_word(w)) {
        *out.entry(values(r, id)).or_insert(0) += 1;
    }
    out
}

/// The least change, by definition: the rows before less the rows after (`gone`), and the rows after less the rows
/// before (`added`), compared by value, each with its count: the reference a delta operator's output is compared
/// with ("run on both, diff").
#[cfg(any(test, feature = "test-hooks"))]
pub fn reference(before: Option<&dyn Read>, after: &dyn Read) -> (BTreeMap<Vec<i64>, usize>, BTreeMap<Vec<i64>, usize>) {
    let b = before.map(rows_of).unwrap_or_default();
    let a = rows_of(after);
    let minus = |x: &BTreeMap<Vec<i64>, usize>, y: &BTreeMap<Vec<i64>, usize>| {
        x.iter().filter_map(|(k, &n)| n.checked_sub(y.get(k).copied().unwrap_or(0)).filter(|&d| d > 0).map(|d| (k.clone(), d))).collect()
    };
    (minus(&b, &a), minus(&a, &b))
}

/// Asserts law L1 at the level of one relation (decision 0678): the rows before, less the rows `gone` names, plus
/// the rows `added` names, are the rows after, as values with their counts; each gone id was live before and is not
/// after, each added id is live after and was not before. The change need not be the least one (`reference`'s): a
/// row killed and derived again is gone and added (a level reset, 0364 R4).
#[cfg(any(test, feature = "test-hooks"))]
pub fn check<S: Read>(s: &Bistate<S>) {
    let collect = |ids: &mut dyn Iterator<Item = RowId>, r: &S| {
        let mut out = BTreeMap::new();
        for id in ids {
            *out.entry(values(r, id)).or_insert(0usize) += 1;
        }
        out
    };
    for id in s.added() {
        assert!(s.after.live(id) && !s.before.as_ref().is_some_and(|b| b.live(id)), "added id {id} is live after only");
    }
    let mut rows = BTreeMap::new();
    if let Some(b) = &s.before {
        for id in s.gone() {
            assert!(b.live(id) && !s.after.live(id), "gone id {id} is live before only");
        }
        rows = rows_of(b);
        for (row, n) in collect(&mut s.gone(), b) {
            let have = rows.get_mut(&row).expect("a gone row is a row before");
            *have -= n;
            if *have == 0 {
                rows.remove(&row);
            }
        }
    }
    let added = collect(&mut s.added(), &s.after);
    for (row, n) in &added {
        *rows.entry(row.clone()).or_insert(0) += n;
    }
    assert_eq!(rows, rows_of(&s.after), "before, less the rows gone, plus the rows added, is after");
    let gone_n = s.before.as_ref().map_or(0, |_| s.gone().count());
    assert_eq!(s.counts(), (gone_n, added.values().sum()), "its counts are its ids'");
    assert_eq!(s.is_identity(), gone_n == 0 && added.is_empty(), "identity when no id changed");
}

/// A relation read alone: its own rows, less its dead bits.
impl Read for View<'_> {
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

/// A kept relation: its packed rows, every one live.
impl Read for crate::kept::Kept<'_> {
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
}

/// A run of rows at one state, read through a function: its ids `0 .. end`, less those `dead` marks (bit `id`). The
/// form a relation that keeps its rows and dead bits in place has at its start and now (setfix's `InRel`).
pub struct Marked<'a, G: Fn(RowId, ColumnIndex) -> i64> {
    pub arity: usize,
    pub end: RowId,
    pub dead: &'a [u64],
    pub get: G,
}

impl<G: Fn(RowId, ColumnIndex) -> i64> Read for Marked<'_, G> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Block, kind};
    use crate::relation::{self, Relation, RelW, Spec};
    use std::sync::Mutex;

    fn spec() -> Spec {
        Spec { widths: vec![8, 8], order: vec![0], indexes: vec![vec![0]], min_log: crate::rows::MIN_LOG }
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
    /// already) is added, and the two are the reference's.
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
            let (gone, added) = reference(s.before().map(|b| b as &dyn Read), s.after());
            assert_eq!((gone.len(), added.len()), (2, 2));
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
    /// both, rows killed since are gone, rows past the start's end added, a row brought back added.
    #[test]
    fn marked_states_read_their_change_by_words() {
        let n = 200u32;
        let get = |id: RowId, c: ColumnIndex| i64::from(id.get()) * 10 + c.get() as i64;
        let mut dead0 = vec![0u64; 2];
        dead0[0] |= 1 << 5;
        dead0[1] |= 1 << 3; // id 67, dead at the start and brought back
        let mut dead = dead0.clone();
        dead[1] &= !(1 << 3);
        dead[0] |= 1 << 9;
        dead[1] |= 1 << 20; // id 84
        let mut now = dead.clone();
        now.resize(4, 0);
        now[2] |= 1 << 1; // id 129, added and dead: in neither state
        let before = Marked { arity: 2, end: RowId::new(100), dead: &dead0, get };
        let after = Marked { arity: 2, end: RowId::new(n), dead: &now, get };
        let s = Bistate::new(before, after);
        check(&s);
        assert_eq!(s.gone().map(RowId::get).collect::<Vec<_>>(), vec![9, 84]);
        let added: Vec<u32> = s.added().map(RowId::get).collect();
        assert_eq!(added[0], 67, "a row brought back is added");
        assert_eq!(added[1..], (100..n).filter(|&id| id != 129).collect::<Vec<_>>()[..]);
    }

    /// Word boundaries: ends of 0, 1, 63, 64, 65 and 128 ids.
    #[test]
    fn ends_at_word_boundaries() {
        for end in [0u32, 1, 63, 64, 65, 128] {
            let state = || Marked { arity: 1, end: RowId::new(end), dead: &[], get: |id: RowId, _| i64::from(id.get()) };
            assert_eq!(state().live_ids().count(), end as usize, "{end}");
            check(&Bistate::cold(state()));
        }
    }
}
