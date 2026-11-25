//! A fact table: one relation at one state, and the operations its readers ask of it (decision 0678 step 3).
//!
//! A table is a `Read` (its rows' cells, columns and chunks, where they lie) that also says how its rows are found:
//!
//! - **its sort column, or none** (the user, 2026-11-24: "All facts are stored sorted on zero or one column. The
//!   column can vary. This enables better join shapes.");
//! - **its runs**: the id ranges each sorted on that column. There are at most two, a version's rows and its
//!   delta's ("one version plus one delta"), so a read in the sort order merges two runs, and a probe of the sort
//!   column is a range in each;
//! - **its probes**: the live rows whose columns equal a key, by the sort column's ranges, an index of its own, or a
//!   scan, as `access` says.
//!
//! The operations every reader asks are built on these three, once, here: a read in the sort order or of a range of
//! it (`ordered`, `range`), membership (`holds`), a key's count and aggregates (`count`, `fold`), semi- and
//! anti-joins (`semi`), and the equi-join (`join`), which takes the merge shape when both sides are sorted on the
//! key and probes the other side otherwise.
//!
//! Nothing here holds a table's rows: every operation reads ids and cells where they lie (decision 0353). An
//! operator holds cursors: an id per run, a key.
//!
//! The change between two states of a table is a `Bistate` of it (`bistate`).

use crate::bistate::{ColumnRead, Read};
use crate::ids::{ColumnIndex, RowId};
use crate::relation::View;
use std::ops::{Bound, RangeBounds};

/// One run of ids `from .. to`, sorted on its table's sort column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub from: RowId,
    pub to: RowId,
}

impl Run {
    pub fn new(from: u32, to: u32) -> Run {
        assert!(from <= to, "a run's start is not past its end");
        Run { from: RowId::new(from), to: RowId::new(to) }
    }

    pub fn len(&self) -> u32 {
        self.to.get() - self.from.get()
    }

    pub fn is_empty(&self) -> bool {
        self.from == self.to
    }

    fn ids(self) -> impl Iterator<Item = RowId> {
        (self.from.get()..self.to.get()).map(RowId::new)
    }
}

/// A table's runs: one or two (constraint 1: a version's rows, then its delta's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Runs {
    runs: [Run; 2],
    n: usize,
}

impl Runs {
    pub fn one(a: Run) -> Runs {
        Runs { runs: [a, Run::new(0, 0)], n: 1 }
    }

    /// A version's run, then its delta's after it.
    pub fn two(a: Run, b: Run) -> Runs {
        assert!(a.to <= b.from, "a delta's ids come after its version's");
        Runs { runs: [a, b], n: 2 }
    }

    pub fn as_slice(&self) -> &[Run] {
        &self.runs[..self.n]
    }
}

/// How a table answers a probe of some columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// The sort column is among them: a range in each run, found by binary search, then the rest tested.
    Sorted,
    /// An index of the table's own on exactly them.
    Indexed,
    /// Neither: every live row tested.
    Scan,
}

/// A relation at one state, with the ways its rows are found.
pub trait Table: Read {
    /// The column its runs are each sorted on, or none.
    fn sort(&self) -> Option<ColumnIndex>;

    /// Its ids in runs, each sorted on `sort` (one run of every id when it has none). Every id lies in one run.
    fn runs(&self) -> Runs;

    /// The ids of `run` whose sort value is `key` (the table sorted): a range of it. By default two binary searches
    /// through the column; a table whose column has its own search (a packed column's parts) gives that.
    fn equal_range(&self, run: Run, key: i64) -> Run {
        let c = self.sort().expect("an equal range of a sorted table");
        let col = self.column(c);
        let lo = first_not_below(&col, run, key, false);
        let hi = first_not_below(&col, Run { from: lo, to: run.to }, key, true);
        Run { from: lo, to: hi }
    }

    /// The first id of `run` whose sort value is not below `key` (`run.to` when none is).
    fn lower_bound(&self, run: Run, key: i64) -> RowId {
        let c = self.sort().expect("a bound of a sorted table");
        first_not_below(&self.column(c), run, key, false)
    }

    /// How `probe` answers `cols`.
    fn access(&self, cols: &[ColumnIndex]) -> Access {
        if self.sort().is_some_and(|s| cols.contains(&s)) { Access::Sorted } else { Access::Scan }
    }

    /// The live ids whose columns `cols` equal `key` (`key[j]` for `cols[j]`), given to `each` until it returns
    /// false; false when it did. With the sort column among `cols`, each run's range of its value, in id order.
    fn probe(&self, cols: &[ColumnIndex], key: &[i64], each: &mut dyn FnMut(RowId) -> bool) -> bool {
        probe_by_runs(self, cols, key, each)
    }
}

/// `Table::probe`'s default: the sort column's range in each run when it is among `cols`, else every live row,
/// each tested on the rest.
pub fn probe_by_runs<T: Table + ?Sized>(t: &T, cols: &[ColumnIndex], key: &[i64], each: &mut dyn FnMut(RowId) -> bool) -> bool {
    assert_eq!(cols.len(), key.len(), "a key value a column");
    let matches = |id: RowId| t.live(id) && cols.iter().zip(key).all(|(&c, &k)| t.get(id, c) == k);
    match t.sort().and_then(|s| cols.iter().position(|&c| c == s)) {
        Some(j) => {
            for &run in t.runs().as_slice() {
                for id in t.equal_range(run, key[j]).ids() {
                    if matches(id) && !each(id) {
                        return false;
                    }
                }
            }
        }
        None => {
            for id in (0..t.end().get()).map(RowId::new) {
                if matches(id) && !each(id) {
                    return false;
                }
            }
        }
    }
    true
}

/// The first id of `run` whose value in `col` is not below `key` (`above`: is above it), `col` sorted over `run`.
fn first_not_below<C: ColumnRead>(col: &C, run: Run, key: i64, above: bool) -> RowId {
    let (mut lo, mut hi) = (run.from.get(), run.to.get());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let v = col.get(RowId::new(mid));
        if v < key || (above && v == key) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    RowId::new(lo)
}

/// A table's live ids in its sort order, between two bounds of the sort value: its runs merged as they are walked.
/// Ties go to the earlier run (the version's before the delta's).
pub struct Ordered<'t, T: Table + 't> {
    t: &'t T,
    col: T::Column<'t>,
    at: [Run; 2],
}

impl<T: Table> Iterator for Ordered<'_, T> {
    type Item = RowId;

    fn next(&mut self) -> Option<RowId> {
        for r in &mut self.at {
            while !r.is_empty() && !self.t.live(r.from) {
                r.from = RowId::new(r.from.get() + 1);
            }
        }
        let [a, b] = &mut self.at;
        let pick = match (a.is_empty(), b.is_empty()) {
            (true, true) => return None,
            (false, true) => a,
            (true, false) => b,
            (false, false) => {
                if self.col.get(b.from) < self.col.get(a.from) {
                    b
                } else {
                    a
                }
            }
        };
        let id = pick.from;
        pick.from = RowId::new(id.get() + 1);
        Some(id)
    }
}

/// Every live id of `t` in its sort order; none when it has no sort column.
pub fn ordered<T: Table>(t: &T) -> Option<Ordered<'_, T>> {
    range(t, ..)
}

/// The live ids of `t` whose sort value lies in `values`, in the sort order; none when it has no sort column.
pub fn range<T: Table>(t: &T, values: impl RangeBounds<i64>) -> Option<Ordered<'_, T>> {
    let c = t.sort()?;
    // The values as `lo .. hi`, half open; `None` for no bound. Values are integers, so `..=x` is `..x + 1`.
    let lo = match values.start_bound() {
        Bound::Included(&x) => Some(x),
        Bound::Excluded(&x) => match x.checked_add(1) {
            Some(x) => Some(x),
            None => return Some(Ordered { t, col: t.column(c), at: [Run::new(0, 0); 2] }),
        },
        Bound::Unbounded => None,
    };
    let hi = match values.end_bound() {
        Bound::Included(&x) => x.checked_add(1),
        Bound::Excluded(&x) => Some(x),
        Bound::Unbounded => None,
    };
    let mut at = [Run::new(0, 0); 2];
    for (slot, &run) in at.iter_mut().zip(t.runs().as_slice()) {
        let from = lo.map_or(run.from, |x| t.lower_bound(run, x));
        let to = hi.map_or(run.to, |x| t.lower_bound(run, x)).max(from);
        *slot = Run { from, to };
    }
    Some(Ordered { t, col: t.column(c), at })
}

/// Whether `t` holds the row `row` live (a value a column).
pub fn holds<T: Table + ?Sized>(t: &T, row: &[i64]) -> bool {
    assert_eq!(row.len(), t.arity(), "a value a column");
    let cols: Vec<ColumnIndex> = ColumnIndex::all(row.len()).collect();
    !t.probe(&cols, row, &mut |_| false)
}

/// How many live rows of `t` have `cols` equal to `key`.
pub fn count<T: Table + ?Sized>(t: &T, cols: &[ColumnIndex], key: &[i64]) -> usize {
    let mut n = 0;
    t.probe(cols, key, &mut |_| {
        n += 1;
        true
    });
    n
}

/// An aggregate of a key's rows: the engine's `count`, `sum`, `min`, `max` and bounded extremes (decision 0511).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agg {
    Count,
    Sum(ColumnIndex),
    Min(ColumnIndex),
    Max(ColumnIndex),
    /// The greatest value of the column below `bound` (`or_eq`: or equal to it).
    MaxBelow { col: ColumnIndex, bound: i64, or_eq: bool },
    /// The least value of the column above `bound` (`or_eq`: or equal to it).
    MinAbove { col: ColumnIndex, bound: i64, or_eq: bool },
}

/// `agg` of the live rows of `t` whose `cols` equal `key`: none for an extreme of no row; a count or sum of none is 0.
/// A sum wraps as the engine's integers do.
pub fn fold<T: Table + ?Sized>(t: &T, cols: &[ColumnIndex], key: &[i64], agg: Agg) -> Option<i64> {
    let mut acc: Option<i64> = match agg {
        Agg::Count | Agg::Sum(_) => Some(0),
        _ => None,
    };
    t.probe(cols, key, &mut |id| {
        let step = |acc: Option<i64>, v: i64, keep: bool, max: bool| match acc {
            _ if !keep => acc,
            None => Some(v),
            Some(a) => Some(if max { a.max(v) } else { a.min(v) }),
        };
        acc = match agg {
            Agg::Count => acc.map(|a| a + 1),
            Agg::Sum(c) => acc.map(|a| a.wrapping_add(t.get(id, c))),
            Agg::Min(c) => step(acc, t.get(id, c), true, false),
            Agg::Max(c) => step(acc, t.get(id, c), true, true),
            Agg::MaxBelow { col, bound, or_eq } => {
                let v = t.get(id, col);
                step(acc, v, v < bound || (or_eq && v == bound), true)
            }
            Agg::MinAbove { col, bound, or_eq } => {
                let v = t.get(id, col);
                step(acc, v, v > bound || (or_eq && v == bound), false)
            }
        };
        true
    });
    acc
}

/// The live ids of `a` that have (`neg`: have no) live row of `b` with `on`'s columns equal, each pair `(a's, b's)`,
/// given to `each` until it returns false. `b` is probed a row of `a` at a time, stopping at its first match.
pub fn semi<A: Table + ?Sized, B: Table + ?Sized>(a: &A, b: &B, on: &[(ColumnIndex, ColumnIndex)], neg: bool, each: &mut dyn FnMut(RowId) -> bool) -> bool {
    let (ac, bc): (Vec<ColumnIndex>, Vec<ColumnIndex>) = on.iter().copied().unzip();
    let mut key = vec![0i64; on.len()];
    for id in (0..a.end().get()).map(RowId::new).filter(|&id| a.live(id)) {
        for (k, &c) in key.iter_mut().zip(&ac) {
            *k = a.get(id, c);
        }
        let found = !b.probe(&bc, &key, &mut |_| false);
        if found != neg && !each(id) {
            return false;
        }
    }
    true
}

/// The shape a join took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Both sides read forward in their sort order, on a pair of `on` that is each side's sort column: each run of
    /// the inner side is searched only from where the last key's rows started.
    Merge,
    /// The outer side scanned, the inner probed a row at a time, through the inner's `access`.
    Probe(Access),
}

/// The live pairs `(a's id, b's id)` whose `on` columns are equal, given to `each` until it returns false.
///
/// With a pair of `on` that is both sides' sort column, the merge shape: `a` read in its order, and `b`'s rows for
/// each key read forward from the last key's (`Shape::Merge`), the other pairs tested. Otherwise `a` scanned and `b`
/// probed on all of `on` (`Shape::Probe`). Pairs come in `a`'s order (its sort order when merged, else id order),
/// and a key's `b` rows run by run.
pub fn join<A: Table, B: Table>(a: &A, b: &B, on: &[(ColumnIndex, ColumnIndex)], each: &mut dyn FnMut(RowId, RowId) -> bool) -> Shape {
    let lead = on.iter().position(|&(ca, cb)| a.sort() == Some(ca) && b.sort() == Some(cb));
    match lead {
        Some(j) => {
            merge_join(a, b, on, j, each);
            Shape::Merge
        }
        None => {
            let (ac, bc): (Vec<ColumnIndex>, Vec<ColumnIndex>) = on.iter().copied().unzip();
            let access = b.access(&bc);
            let mut key = vec![0i64; on.len()];
            for ia in (0..a.end().get()).map(RowId::new).filter(|&id| a.live(id)) {
                for (k, &c) in key.iter_mut().zip(&ac) {
                    *k = a.get(ia, c);
                }
                if !b.probe(&bc, &key, &mut |ib| each(ia, ib)) {
                    break;
                }
            }
            Shape::Probe(access)
        }
    }
}

/// `join`'s merge shape, on `on[lead]`: `a` in its order; for each new key, each of `b`'s runs searched forward from
/// where the last key's rows began (keys rise, so they never lie before it).
fn merge_join<A: Table, B: Table>(a: &A, b: &B, on: &[(ColumnIndex, ColumnIndex)], lead: usize, each: &mut dyn FnMut(RowId, RowId) -> bool) {
    let ka = on[lead].0;
    let acol = a.column(ka);
    // Each of b's runs' rest (from the last key's start: the cursors start at the runs) and its rows of the key.
    let Runs { runs: mut rest, n } = b.runs();
    let mut found: [Run; 2] = [Run::new(0, 0); 2];
    let mut last: Option<i64> = None;
    let Some(walk) = ordered(a) else { unreachable!("the merge shape's outer side is sorted") };
    for ia in walk {
        let k = acol.get(ia);
        if last != Some(k) {
            for (r, f) in rest.iter_mut().zip(found.iter_mut()).take(n) {
                *f = b.equal_range(*r, k);
                r.from = f.from;
            }
            last = Some(k);
        }
        for f in &found[..n] {
            for ib in f.ids() {
                if b.live(ib) && on.iter().all(|&(ca, cb)| a.get(ia, ca) == b.get(ib, cb)) && !each(ia, ib) {
                    return;
                }
            }
        }
    }
}

/// A relation read as a table: frozen, its own rows (and its base's) lie in canonical order, so sorted on the
/// order's first column, and a base and its delta are its two runs. Its probes use its hashed indexes where one is
/// on exactly the probe's columns, in both relations.
impl Table for View<'_> {
    fn sort(&self) -> Option<ColumnIndex> {
        let frozen = self.top.is_frozen() && self.base.is_none_or(|b| b.is_frozen());
        (frozen && self.arity() > 0).then(|| ColumnIndex::new(if self.top.nokey > 0 { self.top.okey[0] } else { 0 }))
    }

    fn runs(&self) -> Runs {
        let (b, end) = (self.top.base_rows, View::total(self));
        match self.base {
            Some(_) => Runs::two(Run::new(0, b), Run::new(b, end)),
            None => Runs::one(Run::new(b, end)),
        }
    }

    fn access(&self, cols: &[ColumnIndex]) -> Access {
        if self.sort().is_some_and(|s| cols.contains(&s)) {
            Access::Sorted
        } else if self.hashed(cols).is_some() {
            Access::Indexed
        } else {
            Access::Scan
        }
    }

    fn probe(&self, cols: &[ColumnIndex], key: &[i64], each: &mut dyn FnMut(RowId) -> bool) -> bool {
        if self.access(cols) != Access::Indexed {
            return probe_by_runs(self, cols, key, each);
        }
        let (i, order) = self.hashed(cols).expect("an index on the columns");
        let k: Vec<i64> = order.iter().map(|&j| key[j]).collect();
        for id in self.probe_iter(i, &k) {
            if !each(RowId::new(id)) {
                return false;
            }
        }
        true
    }
}

impl View<'_> {
    /// A hashed index of every row of both relations, on exactly `cols` in some order: its number (the same in
    /// both), and for each of its key's columns, its place in `cols`.
    fn hashed(&self, cols: &[ColumnIndex]) -> Option<(usize, Vec<usize>)> {
        use crate::relation::HASHED;
        if !self.top.is_frozen() {
            return None;
        }
        let n = self.top.nindex.load(std::sync::atomic::Ordering::Acquire) as usize;
        let covers = |r: &crate::relation::Relation, i: usize| {
            let nr = r.nindex.load(std::sync::atomic::Ordering::Acquire) as usize;
            let ix = (i < nr.min(r.indexes.len())).then(|| r.index_at(i))?;
            let key = &ix.key[..ix.ncols as usize];
            (ix.ordered == HASHED && ix.from == 0 && ix.rows.len() == r.rows.len() as usize && key.len() == cols.len())
                .then(|| key.iter().map(|&c| cols.iter().position(|x| x.get() == c as usize)).collect::<Option<Vec<usize>>>())
                .flatten()
        };
        (0..n.min(self.top.indexes.len())).find_map(|i| {
            let order = covers(self.top, i)?;
            match self.base {
                Some(b) if covers(b, i).as_ref() != Some(&order) => None,
                _ => Some((i, order)),
            }
        })
    }
}

/// A kept relation read as a table: its packed rows in canonical order, one run, sorted on the order's first column
/// when it records an order (an input's granule kept as given records none). Its probes are its packed probes where
/// it has one for the columns: the order's leading columns, or one column's permutation index.
impl Table for crate::kept::Kept<'_> {
    fn sort(&self) -> Option<ColumnIndex> {
        (self.rows.nokey > 0).then(|| ColumnIndex::new(self.rows.okey[0]))
    }

    fn runs(&self) -> Runs {
        Runs::one(Run::new(0, Read::end(self).get()))
    }

    fn equal_range(&self, run: Run, key: i64) -> Run {
        let c = self.sort().expect("an equal range of a sorted table");
        let (lo, hi) = self.rows.col(c.get()).dec().equal_range(run.from.get() as usize, run.to.get() as usize, key);
        Run::new(lo as u32, hi as u32)
    }

    fn lower_bound(&self, run: Run, key: i64) -> RowId {
        let c = self.sort().expect("a bound of a sorted table");
        RowId::new(self.rows.col(c.get()).dec().lower_bound(run.from.get() as usize, run.to.get() as usize, key) as u32)
    }

    fn access(&self, cols: &[ColumnIndex]) -> Access {
        if self.sort().is_some_and(|s| cols.contains(&s)) {
            Access::Sorted
        } else if self.packed_probe(cols) {
            Access::Indexed
        } else {
            Access::Scan
        }
    }

    fn probe(&self, cols: &[ColumnIndex], key: &[i64], each: &mut dyn FnMut(RowId) -> bool) -> bool {
        if !self.packed_probe(cols) {
            return probe_by_runs(self, cols, key, each);
        }
        let c: Vec<u16> = cols.iter().map(|c| c.get() as u16).collect();
        for p in self.rows.probe(&c, key) {
            if !each(RowId::new(p as u32)) {
                return false;
            }
        }
        true
    }
}

impl crate::kept::Kept<'_> {
    /// Whether `Packed::probe` answers `cols` exactly: they are the order's leading columns, or one column with a
    /// permutation index.
    fn packed_probe(&self, cols: &[ColumnIndex]) -> bool {
        let ok = &self.rows.okey[..self.rows.nokey as usize];
        let leading = !cols.is_empty() && cols.len() <= ok.len() && cols.iter().all(|c| ok[..cols.len()].contains(&(c.get() as u16)));
        let indexed = cols.len() == 1 && self.rows.indexes.as_slice().iter().any(|ix| ix.col as usize == cols[0].get());
        leading || indexed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Block, kind};
    use crate::relation::{self, Relation, RelW, Spec};
    use std::sync::Mutex;

    fn c(i: usize) -> ColumnIndex {
        ColumnIndex::new(i as u16)
    }

    /// A base relation and its delta in one block, as `bistate`'s tests make them: sorted on column `order`, a
    /// hashed index on each of `indexes`.
    fn with_delta(order: u16, indexes: &[&[u16]], base: &[[i64; 2]], added: &[[i64; 2]], killed: &[u32], f: impl FnOnce(View<'_>)) {
        let block = Block::anon(kind::FACTS, 16 << 20);
        let spec = Spec { widths: vec![8, 8], order: vec![order], indexes: indexes.iter().map(|k| k.to_vec()).collect(), min_log: crate::rows::MIN_LOG };
        let lock = Mutex::new(());
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

    /// The live rows of `t`, by id: what every operation is checked against, read a cell at a time.
    fn live_rows<T: Table>(t: &T) -> Vec<(RowId, [i64; 2])> {
        (0..t.end().get()).map(RowId::new).filter(|&id| t.live(id)).map(|id| (id, [t.get(id, c(0)), t.get(id, c(1))])).collect()
    }

    fn base_rows() -> (Vec<[i64; 2]>, Vec<[i64; 2]>) {
        // Column 0 repeats (keys of 3 rows), column 1 varies; the delta interleaves its keys with the base's.
        let base: Vec<[i64; 2]> = (0..60).map(|i| [i / 3 * 2, (i * 37) % 23]).collect();
        let added: Vec<[i64; 2]> = (0..40).map(|i| [i / 2 * 2 + 1, (i * 11) % 17]).chain([[4, 100], [10, 101]]).collect();
        (base, added)
    }

    /// Every probe of a view, by any access, gives exactly the live rows with the key: the sort column's two ranges,
    /// a hashed index on the other column, and a scan of both columns.
    #[test]
    fn a_views_probes_give_the_live_rows_of_their_key() {
        let (base, added) = base_rows();
        with_delta(0, &[&[1]], &base, &added, &[0, 5, 6, 31], |v| {
            assert_eq!(v.sort(), Some(c(0)));
            assert_eq!(v.runs().as_slice().len(), 2);
            assert_eq!(v.access(&[c(0)]), Access::Sorted);
            assert_eq!(v.access(&[c(1)]), Access::Indexed);
            assert_eq!(v.access(&[c(1), c(0)]), Access::Sorted);
            let rows = live_rows(&v);
            for cols in [vec![c(0)], vec![c(1)], vec![c(1), c(0)]] {
                for k0 in -1..45 {
                    for k1 in [-1, 0, 3, 16, 100] {
                        let key: Vec<i64> = cols.iter().map(|x| if x.get() == 0 { k0 } else { k1 }).collect();
                        let mut got = Vec::new();
                        Table::probe(&v, &cols, &key, &mut |id| {
                            got.push(id);
                            true
                        });
                        got.sort();
                        let want: Vec<RowId> = rows.iter().filter(|(_, r)| cols.iter().zip(&key).all(|(x, k)| r[x.get()] == *k)).map(|r| r.0).collect();
                        assert_eq!(got, want, "{cols:?} = {key:?}");
                        assert_eq!(count(&v, &cols, &key), want.len());
                    }
                }
            }
            for &(_, r) in &rows {
                assert!(holds(&v, &r));
            }
            assert!(!holds(&v, &base[0]), "a killed row is not held");
            assert!(!holds(&v, &[1, 99]));
        });
    }

    /// The sort order of a version and its delta is their runs merged: every live row once, sorted on the sort
    /// column, and a range of values is the rows with those values, for every kind of bound.
    #[test]
    fn a_views_ordered_reads_merge_its_two_runs() {
        let (base, added) = base_rows();
        with_delta(0, &[], &base, &added, &[2, 3, 40], |v| {
            let rows = live_rows(&v);
            let walk: Vec<RowId> = ordered(&v).unwrap().collect();
            assert_eq!(walk.len(), rows.len());
            let vals: Vec<i64> = walk.iter().map(|&id| Read::get(&v, id, c(0))).collect();
            assert!(vals.is_sorted(), "{vals:?}");
            let mut ids = walk.clone();
            ids.sort();
            assert_eq!(ids, rows.iter().map(|r| r.0).collect::<Vec<_>>());
            let check = |got: Vec<RowId>, keep: &dyn Fn(i64) -> bool| {
                let want: Vec<i64> = vals.iter().copied().filter(|&x| keep(x)).collect();
                assert_eq!(got.iter().map(|&id| Read::get(&v, id, c(0))).collect::<Vec<_>>(), want);
            };
            check(range(&v, 7..20).unwrap().collect(), &|x| (7..20).contains(&x));
            check(range(&v, 7..=20).unwrap().collect(), &|x| (7..=20).contains(&x));
            check(range(&v, ..=4).unwrap().collect(), &|x| x <= 4);
            check(range(&v, (Bound::Excluded(9), Bound::Unbounded)).unwrap().collect(), &|x| x > 9);
            check(range(&v, (Bound::Excluded(i64::MAX), Bound::Unbounded)).unwrap().collect(), &|_| false);
            check(range(&v, 30..10).unwrap().collect(), &|_| false);
        });
    }

    /// A key's aggregates read its probe: count, sum, min, max and the bounded extremes, none for no row.
    #[test]
    fn a_keys_aggregates_read_its_rows() {
        let (base, added) = base_rows();
        with_delta(0, &[&[1]], &base, &added, &[1, 7], |v| {
            let rows = live_rows(&v);
            for k in -1..42 {
                let vals: Vec<i64> = rows.iter().filter(|r| r.1[0] == k).map(|r| r.1[1]).collect();
                let f = |agg| fold(&v, &[c(0)], &[k], agg);
                assert_eq!(f(Agg::Count), Some(vals.len() as i64));
                assert_eq!(f(Agg::Sum(c(1))), Some(vals.iter().sum()));
                assert_eq!(f(Agg::Min(c(1))), vals.iter().min().copied());
                assert_eq!(f(Agg::Max(c(1))), vals.iter().max().copied());
                for (bound, or_eq) in [(5, false), (5, true), (12, false), (0, true)] {
                    let below = vals.iter().copied().filter(|&x| x < bound || (or_eq && x == bound)).max();
                    let above = vals.iter().copied().filter(|&x| x > bound || (or_eq && x == bound)).min();
                    assert_eq!(f(Agg::MaxBelow { col: c(1), bound, or_eq }), below, "key {k}, below {bound} {or_eq}");
                    assert_eq!(f(Agg::MinAbove { col: c(1), bound, or_eq }), above, "key {k}, above {bound} {or_eq}");
                }
            }
        });
    }

    /// Every pair a nested loop over the cells finds.
    fn pairs_by_cells<A: Table, B: Table>(a: &A, b: &B, on: &[(ColumnIndex, ColumnIndex)]) -> Vec<(RowId, RowId)> {
        let mut out = Vec::new();
        for (ia, _) in live_rows(a) {
            for (ib, _) in live_rows(b) {
                if on.iter().all(|&(ca, cb)| a.get(ia, ca) == b.get(ib, cb)) {
                    out.push((ia, ib));
                }
            }
        }
        out
    }

    fn join_all<A: Table, B: Table>(a: &A, b: &B, on: &[(ColumnIndex, ColumnIndex)]) -> (Shape, Vec<(RowId, RowId)>) {
        let mut got = Vec::new();
        let shape = join(a, b, on, &mut |x, y| {
            got.push((x, y));
            true
        });
        got.sort();
        (shape, got)
    }

    /// A join takes the merge shape when both sides are sorted on the key and probes otherwise, and every shape
    /// gives the pairs a nested loop over the cells gives; a semi-join keeps the rows with a match, an anti-join
    /// those with none.
    #[test]
    fn joins_take_the_shape_the_sort_affords_and_agree() {
        let (base, added) = base_rows();
        let other: Vec<[i64; 2]> = (0..50).map(|i| [(i * 7) % 45, i % 9]).collect();
        let more: Vec<[i64; 2]> = (0..20).map(|i| [i * 3 + 1, (i * 5) % 9]).collect();
        with_delta(0, &[&[1]], &base, &added, &[0, 4, 50], |a| {
            with_delta(0, &[&[1]], &other, &more, &[2, 9], |b| {
                with_delta(1, &[], &other, &more, &[3], |b1| {
                    let on0 = [(c(0), c(0))];
                    let (shape, got) = join_all(&a, &b, &on0);
                    assert_eq!(shape, Shape::Merge);
                    assert_eq!(got, pairs_by_cells(&a, &b, &on0));
                    assert!(!got.is_empty());
                    // Both columns: merged on the sort column, the other pair tested.
                    let on2 = [(c(1), c(1)), (c(0), c(0))];
                    let (shape, got) = join_all(&a, &b, &on2);
                    assert_eq!(shape, Shape::Merge);
                    assert_eq!(got, pairs_by_cells(&a, &b, &on2));
                    // a's column 1 against b's: neither side sorted on it, b indexed.
                    let on1 = [(c(1), c(1))];
                    let (shape, got) = join_all(&a, &b, &on1);
                    assert_eq!(shape, Shape::Probe(Access::Indexed));
                    assert_eq!(got, pairs_by_cells(&a, &b, &on1));
                    // a sorted on column 0, b1 on column 1: a's 0 against b1's 1 is merged, though the columns differ.
                    let on01 = [(c(0), c(1))];
                    let (shape, got) = join_all(&a, &b1, &on01);
                    assert_eq!(shape, Shape::Merge);
                    assert_eq!(got, pairs_by_cells(&a, &b1, &on01));
                    // a's 1 against b1's 1: only the inner side sorted on it, so probed by its sort column's ranges.
                    let (shape, got) = join_all(&a, &b1, &on1);
                    assert_eq!(shape, Shape::Probe(Access::Sorted));
                    assert_eq!(got, pairs_by_cells(&a, &b1, &on1));
                    // b1 has no index on column 0: a scan.
                    let (shape, got) = join_all(&a, &b1, &on0);
                    assert_eq!(shape, Shape::Probe(Access::Scan));
                    assert_eq!(got, pairs_by_cells(&a, &b1, &on0));
                    // Semi- and anti-joins split a's live rows.
                    let all = pairs_by_cells(&a, &b, &on0);
                    let mut with = Vec::new();
                    semi(&a, &b, &on0, false, &mut |id| {
                        with.push(id);
                        true
                    });
                    let mut without = Vec::new();
                    semi(&a, &b, &on0, true, &mut |id| {
                        without.push(id);
                        true
                    });
                    let mut want: Vec<RowId> = all.iter().map(|p| p.0).collect();
                    want.dedup();
                    assert_eq!(with, want);
                    assert_eq!(with.len() + without.len(), live_rows(&a).len());
                    assert!(without.iter().all(|id| !want.contains(id)));
                    // A join stopped early gives no more pairs.
                    let mut n = 0;
                    join(&a, &b, &on0, &mut |_, _| {
                        n += 1;
                        n < 3
                    });
                    assert_eq!(n, 3);
                });
            });
        });
    }

    /// A kept relation is a table of one run: its probes, ordered reads and joins with a view agree with its cells.
    #[test]
    fn a_kept_relation_is_a_table_of_one_run() {
        use crate::block::Flush;
        use crate::kept::{FactKey, KeptChain, KeptFacts, KeptRead, KeptW, Meta};
        let d = std::env::temp_dir().join(format!("rerail-table-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        crate::mapped::make_folder(&d).unwrap();
        let data: Vec<(i64, i64)> = (0..500).map(|i| ((i * 7) % 90, i % 13)).collect();
        let cols: Vec<i64> = data.iter().map(|r| r.0).chain(data.iter().map(|r| r.1)).collect();
        let mut w = KeptW::create(&d.join("t.blk"), 5, 16 << 20).unwrap();
        let key = FactKey { rel: "r", ident: &[7; 32], part: 0 };
        w.put_cols(Meta { key, digest: 1, reads: &[], types: &[] }, &cols, data.len(), 2, None, crate::packed::Order::Canonical).unwrap();
        w.finish(Flush::Synced).unwrap();
        let files = [KeptFacts::open(&d.join("t.blk"), 5).unwrap()];
        let chain = KeptChain { files: &files };
        let k = chain.get(&key).unwrap();
        let sort = k.sort().expect("a canonical packing records its order");
        assert_eq!(k.runs().as_slice().len(), 1);
        let rows = live_rows(&k);
        let mut distinct: Vec<(i64, i64)> = data.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(rows.len(), distinct.len());
        for col in [sort, c(1 - sort.get())] {
            for v in -1..95 {
                let mut got = Vec::new();
                k.probe(&[col], &[v], &mut |id| {
                    got.push(id);
                    true
                });
                got.sort();
                let want: Vec<RowId> = rows.iter().filter(|r| r.1[col.get()] == v).map(|r| r.0).collect();
                assert_eq!(got, want, "{col:?} = {v}");
            }
        }
        let vals: Vec<i64> = ordered(&k).unwrap().map(|id| k.get(id, sort)).collect();
        assert!(vals.is_sorted());
        assert_eq!(vals.len(), rows.len());
        let mid: Vec<i64> = range(&k, 20..40).unwrap().map(|id| k.get(id, sort)).collect();
        assert_eq!(mid, vals.iter().copied().filter(|x| (20..40).contains(x)).collect::<Vec<_>>());
        // Joined with a view sorted on the same column: merged, and the pairs a nested loop gives.
        let (base, added) = base_rows();
        with_delta(sort.get() as u16, &[], &base, &added, &[1], |v| {
            let on = [(sort, sort)];
            let (shape, got) = join_all(&k, &v, &on);
            assert_eq!(shape, Shape::Merge);
            assert_eq!(got, pairs_by_cells(&k, &v, &on));
            let (shape, got) = join_all(&v, &k, &on);
            assert_eq!(shape, Shape::Merge);
            assert_eq!(got, pairs_by_cells(&v, &k, &on));
        });
        let _ = std::fs::remove_dir_all(&d);
    }
}
