//! The operations on fact tables in their change form (decision 0678 step 5): each takes its inputs as bistates (a
//! version and its delta, `bistate`) and gives its output's change, without reading the inputs' rows that did not
//! change except through probes of the keys that did.
//!
//! An input `A` across a change has three parts, read by id where they lie:
//!
//! - `A_gone`: live before, dead after (`Bistate::gone`);
//! - `A_added`: dead or absent before, live after (`Bistate::added`);
//! - `A_kept`: live in both. Never listed: a probe of the after state finds a row, and the before state's liveness
//!   says whether it is kept (`kept`).
//!
//! Every output row is gone or added at most once, and a row in both states is in neither, so an output's change
//! is the least one when its inputs' are. The laws, checked by `check` against the cold operations of `table`:
//!
//! - **L1**: the output before, less what is gone, plus what is added, is the output after; what is gone was in the
//!   output before, what is added was not;
//! - **L3**: a chain of changes applied in turn to a cold output is the cold output of the last state (the tests).
//!
//! An output row is given by the ids of the input rows it is made of (a join's pair, a semi-join's row) or by its
//! values (an aggregate's key and value); a gone row's values are its before state's, an added row's its after
//! state's. Nothing is collected but the keys an input's change touches (a semi-join's and an aggregate's), each
//! once.

use crate::bistate::{Bistate, Read};
use crate::ids::{ColumnIndex, RowId};
use crate::table::{Agg, Table, count, fold};
use rustc_hash::FxHashSet;

/// Whether an output row is gone or added.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Sign {
    Gone,
    Added,
}

/// Whether row `id` of `s` is kept: live before and after.
pub fn kept<T: Read>(s: &Bistate<T>, id: RowId) -> bool {
    s.before().is_some_and(|b| id < b.end() && b.live(id)) && s.after().live(id)
}

/// Row `id`'s values in `cols`, read from `t` into `key`.
fn read_key<T: Read + ?Sized>(t: &T, id: RowId, cols: &[ColumnIndex], key: &mut [i64]) {
    for (k, &c) in key.iter_mut().zip(cols) {
        *k = t.get(id, c);
    }
}

/// The change of `A ⋈ B` on `on` (pairs of `A`'s and `B`'s columns): each pair `(a, b)` of ids gone or added.
///
/// - gone: `A_gone ⋈ B_before`, and `A_kept ⋈ B_gone`;
/// - added: `A_added ⋈ B_after`, and `A_kept ⋈ B_added`.
///
/// A pair is in one term at most: an `A` row is gone, added or kept, and a kept one meets only `B`'s change. Each
/// term probes one side for the other side's changed rows: its work is the change's rows and their matches.
pub fn join<A: Table, B: Table>(a: &Bistate<A>, b: &Bistate<B>, on: &[(ColumnIndex, ColumnIndex)], each: &mut dyn FnMut(Sign, RowId, RowId)) {
    let (ac, bc): (Vec<ColumnIndex>, Vec<ColumnIndex>) = on.iter().copied().unzip();
    let mut key = vec![0i64; on.len()];
    if let (Some(ab), Some(bb)) = (a.before(), b.before()) {
        for ia in a.gone() {
            read_key(ab, ia, &ac, &mut key);
            bb.probe(&bc, &key, &mut |ib| {
                each(Sign::Gone, ia, ib);
                true
            });
        }
        for ib in b.gone() {
            read_key(bb, ib, &bc, &mut key);
            a.after().probe(&ac, &key, &mut |ia| {
                if kept(a, ia) {
                    each(Sign::Gone, ia, ib);
                }
                true
            });
        }
    }
    for ia in a.added() {
        read_key(a.after(), ia, &ac, &mut key);
        b.after().probe(&bc, &key, &mut |ib| {
            each(Sign::Added, ia, ib);
            true
        });
    }
    if a.before().is_some() {
        for ib in b.added() {
            read_key(b.after(), ib, &bc, &mut key);
            a.after().probe(&ac, &key, &mut |ia| {
                if kept(a, ia) {
                    each(Sign::Added, ia, ib);
                }
                true
            });
        }
    }
}

/// Whether `t` has a live row whose `cols` are `key`; none for no state.
fn has<T: Table>(t: Option<&T>, cols: &[ColumnIndex], key: &[i64]) -> bool {
    t.is_some_and(|t| !t.probe(cols, key, &mut |_| false))
}

/// The keys `B`'s change touches on `cols`: of its rows gone (read before) and added (read after), each once.
fn touched<T: Table>(s: &Bistate<T>, cols: &[ColumnIndex]) -> FxHashSet<Vec<i64>> {
    let mut keys = FxHashSet::default();
    let mut key = vec![0i64; cols.len()];
    if let Some(b) = s.before() {
        for id in s.gone() {
            read_key(b, id, cols, &mut key);
            keys.insert(key.clone());
        }
    }
    for id in s.added() {
        read_key(s.after(), id, cols, &mut key);
        keys.insert(key.clone());
    }
    keys
}

/// The change of `A ⋉ B` on `on` (`neg`: `A ▷ B`, the rows of `A` with no match): each row of `A` gone or added.
///
/// - `A`'s changed rows: one gone that matched before (did not, `neg`) is gone; one added that matches after is
///   added;
/// - `A`'s kept rows: only those whose key `B`'s change touches can change. For each such key whose match in `B`
///   came or went, its kept rows go or come.
pub fn semi<A: Table, B: Table>(a: &Bistate<A>, b: &Bistate<B>, on: &[(ColumnIndex, ColumnIndex)], neg: bool, each: &mut dyn FnMut(Sign, RowId)) {
    let (ac, bc): (Vec<ColumnIndex>, Vec<ColumnIndex>) = on.iter().copied().unzip();
    let mut key = vec![0i64; on.len()];
    if let Some(ab) = a.before() {
        for ia in a.gone() {
            read_key(ab, ia, &ac, &mut key);
            if has(b.before(), &bc, &key) != neg {
                each(Sign::Gone, ia);
            }
        }
    }
    for ia in a.added() {
        read_key(a.after(), ia, &ac, &mut key);
        if has(Some(b.after()), &bc, &key) != neg {
            each(Sign::Added, ia);
        }
    }
    if a.before().is_none() {
        return;
    }
    for key in touched(b, &bc) {
        let (was, is) = (has(b.before(), &bc, &key) != neg, has(Some(b.after()), &bc, &key) != neg);
        if was == is {
            continue;
        }
        let sign = if is { Sign::Added } else { Sign::Gone };
        a.after().probe(&ac, &key, &mut |ia| {
            if kept(a, ia) {
                each(sign, ia);
            }
            true
        });
    }
}

/// The change of `A` grouped by `by` under `agg` (a row `(key, value)` for each key `A` holds): each key whose value
/// changed, its old row gone (where it had one) and its new one added (where it has one). Only the keys `A`'s change
/// touches are read, each in both states.
pub fn group<A: Table>(a: &Bistate<A>, by: &[ColumnIndex], agg: Agg, each: &mut dyn FnMut(Sign, &[i64], i64)) {
    let value = |t: Option<&A>, key: &[i64]| t.filter(|t| count(*t, by, key) > 0).and_then(|t| fold(t, by, key, agg));
    for key in touched(a, by) {
        let (old, new) = (value(a.before(), &key), value(Some(a.after()), &key));
        if old == new {
            continue;
        }
        if let Some(v) = old {
            each(Sign::Gone, &key, v);
        }
        if let Some(v) = new {
            each(Sign::Added, &key, v);
        }
    }
}

/// Checks L1 for an operation's change against its cold form (decision 0678 step 5): `cold` gives the output of a
/// state of the inputs as rows of values (`None` for no state: an empty output); `change` the rows gone and added.
/// Gone rows were in the output before, added ones were not, none is both, and the output before less what is gone
/// plus what is added is the output after.
#[cfg(any(test, feature = "test-hooks"))]
pub fn check(before: Option<Vec<Vec<i64>>>, after: Vec<Vec<i64>>, change: Vec<(Sign, Vec<i64>)>) {
    use std::collections::BTreeSet;
    let before: BTreeSet<Vec<i64>> = before.unwrap_or_default().into_iter().collect();
    let after: BTreeSet<Vec<i64>> = after.into_iter().collect();
    let mut seen = BTreeSet::new();
    let mut out = before.clone();
    for (sign, row) in &change {
        assert!(seen.insert(row.clone()), "{row:?}: gone or added once");
        match sign {
            Sign::Gone => assert!(out.remove(row), "{row:?}: gone, so in the output before"),
            Sign::Added => {
                assert!(!before.contains(row), "{row:?}: added, so not in the output before");
            }
        }
    }
    for (sign, row) in change {
        if sign == Sign::Added {
            out.insert(row);
        }
    }
    assert_eq!(out, after, "the output before, less what is gone, plus what is added, is the output after");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Block, kind};
    use crate::relation::{self, Relation, RelW, Spec, View};
    use std::sync::Mutex;

    fn c(i: usize) -> ColumnIndex {
        ColumnIndex::new(i as u16)
    }

    /// A chain of states of one relation of two columns, each a version (a full relation of the last state's live
    /// rows: a roll-up) and a delta of it, in one block. `step(k, rows)` gives state `k`'s change: rows to add, and
    /// rows (values) of the version to kill.
    struct Chain {
        block: Block,
        spec: Spec,
        lock: Mutex<()>,
    }

    impl Chain {
        fn new(order: u16, indexes: &[&[u16]]) -> Chain {
            let spec = Spec { widths: vec![8, 8], sort: Some(order), indexes: indexes.iter().map(|k| k.to_vec()).collect(), min_log: crate::rows::MIN_LOG };
            Chain { block: Block::anon(kind::FACTS, 64 << 20), spec, lock: Mutex::new(()) }
        }

        /// A full relation of `rows`, frozen.
        fn full(&self, rows: &[[i64; 2]]) -> &Relation {
            let r = Relation::create(&self.block, &self.spec, None);
            let w = RelW { block: &self.block, rel: r, lock: &self.lock };
            for row in rows {
                w.push(row);
            }
            relation::freeze(&self.block, r, &self.spec, None);
            r
        }

        /// A delta of `base`: `add` pushed, the base's rows equal to `kill` killed.
        fn delta(&self, base: &Relation, add: &[[i64; 2]], kill: &[[i64; 2]]) -> &Relation {
            let d = Relation::create(&self.block, &self.spec, Some(base));
            let w = RelW { block: &self.block, rel: d, lock: &self.lock };
            for row in add {
                w.push(row);
            }
            for row in kill {
                for id in 0..base.rows.len() {
                    if base.rows.get(id, 0) == row[0] && base.rows.get(id, 1) == row[1] {
                        w.kill(id);
                    }
                }
            }
            relation::freeze(&self.block, d, &self.spec, Some(base));
            d
        }
    }

    fn rows_of<T: Read>(t: &T) -> Vec<[i64; 2]> {
        t.live_ids().map(|id| [Read::get(t, id, c(0)), Read::get(t, id, c(1))]).collect()
    }

    /// A small pseudo-random stream: the tests' changes, the same every run.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self, n: i64) -> i64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 33) % n as u64) as i64
        }
    }

    /// State `k` of a chain: its version's rows (the last state's live rows), and its change.
    /// A state of a chain: its version's rows, the rows its delta adds, and those it kills.
    type State = (Vec<[i64; 2]>, Vec<[i64; 2]>, Vec<[i64; 2]>);

    fn states(seed: u64, steps: usize, keys: i64) -> Vec<State> {
        let mut r = Rng(seed);
        let mut rows: Vec<[i64; 2]> = (0..30).map(|_| [r.next(keys), r.next(9)]).collect();
        rows.sort();
        rows.dedup();
        let mut out = Vec::new();
        for _ in 0..steps {
            let add: Vec<[i64; 2]> = (0..r.next(8)).map(|_| [r.next(keys), r.next(9)]).collect();
            let kill: Vec<[i64; 2]> = rows.iter().filter(|_| r.next(6) == 0).copied().collect();
            out.push((rows.clone(), add.clone(), kill.clone()));
            let mut next: Vec<[i64; 2]> = rows.iter().filter(|x| !kill.contains(x)).copied().chain(add).collect();
            next.sort();
            next.dedup();
            rows = next;
        }
        out
    }

    /// Two relations' chains, side by side, each state read as a bistate of views.
    fn each_state(order_a: u16, order_b: u16, steps: usize, mut f: impl FnMut(usize, &Bistate<View<'_>>, &Bistate<View<'_>>)) {
        let (ca, cb) = (Chain::new(order_a, &[&[0], &[1]]), Chain::new(order_b, &[&[0], &[1]]));
        let (sa, sb) = (states(7, steps, 12), states(11, steps, 12));
        for (k, ((va, adda, killa), (vb, addb, killb))) in sa.iter().zip(&sb).enumerate() {
            let (ba, bb) = (ca.full(va), cb.full(vb));
            let (da, db) = (ca.delta(ba, adda, killa), cb.delta(bb, addb, killb));
            let (a, b) = (View { base: Some(ba), top: da }, View { base: Some(bb), top: db });
            f(k, &a.bistate(), &b.bistate());
        }
    }

    fn join_rows<A: Table, B: Table>(a: &A, b: &B, on: &[(ColumnIndex, ColumnIndex)]) -> Vec<Vec<i64>> {
        let mut out = Vec::new();
        crate::table::join(a, b, on, &mut |x, y| {
            out.push(vec![a.get(x, c(0)), a.get(x, c(1)), b.get(y, c(0)), b.get(y, c(1))]);
            true
        });
        out
    }

    /// A join's change keeps L1 at every state, on every join shape and key: merged, probed by index and by range,
    /// on one column and on two; and the changes of a chain, applied in turn, are the last state's cold join (L3).
    #[test]
    fn a_joins_change_keeps_the_laws() {
        for (order_b, on) in [(0u16, vec![(c(0), c(0))]), (1, vec![(c(0), c(0))]), (0, vec![(c(1), c(1))]), (1, vec![(c(0), c(0)), (c(1), c(1))])] {
            let mut applied: Option<std::collections::BTreeSet<Vec<i64>>> = None;
            each_state(0, order_b, 12, |k, a, b| {
                let before = join_rows(a.before().unwrap(), b.before().unwrap(), &on);
                let after = join_rows(a.after(), b.after(), &on);
                let mut change = Vec::new();
                join(a, b, &on, &mut |s, x, y| {
                    let (ta, tb) = if s == Sign::Gone { (a.before().unwrap(), b.before().unwrap()) } else { (a.after(), b.after()) };
                    change.push((s, vec![Read::get(ta, x, c(0)), Read::get(ta, x, c(1)), Read::get(tb, y, c(0)), Read::get(tb, y, c(1))]));
                });
                // L3: the chain's output, carried from the first state's cold output by the changes alone.
                let out = applied.get_or_insert_with(|| before.iter().cloned().collect());
                for (s, row) in &change {
                    if *s == Sign::Gone {
                        out.remove(row);
                    }
                }
                for (s, row) in &change {
                    if *s == Sign::Added {
                        out.insert(row.clone());
                    }
                }
                assert_eq!(*out, after.iter().cloned().collect(), "state {k}: the changes applied are the cold join");
                check(Some(before), after, change);
            });
        }
    }

    /// A semi-join's and an anti-join's change keep L1 and L3, a kept row going and coming as its key's match in
    /// the other relation goes and comes.
    #[test]
    fn a_semi_joins_change_keeps_the_laws() {
        for neg in [false, true] {
            for on in [vec![(c(0), c(0))], vec![(c(1), c(0))]] {
                let mut applied: Option<std::collections::BTreeSet<Vec<i64>>> = None;
                let mut kept_moved = 0;
                each_state(0, 0, 12, |k, a, b| {
                    let semi_rows = |ta: &View, tb: &View| {
                        let mut out = Vec::new();
                        crate::table::semi(ta, tb, &on, neg, &mut |x| {
                            out.push(vec![Read::get(ta, x, c(0)), Read::get(ta, x, c(1))]);
                            true
                        });
                        out
                    };
                    let before = semi_rows(a.before().unwrap(), b.before().unwrap());
                    let after = semi_rows(a.after(), b.after());
                    let mut change = Vec::new();
                    semi(a, b, &on, neg, &mut |s, x| {
                        kept_moved += usize::from(kept(a, x));
                        let t = if s == Sign::Gone { a.before().unwrap() } else { a.after() };
                        change.push((s, vec![Read::get(t, x, c(0)), Read::get(t, x, c(1))]));
                    });
                    let out = applied.get_or_insert_with(|| before.iter().cloned().collect());
                    for (s, row) in &change {
                        match s {
                            Sign::Gone => {
                                out.remove(row);
                            }
                            Sign::Added => {
                                out.insert(row.clone());
                            }
                        }
                    }
                    assert_eq!(*out, after.iter().cloned().collect(), "state {k}");
                    check(Some(before), after, change);
                });
                assert!(kept_moved > 0, "some kept row went or came with its match ({neg}, {on:?})");
            }
        }
    }

    /// A grouped aggregate's change keeps L1 and L3 for every aggregate: each key the change touches, its old row
    /// gone and its new one added.
    #[test]
    fn a_groups_change_keeps_the_laws() {
        let aggs = [Agg::Count, Agg::Sum(c(1)), Agg::Min(c(1)), Agg::Max(c(1)), Agg::MaxBelow { col: c(1), bound: 5, or_eq: false }, Agg::MinAbove { col: c(1), bound: 3, or_eq: true }];
        for agg in aggs {
            let mut applied: Option<std::collections::BTreeSet<Vec<i64>>> = None;
            each_state(0, 0, 12, |k, a, _| {
                let grouped = |t: &View| -> Vec<Vec<i64>> {
                    let mut keys: Vec<i64> = rows_of(t).iter().map(|r| r[0]).collect();
                    keys.sort_unstable();
                    keys.dedup();
                    keys.into_iter().filter_map(|key| fold(t, &[c(0)], &[key], agg).map(|v| vec![key, v])).collect()
                };
                let before = grouped(a.before().unwrap());
                let after = grouped(a.after());
                let mut change = Vec::new();
                group(a, &[c(0)], agg, &mut |s, key, v| change.push((s, vec![key[0], v])));
                let out = applied.get_or_insert_with(|| before.iter().cloned().collect());
                for (s, row) in &change {
                    if *s == Sign::Gone {
                        out.remove(row);
                    }
                }
                for (s, row) in &change {
                    if *s == Sign::Added {
                        out.insert(row.clone());
                    }
                }
                assert_eq!(*out, after.iter().cloned().collect(), "state {k}: {agg:?}");
                check(Some(before), after, change);
            });
        }
    }

    /// A cold run is a change from nothing: every output row added.
    #[test]
    fn a_cold_runs_change_is_its_whole_output() {
        let ch = Chain::new(0, &[&[1]]);
        let ra = ch.full(&[[1, 2], [3, 4], [3, 5]]);
        let rb = ch.full(&[[3, 9], [1, 1], [7, 7]]);
        let (a, b) = (View::full(ra), View::full(rb));
        let (sa, sb) = (a.bistate(), b.bistate());
        let mut n = 0;
        join(&sa, &sb, &[(c(0), c(0))], &mut |s, _, _| {
            assert_eq!(s, Sign::Added);
            n += 1;
        });
        assert_eq!(n, 3);
    }
}
