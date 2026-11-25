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
//!
//! Decision 0678 step 9: the operations whose change would otherwise read more than the change keep counts, each a
//! relation in a block with a version and a delta like any other (`State`, `settle`): a projecting head its rows'
//! derivation counts (`project`), a count or sum each group's count and sum (`counted`), an extreme each (group,
//! value)'s count (`extreme`). Each costs O(1) a row changed; an extreme reads its group's distinct values once, when
//! its extreme goes. `group` keeps nothing and reads each touched group whole. A recursive head keeps no count (a
//! cycle can hold its own count up): its levels decide (decision 0364).

use crate::bistate::{Bistate, Read};
use crate::ids::{ColumnIndex, RowId};
use crate::relation::{RelW, Relation, View};
use crate::table::{Agg, Table, count, fold};
use rustc_hash::{FxHashMap, FxHashSet};

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
/// touches are read, each in both states: a key's every row, so its cost is the touched groups' sizes. It keeps
/// nothing; `counted` and `extreme` keep counts and cost the change.
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

/// A change's sums by key (decision 0678 step 9): for each key the change touches, once, the signed sum of its rows'
/// accumulators, a count first, then a sum of each summed column. Only the change's keys are held.
pub struct Tally {
    width: usize,
    accs: usize,
    at: FxHashMap<Box<[i64]>, Box<[i64]>>,
}

impl Tally {
    /// A tally of keys of `width` values, each with `accs` accumulators (a count, then `accs - 1` sums).
    pub fn new(width: usize, accs: usize) -> Tally {
        assert!(accs >= 1, "a count first");
        Tally { width, accs, at: FxHashMap::default() }
    }

    /// A row of key `key` gone or added, its summed values `sums` (`accs - 1` of them).
    pub fn add(&mut self, sign: Sign, key: &[i64], sums: &[i64]) {
        debug_assert_eq!((key.len(), sums.len() + 1), (self.width, self.accs));
        let d = if sign == Sign::Added { 1 } else { -1 };
        let e = self.at.entry(key.into()).or_insert_with(|| vec![0; self.accs].into());
        e[0] += d;
        for (a, &v) in e[1..].iter_mut().zip(sums) {
            *a = a.wrapping_add(v.wrapping_mul(d));
        }
    }

    /// `a`'s change onto columns `key`, with columns `sums` summed: each gone row less, each added row more; a row
    /// `keep` refuses (a bounded extreme's row out of bounds) left out.
    pub fn of<A: Table>(a: &Bistate<A>, key: &[ColumnIndex], sums: &[ColumnIndex], keep: &dyn Fn(&A, RowId) -> bool) -> Tally {
        let mut t = Tally::new(key.len(), sums.len() + 1);
        let (mut k, mut v) = (vec![0i64; key.len()], vec![0i64; sums.len()]);
        let mut put = |t: &mut Tally, s: Sign, tab: &A, id: RowId| {
            if keep(tab, id) {
                read_key(tab, id, key, &mut k);
                read_key(tab, id, sums, &mut v);
                t.add(s, &k, &v);
            }
        };
        if let Some(b) = a.before() {
            for id in a.gone() {
                put(&mut t, Sign::Gone, b, id);
            }
        }
        for id in a.added() {
            put(&mut t, Sign::Added, a.after(), id);
        }
        t
    }

    /// The keys it holds.
    pub fn len(&self) -> usize {
        self.at.len()
    }

    pub fn is_empty(&self) -> bool {
        self.at.is_empty()
    }
}

/// A kept state of counts (decision 0678 step 9): `rel`, a frozen full relation of a tally's keys and their
/// accumulators (`width + accs` columns, a key once, its count above 0; none before the first change), whose index
/// `by_key` is on its key's columns in order.
#[derive(Clone, Copy)]
pub struct State<'a> {
    pub rel: Option<&'a Relation>,
    pub by_key: usize,
}

/// What `settle` tells of each key whose accumulators changed: the key, its accumulators before and after (`None`:
/// no row).
pub type Settled<'f> = dyn FnMut(&[i64], Option<&[i64]>, Option<&[i64]>) + 'f;

/// Settles tally `t` against kept state `kept`. Each key's row is found by one probe; where its accumulators change, the row is killed and its new one
/// pushed into `w` (a delta of `kept`), and a key whose count reaches 0 has none: the state's change, in the
/// relation's block (0353: a derived fact written once). `each(key, old, new)` for each key whose accumulators
/// changed: before and after, `None` for no row. So a key's count is the number of rows (or derivations) giving it.
pub fn settle(kept: State, t: &Tally, w: &RelW, each: &mut Settled) {
    let (view, ix) = (kept.rel.map(View::full), kept.by_key);
    let mut row = vec![0i64; t.width + t.accs];
    let mut old = vec![0i64; t.accs];
    for (key, d) in &t.at {
        let found = view.as_ref().and_then(|v| v.probe_iter(ix, key).next());
        if let (Some(v), Some(id)) = (&view, found) {
            for (j, o) in old.iter_mut().enumerate() {
                *o = v.get(id, t.width + j);
            }
        } else {
            old.fill(0);
        }
        // copy: derived (the key's new row of counts, written once into the state's delta)
        row[..t.width].copy_from_slice(key);
        for j in 0..t.accs {
            row[t.width + j] = old[j].wrapping_add(d[j]);
        }
        let new = &row[t.width..];
        assert!(new[0] >= 0, "{key:?}: a count below 0 (a row gone that was never counted)");
        if new == &old[..] {
            continue;
        }
        if let Some(id) = found {
            w.kill(id);
        }
        if new[0] > 0 {
            w.push(&row);
        }
        each(key, found.map(|_| &old[..]), (new[0] > 0).then_some(&row[t.width..]));
    }
}

/// A projecting head's change by derivation counts (decision 0678 step 9): `t` its derivations' change onto the
/// head's columns (one accumulator, the count), `kept` each head row's count (`settle`'s state, its delta written into
/// `w`). A head row is gone when its count reaches 0 and added when it leaves 0: O(1) a derivation gained or lost,
/// with no search for another derivation (0328's recheck searches the head's other derivations).
pub fn project(kept: State, t: &Tally, w: &RelW, each: &mut dyn FnMut(Sign, &[i64])) {
    settle(kept, t, w, &mut |key, old, new| match (old, new) {
        (Some(_), None) => each(Sign::Gone, key),
        (None, Some(_)) => each(Sign::Added, key),
        _ => {}
    });
}

/// A count's or a sum's change by group (`agg` `Count`, or `Sum` with a tally of a count and that sum): each group's
/// count and sum kept (`settle`'s state, its delta written into `w`), so a row gone or added costs O(1), never a read
/// of its group. A group whose value changed: its old row gone and its new one added; a group of no rows has none.
pub fn counted(kept: State, t: &Tally, agg: Agg, w: &RelW, each: &mut dyn FnMut(Sign, &[i64], i64)) {
    let at = match agg {
        Agg::Count => 0,
        Agg::Sum(_) => 1,
        _ => panic!("{agg:?}: an extreme is `extreme`'s"),
    };
    settle(kept, t, w, &mut |key, old, new| {
        let (o, n) = (old.map(|a| a[at]), new.map(|a| a[at]));
        if o == n {
            return;
        }
        if let Some(v) = o {
            each(Sign::Gone, key, v);
        }
        if let Some(v) = n {
            each(Sign::Added, key, v);
        }
    });
}

/// An extreme's change by group (`agg` `Min`, `Max`, `MinAbove`, `MaxBelow`): `t` the change's rows onto (group,
/// value), the rows out of bounds left out (`Tally::of`'s `keep`), and `kept` a count of each (group, value)
/// (`settle`'s state, its index `by_key` on both; `by_group` its index on the group's columns; its delta written
/// into `w`).
/// `out` is the output before (group, then its extreme), probed on the group's columns.
///
/// A group's new extreme is the old one or a value the change gave it, O(1) a row, while the old one's count stays
/// above 0. When it reaches 0, the group's values are read by one probe of `kept` (its distinct values, not its rows).
pub fn extreme<O: Table>(kept: State, by_group: usize, t: &Tally, out: Option<&O>, agg: Agg, w: &RelW, each: &mut dyn FnMut(Sign, &[i64], i64)) {
    let max = match agg {
        Agg::Max(_) | Agg::MaxBelow { .. } => true,
        Agg::Min(_) | Agg::MinAbove { .. } => false,
        _ => panic!("{agg:?}: a count or a sum is `counted`'s"),
    };
    let g = t.width - 1;
    let better = |x: i64, y: i64| if max { x > y } else { x < y };
    // Each touched group's values that came (from a count of 0) and went (to 0).
    let mut groups: FxHashMap<Box<[i64]>, Moved> = FxHashMap::default();
    settle(kept, t, w, &mut |key, old, new| {
        let e = groups.entry(key[..g].into()).or_default();
        match (old, new) {
            (None, Some(_)) => e.came.push(key[g]),
            (Some(_), None) => e.went.push(key[g]),
            _ => {}
        }
    });
    let gcols: Vec<ColumnIndex> = (0..g as u16).map(ColumnIndex::new).collect();
    let view = kept.rel.map(View::full);
    for (group, Moved { came, went }) in groups {
        let mut old = None;
        if let Some(o) = out {
            o.probe(&gcols, &group, &mut |id| {
                old = Some(o.get(id, ColumnIndex::new(g as u16)));
                false
            });
        }
        let best = |vals: &mut dyn Iterator<Item = i64>| vals.reduce(|a, b| if better(b, a) { b } else { a });
        let gained = best(&mut came.iter().copied());
        let new = match old {
            Some(v) if !went.contains(&v) => Some(gained.filter(|&x| better(x, v)).unwrap_or(v)),
            // The extreme went (or there was none): the group's values kept, less those gone, and those come.
            _ => {
                let kept_vals = view.iter().flat_map(|v| v.probe_iter(by_group, &group).map(|id| v.get(id, g)));
                let mut all = kept_vals.filter(|x| !went.contains(x)).chain(came.iter().copied());
                best(&mut all)
            }
        };
        if old == new {
            continue;
        }
        if let Some(v) = old {
            each(Sign::Gone, &group, v);
        }
        if let Some(v) = new {
            each(Sign::Added, &group, v);
        }
    }
}

/// A group's values that came into its kept counts (from 0) and went (to 0) in one change.
#[derive(Default)]
struct Moved {
    came: Vec<i64>,
    went: Vec<i64>,
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

/// A value a part's column is matched with (`body`): a variable, a constant, or any value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Term {
    Var(usize),
    Const(i64),
    Any,
}

/// A comparison of two values (`Part::Test`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Cmp {
    pub fn holds(self, a: i64, b: i64) -> bool {
        match self {
            Cmp::Eq => a == b,
            Cmp::Ne => a != b,
            Cmp::Lt => a < b,
            Cmp::Le => a <= b,
            Cmp::Gt => a > b,
            Cmp::Ge => a >= b,
        }
    }
}

/// One part of a conjunctive body (`body`).
pub enum Part<'a, T> {
    /// The rows of a relation whose columns match the terms: a join on the variables bound before it, binding the
    /// rest.
    Join(&'a Bistate<T>, &'a [Term]),
    /// No row of a relation matches the terms (each variable bound before it, `Any` matching anything): an anti-join.
    Not(&'a Bistate<T>, &'a [Term]),
    /// Two bound values compared: a filter.
    Test(Term, Cmp, Term),
}

/// Which state a body's parts read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Before,
    After,
}

/// How `body` reads its parts: every part on one side (a cold run's derivations, `All(After)`), or the term of part
/// `i`'s change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Body {
    All(Side),
    Term(usize),
}

/// A derivation of a body (`body`): each part's row, and each variable's value, read through the row that bound it
/// where that row lies.
pub struct Tuple<'t, 'a, T> {
    rows: &'t [Option<(&'a T, RowId)>],
    binder: &'t [Option<(usize, ColumnIndex)>],
}

impl<T: Read> Tuple<'_, '_, T> {
    /// Part `p`'s row: a join's; a negation's whose change bound its variables (`Term`); none for the others.
    pub fn row(&self, p: usize) -> Option<RowId> {
        self.rows[p].map(|r| r.1)
    }

    /// Variable `v`'s value.
    pub fn var(&self, v: usize) -> i64 {
        let (p, c) = self.binder[v].expect("a bound variable");
        let (t, id) = self.rows[p].expect("a bound part");
        t.get(id, c)
    }

    /// A term's value; `None` for `Any`.
    pub fn value(&self, t: Term) -> Option<i64> {
        match t {
            Term::Var(v) => Some(self.var(v)),
            Term::Const(c) => Some(c),
            Term::Any => None,
        }
    }
}

/// The change of a conjunctive body of `k` parts over `vars` variables (decision 0686: 0678 step 5's join, from two
/// inputs to `k`). Its derivations are the tuples of rows, one a join part, that agree on every variable and pass
/// every negation and test. `how` says which are given to `each`, with a sign:
///
/// - `All(side)`: every derivation with every part read on `side` (a cold run's: `All(After)`), each added;
/// - `Term(i)`: part `i`'s term of the change. With `f_j` part `j`'s factor for a binding (a join's match, a
///   negation's absence of one), the body's change telescopes:
///
///   Π f_j(after) − Π f_j(before) = Σ_i [Π_{j<i} f_j(after)] · (f_i(after) − f_i(before)) · [Π_{j>i} f_j(before)]
///
///   so term `i` reads part `i`'s change, the parts before it after and the parts after it before. A join's change is
///   its rows gone (read before, sign gone) and added (read after, sign added); a negation's is each key its
///   relation's change touches, once, where the key's absence changed; a test never changes and has no term. The
///   terms' sum over the parts whose input changed is the body's change: each derivation gone or added, a derivation
///   counted once a term it is in (L1, by the identity above). A part whose input did not change has no term (L4).
///
/// Term `i` reads part `i` first and the others in order after it; each variable is bound by its first occurrence
/// in that order and tested at the rest. A negation or test reached before one of its variables is bound is a
/// misuse (the caller refuses such a body), except part `i` itself, whose change binds them.
pub fn body<T: Table>(parts: &[Part<'_, T>], vars: usize, how: Body, each: &mut dyn FnMut(Sign, &Tuple<'_, '_, T>)) {
    let order: Vec<usize> = match how {
        Body::All(_) => (0..parts.len()).collect(),
        Body::Term(i) => std::iter::once(i).chain((0..parts.len()).filter(|&j| j != i)).collect(),
    };
    if let Body::Term(i) = how {
        if matches!(parts[i], Part::Test(..)) {
            return;
        }
    }
    let mut w = Walk { parts, how, order, rows: vec![None; parts.len()], binder: vec![None; vars], each };
    w.at(0, 1);
}

struct Walk<'p, 'a, 'e, T> {
    parts: &'p [Part<'a, T>],
    how: Body,
    order: Vec<usize>,
    rows: Vec<Option<(&'a T, RowId)>>,
    binder: Vec<Option<(usize, ColumnIndex)>>,
    each: &'e mut dyn FnMut(Sign, &Tuple<'_, 'a, T>),
}

impl<'a, T: Table> Walk<'_, 'a, '_, T> {
    /// A term's value now: a constant's, a bound variable's; `None` for `Any` and an unbound variable.
    fn value(&self, t: Term) -> Option<i64> {
        match t {
            Term::Const(c) => Some(c),
            Term::Any => None,
            Term::Var(v) => self.binder[v].map(|(p, c)| {
                let (t, id) = self.rows[p].expect("a bound part");
                t.get(id, c)
            }),
        }
    }

    /// The state part `p` reads, when it is not the term's changed part: after before the changed part, before after
    /// it (`None`: a cold relation's before, no rows).
    fn state(&self, p: usize, b: &'a Bistate<T>) -> Option<&'a T> {
        let after = match self.how {
            Body::All(s) => s == Side::After,
            Body::Term(i) => p < i,
        };
        if after { Some(b.after()) } else { b.before() }
    }

    /// Whether row `id` of `t` agrees with `args` given what is bound; the variables it binds (first occurrences in
    /// it, each with its column) are pushed to `binds`.
    fn agrees(&self, t: &T, id: RowId, args: &[Term], binds: &mut Vec<(usize, ColumnIndex)>) -> bool {
        binds.clear();
        for (c, &a) in args.iter().enumerate() {
            let c = ColumnIndex::new(c as u16);
            let v = t.get(id, c);
            match a {
                Term::Any => {}
                Term::Const(k) => {
                    if v != k {
                        return false;
                    }
                }
                Term::Var(x) => {
                    if let Some(b) = self.value(a) {
                        if b != v {
                            return false;
                        }
                    } else if let Some(&(_, first)) = binds.iter().find(|b| b.0 == x) {
                        if t.get(id, first) != v {
                            return false;
                        }
                    } else {
                        binds.push((x, c));
                    }
                }
            }
        }
        true
    }

    /// Row `id` of `t` as part `p`'s, its variables bound, then the parts after it.
    fn take(&mut self, pos: usize, sign: i64, p: usize, t: &'a T, id: RowId, args: &[Term]) {
        let mut binds = Vec::new();
        if !self.agrees(t, id, args, &mut binds) {
            return;
        }
        self.rows[p] = Some((t, id));
        for &(x, c) in &binds {
            self.binder[x] = Some((p, c));
        }
        self.at(pos + 1, sign);
        for &(x, _) in &binds {
            self.binder[x] = None;
        }
        self.rows[p] = None;
    }

    /// The bound columns of `args` and their values.
    fn key(&self, args: &[Term]) -> (Vec<ColumnIndex>, Vec<i64>) {
        args.iter().enumerate().filter_map(|(c, &a)| self.value(a).map(|v| (ColumnIndex::new(c as u16), v))).unzip()
    }

    /// Whether `t` (`None`: no rows) has no live row whose `cols` are `key`.
    fn none(t: Option<&T>, cols: &[ColumnIndex], key: &[i64]) -> bool {
        t.is_none_or(|t| t.probe(cols, key, &mut |_| false))
    }

    /// A negation's term: each key of `ids` (rows of `t`, a side of `b`'s change) not in `seen`, where the key's
    /// absence from `b` changed, bound through its row; the parts after it run with the absence's change as sign.
    #[allow(clippy::too_many_arguments)]
    fn touched(&mut self, pos: usize, sign: i64, p: usize, b: &'a Bistate<T>, t: &'a T, ids: impl Iterator<Item = RowId>, args: &[Term], seen: &mut FxHashSet<Vec<i64>>) {
        let cols: Vec<ColumnIndex> = args.iter().enumerate().filter(|(_, a)| **a != Term::Any).map(|(c, _)| ColumnIndex::new(c as u16)).collect();
        let mut key = vec![0i64; cols.len()];
        let mut binds = Vec::new();
        for id in ids {
            if !self.agrees(t, id, args, &mut binds) {
                continue;
            }
            read_key(t, id, &cols, &mut key);
            // copy: not block data (a touched key, held once to read it once, as `touched`'s)
            if !seen.insert(key.clone()) {
                continue;
            }
            let f = i64::from(Self::none(Some(b.after()), &cols, &key)) - i64::from(Self::none(b.before(), &cols, &key));
            if f == 0 {
                continue;
            }
            self.rows[p] = Some((t, id));
            for &(x, c) in &binds {
                self.binder[x] = Some((p, c));
            }
            self.at(pos + 1, sign * f);
            for &(x, _) in &binds {
                self.binder[x] = None;
            }
            self.rows[p] = None;
        }
    }

    fn at(&mut self, pos: usize, sign: i64) {
        let Some(&p) = self.order.get(pos) else {
            let tuple = Tuple { rows: &self.rows, binder: &self.binder };
            (self.each)(if sign > 0 { Sign::Added } else { Sign::Gone }, &tuple);
            return;
        };
        let changed = self.how == Body::Term(p);
        match self.parts[p] {
            Part::Join(b, args) => {
                if changed {
                    if let Some(before) = b.before() {
                        for id in b.gone() {
                            self.take(pos, -sign, p, before, id, args);
                        }
                    }
                    for id in b.added() {
                        self.take(pos, sign, p, b.after(), id, args);
                    }
                    return;
                }
                let Some(t) = self.state(p, b) else { return };
                let (cols, key) = self.key(args);
                if cols.is_empty() {
                    for id in t.live_ids() {
                        self.take(pos, sign, p, t, id, args);
                    }
                } else {
                    t.probe(&cols, &key, &mut |id| {
                        self.take(pos, sign, p, t, id, args);
                        true
                    });
                }
            }
            Part::Not(b, args) => {
                if !changed {
                    let (cols, key) = self.key(args);
                    assert!(args.iter().all(|&a| a == Term::Any || self.value(a).is_some()), "a negation's variables bound before it");
                    if Self::none(self.state(p, b), &cols, &key) {
                        self.at(pos + 1, sign);
                    }
                    return;
                }
                // Each key the change touches, once, bound through a row of the change that has it.
                let mut seen: FxHashSet<Vec<i64>> = FxHashSet::default();
                if let Some(before) = b.before() {
                    self.touched(pos, sign, p, b, before, b.gone(), args, &mut seen);
                }
                self.touched(pos, sign, p, b, b.after(), b.added(), args, &mut seen);
            }
            Part::Test(a, op, b) => {
                let (x, y) = (self.value(a).expect("a test's terms bound"), self.value(b).expect("a test's terms bound"));
                if op.holds(x, y) {
                    self.at(pos + 1, sign);
                }
            }
        }
    }
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

    /// A kept state of counts (decision 0678 step 9) across a chain: a version (a full relation) and the delta a
    /// change settles into it, rolled up into the next version.
    struct Counts {
        block: Block,
        spec: Spec,
        lock: Mutex<()>,
    }

    impl Counts {
        /// Rows of `arity` columns, indexed on each of `indexes`.
        fn new(arity: usize, indexes: &[&[u16]]) -> Counts {
            let spec = Spec { widths: vec![8; arity], sort: None, indexes: indexes.iter().map(|k| k.to_vec()).collect(), min_log: crate::rows::MIN_LOG };
            Counts { block: Block::anon(kind::FACTS, 64 << 20), spec, lock: Mutex::new(()) }
        }

        /// A delta of `kept` (none: a full relation), its writer given to `f`, then frozen.
        fn change(&self, kept: Option<&Relation>, f: impl FnOnce(&RelW)) -> &Relation {
            let d = Relation::create(&self.block, &self.spec, kept);
            f(&RelW { block: &self.block, rel: d, lock: &self.lock });
            relation::freeze(&self.block, d, &self.spec, kept);
            d
        }

        /// The state after: `kept` and its delta `d` rolled up into a full relation.
        fn roll<'a>(&'a self, kept: Option<&'a Relation>, d: &'a Relation) -> &'a Relation {
            let Some(k) = kept else { return d };
            let v = View { base: Some(k), top: d };
            let rows: Vec<Vec<i64>> = v.ordered().into_iter().map(|id| v.row(id)).collect();
            self.change(None, |w| rows.iter().for_each(|r| _ = w.push(r)))
        }
    }

    fn live_rows(r: &Relation) -> std::collections::BTreeSet<Vec<i64>> {
        let v = View::full(r);
        v.ordered().into_iter().map(|id| v.row(id)).collect()
    }

    /// A chain of one relation's states, each its bistate, with the tally of its version's rows (a change from
    /// nothing) given first.
    fn each_change(steps: usize, mut f: impl FnMut(usize, &Bistate<View<'_>>)) {
        let ca = Chain::new(0, &[&[0], &[1]]);
        for (k, (v, add, kill)) in states(7, steps, 12).iter().enumerate() {
            let base = ca.full(v);
            let d = ca.delta(base, add, kill);
            if k == 0 {
                f(usize::MAX, &View::full(base).bistate());
            }
            f(k, &View { base: Some(base), top: d }.bistate());
        }
    }

    /// Decision 0678 step 9. A head projecting a column away (the second column of `A`'s rows) changes by its
    /// derivation counts: a head row goes only when its last derivation does. L1 at every state and L3 over the
    /// chain; the kept counts are the cold counts of every state after.
    #[test]
    fn a_projections_change_by_derivation_counts_keeps_the_laws() {
        let counts = Counts::new(2, &[&[0]]);
        let mut kept: Option<&Relation> = None;
        let mut applied = std::collections::BTreeSet::<Vec<i64>>::new();
        each_change(12, |k, a| {
            let distinct = |t: Option<&View>| -> Vec<Vec<i64>> {
                let mut v: Vec<Vec<i64>> = t.map(|t| rows_of(t).iter().map(|r| vec![r[1]]).collect()).unwrap_or_default();
                v.sort();
                v.dedup();
                v
            };
            let t = Tally::of(a, &[c(1)], &[], &|_, _| true);
            let mut change = Vec::new();
            let d = counts.change(kept, |w| project(super::State { rel: kept, by_key: 0 }, &t, w, &mut |s, key| change.push((s, key.to_vec()))));
            kept = Some(counts.roll(kept, d));
            for (s, row) in &change {
                if *s == Sign::Gone {
                    applied.remove(row);
                } else {
                    applied.insert(row.clone());
                }
            }
            assert_eq!(applied.iter().cloned().collect::<Vec<_>>(), distinct(Some(a.after())), "state {k}: applied");
            check(Some(distinct(a.before())), distinct(Some(a.after())), change);
            let mut cold = std::collections::BTreeMap::<i64, i64>::new();
            for r in rows_of(a.after()) {
                *cold.entry(r[1]).or_default() += 1;
            }
            let want: std::collections::BTreeSet<Vec<i64>> = cold.into_iter().map(|(v, n)| vec![v, n]).collect();
            assert_eq!(live_rows(kept.unwrap()), want, "state {k}: each head row's derivations counted");
        });
    }

    /// Decision 0678 step 9. Every aggregate's change from kept counts keeps L1 and L3 and agrees with `group`'s cold
    /// fold at every state: a count and a sum from each group's count and sum, an extreme (bounded or not) from each
    /// (group, value)'s count.
    #[test]
    fn an_aggregates_change_from_kept_counts_keeps_the_laws() {
        let aggs = [Agg::Count, Agg::Sum(c(1)), Agg::Min(c(1)), Agg::Max(c(1)), Agg::MaxBelow { col: c(1), bound: 5, or_eq: false }, Agg::MinAbove { col: c(1), bound: 3, or_eq: true }];
        for agg in aggs {
            let extreme_of = !matches!(agg, Agg::Count | Agg::Sum(_));
            // (group, value, count) for an extreme; (group, count) for a count; (group, count, sum) for a sum.
            let counts = match agg {
                Agg::Count => Counts::new(2, &[&[0]]),
                Agg::Sum(_) => Counts::new(3, &[&[0]]),
                _ => Counts::new(3, &[&[0, 1], &[0]]),
            };
            let outs = Counts::new(2, &[&[0]]);
            let (mut kept, mut out): (Option<&Relation>, Option<&Relation>) = (None, None);
            let mut extremes = 0;
            each_change(12, |k, a| {
                let grouped = |t: Option<&View>| -> Vec<Vec<i64>> {
                    let Some(t) = t else { return Vec::new() };
                    let mut keys: Vec<i64> = rows_of(t).iter().map(|r| r[0]).collect();
                    keys.sort_unstable();
                    keys.dedup();
                    keys.into_iter().filter_map(|key| fold(t, &[c(0)], &[key], agg).map(|v| vec![key, v])).collect()
                };
                let before = if k == usize::MAX { Vec::new() } else { grouped(a.before()) };
                let after = grouped(Some(a.after()));
                let mut change = Vec::new();
                let d = if extreme_of {
                    let inside = |t: &View, id: RowId| {
                        let v = Read::get(t, id, c(1));
                        match agg {
                            Agg::MaxBelow { bound, or_eq, .. } => v < bound || (or_eq && v == bound),
                            Agg::MinAbove { bound, or_eq, .. } => v > bound || (or_eq && v == bound),
                            _ => true,
                        }
                    };
                    let t = Tally::of(a, &[c(0), c(1)], &[], &inside);
                    let o = out.map(View::full);
                    counts.change(kept, |w| extreme(super::State { rel: kept, by_key: 0 }, 1, &t, o.as_ref(), agg, w, &mut |s, key, v| change.push((s, vec![key[0], v]))))
                } else {
                    let sums: &[ColumnIndex] = if agg == Agg::Count { &[] } else { &[c(1)] };
                    let t = Tally::of(a, &[c(0)], sums, &|_, _| true);
                    counts.change(kept, |w| counted(super::State { rel: kept, by_key: 0 }, &t, agg, w, &mut |s, key, v| change.push((s, vec![key[0], v]))))
                };
                kept = Some(counts.roll(kept, d));
                let od = outs.change(out, |w| {
                    let ov = out.map(View::full);
                    for (s, row) in &change {
                        match s {
                            Sign::Gone => {
                                let v = ov.as_ref().expect("gone, so in the output before");
                                let id = v.probe_iter(0, &row[..1]).find(|&id| v.row(id) == *row).expect("in the output before");
                                w.kill(id);
                            }
                            Sign::Added => _ = w.push(row),
                        }
                    }
                });
                out = Some(outs.roll(out, od));
                extremes += change.len();
                assert_eq!(live_rows(out.unwrap()), after.iter().cloned().collect(), "state {k}: {agg:?}, applied");
                check(Some(before), after, change);
            });
            assert!(extremes > 0, "{agg:?}: the chain changed it");
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

    /// A body's derivations on one side, by brute force: every live row of every join part, in order, kept where it
    /// agrees; each derivation's variables' values, counted.
    fn brute<T: Table>(parts: &[Part<'_, T>], vars: usize, side: Side) -> std::collections::BTreeMap<Vec<i64>, i64> {
        fn go<T: Table>(parts: &[Part<'_, T>], side: Side, vals: &mut Vec<Option<i64>>, out: &mut std::collections::BTreeMap<Vec<i64>, i64>) {
            let Some((first, rest)) = parts.split_first() else {
                *out.entry(vals.iter().map(|v| v.expect("bound")).collect()).or_default() += 1;
                return;
            };
            let state = |b: &Bistate<T>| -> Option<*const T> { if side == Side::After { Some(b.after() as *const T) } else { b.before().map(|t| t as *const T) } };
            let fits = |t: &T, id: RowId, args: &[Term], vals: &mut Vec<Option<i64>>| -> bool {
                for (c, &a) in args.iter().enumerate() {
                    let v = t.get(id, ColumnIndex::new(c as u16));
                    match a {
                        Term::Any => {}
                        Term::Const(k) if k != v => return false,
                        Term::Const(_) => {}
                        Term::Var(x) => match vals[x] {
                            Some(b) if b != v => return false,
                            Some(_) => {}
                            None => vals[x] = Some(v),
                        },
                    }
                }
                true
            };
            match first {
                Part::Join(b, args) => {
                    let Some(t) = state(b) else { return };
                    // SAFETY: a state of `b`, alive while `parts` is.
                    let t = unsafe { &*t };
                    for id in t.live_ids() {
                        let saved = vals.clone();
                        if fits(t, id, args, vals) {
                            go(rest, side, vals, out);
                        }
                        *vals = saved;
                    }
                }
                Part::Not(b, args) => {
                    let mut any = false;
                    if let Some(t) = state(b) {
                        // SAFETY: as above.
                        let t = unsafe { &*t };
                        for id in t.live_ids() {
                            let saved = vals.clone();
                            any |= fits(t, id, args, vals);
                            *vals = saved;
                        }
                    }
                    if !any {
                        go(rest, side, vals, out);
                    }
                }
                Part::Test(a, op, b) => {
                    let v = |t: Term| match t {
                        Term::Var(x) => vals[x].expect("bound"),
                        Term::Const(k) => k,
                        Term::Any => unreachable!(),
                    };
                    if op.holds(v(*a), v(*b)) {
                        go(rest, side, vals, out);
                    }
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        go(parts, side, &mut vec![None; vars], &mut out);
        out
    }

    /// A body's change, the sum of its parts' terms, keeps L1 at every state of two relations' chains, on bodies of
    /// joins, negations and tests, with constants, wildcards and a variable twice in one part; and its cold run is
    /// its derivations after.
    #[test]
    fn a_bodys_change_is_the_sum_of_its_terms() {
        use Term::{Any, Const, Var};
        let shapes: Vec<(usize, Vec<(u8, usize, Vec<Term>)>)> = vec![
            // (kind: 0 join, 1 not, 2 test; relation 0 = a, 1 = b, or for a test the op; terms)
            (3, vec![(0, 0, vec![Var(0), Var(1)]), (0, 1, vec![Var(1), Var(2)]), (1, 0, vec![Var(2), Any]), (2, 1, vec![Var(0), Var(2)])]),
            (3, vec![(0, 0, vec![Var(0), Var(1)]), (1, 1, vec![Var(0), Var(1)]), (0, 1, vec![Var(0), Var(2)])]),
            (2, vec![(0, 0, vec![Var(0), Var(1)]), (0, 0, vec![Var(1), Var(0)])]),
            (1, vec![(0, 1, vec![Var(0), Var(0)])]),
            (2, vec![(0, 1, vec![Var(0), Const(3)]), (0, 0, vec![Var(0), Var(1)]), (2, 3, vec![Var(1), Const(4)]), (1, 1, vec![Var(1), Any])]),
        ];
        let ops = [Cmp::Eq, Cmp::Ne, Cmp::Lt, Cmp::Le, Cmp::Gt, Cmp::Ge];
        for (n, (vars, shape)) in shapes.iter().enumerate() {
            each_state(0, 1, 12, |k, a, b| {
                let rels = [a, b];
                let parts: Vec<Part<'_, View<'_>>> = shape
                    .iter()
                    .map(|(kind, r, t)| match kind {
                        0 => Part::Join(rels[*r], t),
                        1 => Part::Not(rels[*r], t),
                        _ => Part::Test(t[0], ops[*r], t[1]),
                    })
                    .collect();
                let (before, after) = (brute(&parts, *vars, Side::Before), brute(&parts, *vars, Side::After));
                let mut applied = before.clone();
                for i in 0..parts.len() {
                    body(&parts, *vars, Body::Term(i), &mut |s, t| {
                        let row: Vec<i64> = (0..*vars).map(|v| t.var(v)).collect();
                        *applied.entry(row).or_default() += if s == Sign::Added { 1 } else { -1 };
                    });
                }
                applied.retain(|_, n| *n != 0);
                assert_eq!(applied, after, "shape {n}, state {k}: the terms' sum is the change");
                let mut cold = std::collections::BTreeMap::new();
                body(&parts, *vars, Body::All(Side::After), &mut |s, t| {
                    assert_eq!(s, Sign::Added);
                    *cold.entry((0..*vars).map(|v| t.var(v)).collect::<Vec<i64>>()).or_default() += 1;
                });
                assert_eq!(cold, after, "shape {n}, state {k}: the cold run");
            });
        }
    }
}
