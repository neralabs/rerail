//! Lowering: the delta-operator DAG (`norm::Graph`) becomes one timely/differential dataflow over
//! tagged records `(tag, row)` (tags: `gen::tags`). Every operator below is the generic base +
//! delta expansion of its relational operator; no rule-specific code exists here.
//!
//! Delta mode (outside recursion), a record `(t, row)` is a signed difference of view `t` from
//! its parent's view:
//! - map, filter: per record, tag kept (linear).
//! - join: one `join_core` over two arrangements; a pair (t1, t2) yields `tags::product(t1, t2)`
//!   (bilinear: A_B⋈B_B, ΔA⋈B_B, A_B⋈ΔB, ΔA⋈ΔB; nothing across worktrees).
//! - presence (DISTINCT, the right side of semi/antijoins): per key, counts per tag; the base
//!   emits [c_0 > 0]; tag t emits [below + d_t > 0] − [below > 0] with `below` the count of t's
//!   parent view (count-based, exact under retractions).
//! - semijoin = left ⋈ presence(right keys); antijoin = left − semijoin (bilinear in ±1).
//! - group recompute (GROUP BY, windows, DISTINCT on a prefix): per key, out(t) = logic(view(t))
//!   − logic(view(parent(t))) for every tag with members at the key; the base emits logic(base).
//! - fixpoint: differential `iterate` (semi-naive) in delta mode: the body is the same base +
//!   delta expansion at the iteration's timestamp, each round's distinct the count-based presence;
//!   a base load is a plain semi-naive evaluation, a worktree iterates only its differences.
use super::norm::{Graph, Key, KeyMode, NodeId, Op, X};
use super::tags::{self, W, BASE};
use super::val::{self, Kind, Row, Val};
use differential_dataflow::input::{Input, InputSession};
use differential_dataflow::operators::arrange::{Arranged, TraceAgent};
use differential_dataflow::trace::wrappers::enter::TraceEnter;
use differential_dataflow::operators::iterate::Iterate;
use differential_dataflow::trace::implementations::{ValBuilder, ValSpine};
use differential_dataflow::{AsCollection, VecCollection};
use rerail_facts::ir::{AggCall, AggFn, RepoSource, WindowKind};
use rerail_facts::sql::value::{self as sv, V};
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::rc::Rc;
use timely::dataflow::operators::probe::Handle;
use timely::dataflow::operators::core::Filter as TFilter;
use timely::dataflow::operators::ToStream;
use timely::dataflow::Scope;
use timely::order::Product;
use timely::PartialOrder;

pub type T = u32;
pub type R = i32;
pub type IT = Product<T, u64>;
pub type Sym = u32;
pub type Cid = u32;

type C<'s, Tm> = VecCollection<'s, Tm, (W, Row), R>;
type Arr<'s, Tm, V2> = Arranged<'s, TraceAgent<ValSpine<Row, (W, V2), Tm, R>>>;

/// Exported rows of one (path, content): `rows(export, path, content)`.
pub type Source = Rc<dyn Fn(u32, Sym, Cid) -> Rc<[Row]>>;

/// Runtime refusals (exact-or-refuse at run time): an order a construct observes was not total,
/// a bare value was ambiguous, or a scalar raised. Counted per construct (every occurrence, for
/// the report); the refusals raised inside an operator's evaluation are also records of the
/// dataflow (see [`REFUSED`]), so a refusal is live per (plan, worktree) exactly while its cause
/// is, and the affected findings are withheld, never guessed.
#[derive(Default, Debug, Clone)]
pub struct Refusals(pub BTreeMap<String, u64>);

/// Tag bit of a refusal record `(t | REFUSED, [construct])`: a signed difference of view t's
/// refusals from its parent's, like any record. Split from the data right where it is raised.
pub const REFUSED: W = 1 << 31;

thread_local! {
    static REFUSALS: RefCell<Refusals> = RefCell::new(Refusals::default());
    /// While an operator evaluates one view: the refusals it raises.
    static CAPTURE: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

pub fn refusals() -> Refusals {
    REFUSALS.with(|r| r.borrow().clone())
}

/// Runs `f`, then restores the runtime-refusal counters: a check's from-scratch dataflow raises
/// the measured run's refusals again, and those are not the run's.
pub fn uncounted<X>(f: impl FnOnce() -> X) -> X {
    let saved = refusals();
    let x = f();
    REFUSALS.with(|r| *r.borrow_mut() = saved);
    x
}

pub(super) fn refuse(what: &str) {
    REFUSALS.with(|r| *r.borrow_mut().0.entry(what.to_owned()).or_default() += 1);
    CAPTURE.with(|c| {
        if let Some(v) = c.borrow_mut().as_mut() {
            v.push(what.to_owned());
        }
    });
}

/// Runs `f`, returning its result and the distinct refusals it raised.
pub(super) fn captured<X>(f: impl FnOnce() -> X) -> (X, Vec<String>) {
    let outer = CAPTURE.with(|c| c.borrow_mut().replace(Vec::new()));
    let x = f();
    let mut got = CAPTURE.with(|c| std::mem::replace(&mut *c.borrow_mut(), outer)).unwrap_or_default();
    got.sort();
    got.dedup();
    (x, got)
}

pub(super) fn refusal_row(at: &str, what: &str) -> Row {
    Row::from([Val::text(val::sym(&format!("{at}: {what}")))])
}

/// A refusal raised outside the operators (a per-file producer's failure for a content).
pub fn refuse_external(what: &str) {
    refuse(what)
}

pub type Out = Rc<RefCell<FxHashMap<(W, Row), R>>>;

// Per-tag state (measurement, `--track`): live records of every operator output, per (operator,
// tag, record hash), and the in-memory size of each live record.
thread_local! {
    static TRACK: RefCell<FxHashMap<(usize, W, u64), (R, u32)>> = RefCell::new(FxHashMap::default());
}

/// (records, bytes) per tag over every tracked operator output.
pub fn tracked() -> FxHashMap<W, (u64, u64)> {
    TRACK.with(|t| {
        let mut per: FxHashMap<W, (u64, u64)> = FxHashMap::default();
        for ((_, w, _), (_, b)) in t.borrow().iter() {
            let e = per.entry(*w).or_default();
            e.0 += 1;
            e.1 += *b as u64;
        }
        per
    })
}

fn track<'s>(c: C<'s, T>, node: usize) -> C<'s, T> {
    c.inspect(move |((w, row), _t, d)| {
        use std::hash::{Hash, Hasher};
        let mut h = rustc_hash::FxHasher::default();
        row.hash(&mut h);
        let k = (node, *w, h.finish());
        let size = (std::mem::size_of::<((W, Row), T, R)>() + row.heap_bytes()) as u32;
        TRACK.with(|t| {
            let mut t = t.borrow_mut();
            let e = t.entry(k).or_insert((0, size));
            e.0 += d;
            if e.0 == 0 {
                t.remove(&k);
            }
        })
    })
}

// Arrangements of the measured dataflow, walkable (`register_traces`): the interner's epoch sweep
// marks every symbol their rows hold. Each walker owns a handle on its trace with both
// compaction frontiers released, so it holds nothing back; it reads the trace's batches as they
// are (`map_batches`), keys and values.
thread_local! {
    static REGISTER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    #[allow(clippy::type_complexity)]
    static WALKERS: RefCell<Vec<Box<dyn Fn(&mut dyn FnMut(&[Val]))>>> = RefCell::new(Vec::new());
}

/// While on, every arrangement built is registered for `walk_rows` (the main dataflow only: a
/// check's from-scratch dataflow is dropped, and its rows with it).
pub fn register_traces(on: bool) {
    REGISTER.with(|r| r.set(on));
}

/// Calls `f` on every row (key or value) every registered arrangement holds.
pub fn walk_rows(f: &mut dyn FnMut(&[Val])) {
    // Mutant 56: the arrangements' rows are not marked (their symbols may be reclaimed).
    if crate::mutant::on(56) {
        return;
    }
    WALKERS.with(|w| w.borrow().iter().for_each(|x| x(f)))
}

trait RowsOf {
    fn rows(&self, f: &mut dyn FnMut(&[Val]));
}
impl RowsOf for () {
    fn rows(&self, _f: &mut dyn FnMut(&[Val])) {}
}
impl RowsOf for Row {
    fn rows(&self, f: &mut dyn FnMut(&[Val])) {
        f(self)
    }
}

fn register<Tm: timely::progress::Timestamp + differential_dataflow::lattice::Lattice, V2: RowsOf + Ord + Clone + 'static>(trace: &TraceAgent<ValSpine<Row, (W, V2), Tm, R>>) {
    use differential_dataflow::trace::cursor::Navigable;
    use differential_dataflow::trace::{Cursor, TraceReader};
    if !REGISTER.with(|r| r.get()) {
        return;
    }
    let mut t = trace.clone();
    t.set_logical_compaction(timely::progress::frontier::AntichainRef::new(&[]));
    t.set_physical_compaction(timely::progress::frontier::AntichainRef::new(&[]));
    WALKERS.with(|w| {
        w.borrow_mut().push(Box::new(move |f: &mut dyn FnMut(&[Val])| {
            t.map_batches(|b| {
                let mut c = b.cursor();
                while let Some(k) = c.get_key(b) {
                    f(k);
                    while let Some((_, v)) = c.get_val(b) {
                        v.rows(f);
                        c.step_val(b);
                    }
                    c.step_key(b);
                }
            })
        }))
    });
}

// --------------------------------------------------------------------------------------------
// Per-record evaluation

pub(super) fn ev(e: &X, row: &[V]) -> Option<V> {
    match super::scalar::eval(e, row, &[]) {
        Ok(v) => Some(v),
        Err(m) => {
            refuse(&format!("scalar error: {m}"));
            None
        }
    }
}

/// A map's output row into `out` (false: a scalar refused). Bare columns are copied; the row is
/// converted to `V` only when an expression needs it.
pub(super) fn map_row(exprs: &[X], row: &[Val], out: &mut Vec<Val>) -> bool {
    out.clear();
    let mut vr: Option<Vec<V>> = None;
    for e in exprs {
        if let rerail_facts::ir::Expr::Col { depth: 0, col } = e {
            out.push(row[*col as usize]);
            continue;
        }
        match ev(e, vr.get_or_insert_with(|| val::row_to_v(row))) {
            Some(v) => out.push(val::from_v(&v)),
            None => return false,
        }
    }
    true
}

fn canon(v: Val) -> Val {
    match v.kind() {
        Kind::Real(b) => {
            let f = f64::from_bits(b);
            if f.fract() == 0.0 && f.abs() < 9.2e18 { Val::int(f as i64) } else { v }
        }
        _ => v,
    }
}

/// A row's key under `key`; `None` when an equality key is NULL (it matches nothing).
pub(super) fn key_of(row: &[Val], key: &Key, vrow: &mut Option<Vec<V>>) -> Option<Row> {
    key.iter()
        .map(|(x, mode)| {
            let fast = match x {
                rerail_facts::ir::Expr::Col { depth: 0, col } => Some(row[*col as usize]),
                _ => None,
            };
            Some(match (fast, mode) {
                (Some(v), KeyMode::Group) => canon(v),
                (Some(v), KeyMode::Eq(_)) if v.is_null() => return None,
                (Some(v), KeyMode::Eq(rerail_facts::ir::Aff::None)) => canon(v),
                _ => {
                    let r = vrow.get_or_insert_with(|| val::row_to_v(row));
                    let v = ev(x, r)?;
                    match mode {
                        KeyMode::Group => val::group_key(&v),
                        KeyMode::Eq(aff) => val::eq_key(&sv::compare_affinity(&v, *aff))?,
                    }
                }
            })
        })
        .collect()
}

fn keyed<'s, Tm: timely::progress::Timestamp + differential_dataflow::lattice::Lattice>(c: C<'s, Tm>, key: Key, name: &str) -> Arr<'s, Tm, Row> {
    let a = c
        .flat_map(move |(w, row)| {
            let mut vr = None;
            key_of(&row, &key, &mut vr).map(|k| (k, (w, row)))
        })
        .arrange_by_key_named(name);
    register(&a.trace);
    a
}

fn flip(before: R, after: R) -> R {
    debug_assert!(before >= 0 && after >= 0, "negative multiplicity: {before} -> {after}");
    (after > 0) as R - (before > 0) as R
}

/// Per key: the count of each tag's records (vals sorted by tag).
fn per_tag<V2>(vals: &[(&(W, V2), R)]) -> Vec<(W, R)> {
    let mut per: Vec<(W, R)> = Vec::new();
    for ((w, _), d) in vals {
        let d = if crate::mutant::on(5) { (*d).max(0) } else { *d };
        match per.last_mut() {
            Some((lw, ld)) if lw == w => *ld += d,
            _ => per.push((*w, d)),
        }
    }
    per
}

/// The presence records of one key from its per-tag counts (sorted by tag, zeros allowed): the
/// base emits [c_0 > 0]; tag t emits [below + c_t > 0] − [below > 0], `below` the count of t's
/// parent view. Pushed into `out` as (tag, ±1).
fn presence_of(per: &[(W, R)], out: &mut Vec<(W, R)>) {
    let at = |t: W| per.iter().find(|(w, _)| *w == t).map(|(_, c)| *c).unwrap_or(0);
    if at(BASE) > 0 {
        out.push((BASE, 1));
    }
    for &(t, d) in per {
        if t == BASE {
            continue;
        }
        let ch = tags::chain(t);
        let below: R = ch[..ch.len() - 1].iter().map(|x| at(*x)).sum();
        let f = flip(below, below + d);
        if f != 0 {
            out.push((t, f));
        }
    }
}

/// The per-key per-tag counts of a presence: `key -> (tag, ())`, one arrangement.
fn counts<'s, Tm: timely::progress::Timestamp + differential_dataflow::lattice::Lattice>(keys: VecCollection<'s, Tm, (Row, (W, ())), R>, name: &str) -> Arr<'s, Tm, ()> {
    let a = keys.arrange_by_key_named(&format!("{name}:presence-in"));
    register(&a.trace);
    a
}

/// The presence output arrangement over a counts arrangement (`reduce_abelian`): for readers that
/// need presence arranged at partially ordered times (a fixpoint's body).
fn presence_out<'s, Tm: timely::progress::Timestamp + differential_dataflow::lattice::Lattice>(cnt: Arr<'s, Tm, ()>, name: &str) -> Arr<'s, Tm, ()> {
    let a = cnt.reduce_abelian::<_, ValBuilder<Row, (W, ()), Tm, R>, ValSpine<Row, (W, ()), Tm, R>, _, _>(
        &format!("{name}:presence-out"),
        |_k, vals: &[(&(W, ()), R)], out: &mut Vec<((W, ()), R)>| {
            let mut o = Vec::new();
            presence_of(&per_tag(vals), &mut o);
            out.extend(o.into_iter().map(|(w, f)| ((w, ()), f)));
        },
        |vec, key, upds| {
            vec.clear();
            vec.extend(upds.drain(..).map(|(v, t, r)| ((key.clone(), v), t, r)));
        },
    );
    register(&a.trace);
    a
}

/// Presence of keys (count-based): `key -> (tag, ())` with ±1 deltas; counts and output arranged.
fn presence<'s, Tm: timely::progress::Timestamp + differential_dataflow::lattice::Lattice>(keys: VecCollection<'s, Tm, (Row, (W, ())), R>, name: &str) -> Arr<'s, Tm, ()> {
    presence_out(counts(keys, name), name)
}

/// Per-tag counts of one key in a cursor: (tag, Σ diffs) sorted by tag; mutant 5 drops
/// retractions, as `per_tag` does.
fn cursor_counts<Cu>(cur: &mut Cu, st: &Cu::Storage, key: &Row, out: &mut Vec<(W, R)>)
where
    Cu: for<'a> differential_dataflow::trace::Cursor<Key<'a> = &'a Row, Val<'a> = &'a (W, ()), DiffGat<'a> = &'a R>,
{
    cur.seek_key(st, key);
    if cur.get_key(st) != Some(key) {
        return;
    }
    while let Some((w, ())) = cur.get_val(st) {
        let mut s: R = 0;
        cur.map_times(st, |_, d| s += if crate::mutant::on(5) { (*d).max(0) } else { *d });
        match out.last_mut() {
            Some((lw, c)) if lw == w => *c += s,
            _ => out.push((*w, s)),
        }
        cur.step_val(st);
    }
}

/// The presence differences of one key over the times of `upds` ((time, tag, diff), any order),
/// starting from per-tag counts `per`: at each time, presence(after) − presence(before).
fn presence_steps(per: &mut Vec<(W, R)>, upds: &mut [(T, W, R)], out: &mut Vec<(T, W, R)>) {
    upds.sort_unstable();
    let (mut before, mut after) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < upds.len() {
        let t = upds[i].0;
        before.clear();
        presence_of(per, &mut before);
        while i < upds.len() && upds[i].0 == t {
            let (_, w, d) = upds[i];
            let d = if crate::mutant::on(5) { d.max(0) } else { d };
            match per.binary_search_by_key(&w, |x| x.0) {
                Ok(j) => per[j].1 += d,
                Err(j) => per.insert(j, (w, d)),
            }
            i += 1;
        }
        after.clear();
        presence_of(per, &mut after);
        for &(w, f) in &after {
            out.push((t, w, f));
        }
        for &(w, f) in &before {
            out.push((t, w, -f));
        }
    }
    differential_dataflow::consolidation::consolidate_updates(out);
}

/// Presence as a collection from its counts arrangement alone (delta mode, totally ordered
/// times): per key, the counts before a batch are read from the trace and the output differences
/// are presence(after) − presence(before) at each time, `threshold_total`'s scheme; no output
/// arrangement is kept.
fn presence_total<'s>(cnt: Arr<'s, T, ()>, name: &str) -> C<'s, T> {
    use differential_dataflow::trace::{BatchReader, Cursor, TraceReader};
    use timely::dataflow::channels::pact::Pipeline;
    use timely::dataflow::operators::generic::Operator;
    use timely::progress::Antichain;
    let mut trace = cnt.trace.clone();
    cnt.stream
        .unary_frontier::<timely::container::CapacityContainerBuilder<Vec<((W, Row), T, R)>>, _, _, _>(Pipeline, &format!("{name}:presence-total"), move |_, _| {
            let mut lower = Antichain::from_elem(0 as T);
            let mut upper = Antichain::from_elem(0 as T);
            move |(input, _frontier), output| {
                let mut batches = Vec::new();
                lower.clone_from(&upper);
                let mut cap = None;
                input.for_each(|c, bs| {
                    if cap.is_none() {
                        cap = Some(c.retain(0));
                    }
                    for b in bs.drain(..) {
                        upper.clone_from(b.upper());
                        batches.push(b);
                    }
                });
                if let Some(cap) = cap {
                    let mut session = output.session(&cap);
                    let (mut bc, bs) = differential_dataflow::trace::cursor::cursor_list(batches);
                    let (mut tc, ts) = trace.cursor_through(lower.borrow()).expect("presence: trace through the lower frontier");
                    let (mut per, mut upds, mut out) = (Vec::new(), Vec::new(), Vec::new());
                    while let Some(key) = bc.get_key(&bs) {
                        per.clear();
                        cursor_counts(&mut tc, &ts, key, &mut per);
                        upds.clear();
                        while let Some((w, ())) = bc.get_val(&bs) {
                            bc.map_times(&bs, |t, d| upds.push((*t, *w, *d)));
                            bc.step_val(&bs);
                        }
                        out.clear();
                        presence_steps(&mut per, &mut upds, &mut out);
                        for &(t, w, f) in &out {
                            session.give(((w, key.clone()), t, f));
                        }
                        bc.step_key(&bs);
                    }
                }
                trace.set_logical_compaction(upper.borrow());
                trace.set_physical_compaction(upper.borrow());
            }
        })
        .as_collection()
}

/// Semijoin of `left` (key -> (tag, row)) by the presence of the keys counted in `cnt` (delta
/// mode, totally ordered times), without an arrangement of the presence: the bilinear expansion
/// of `join_core` (each pair of left and count batches is joined once, by the later of the two,
/// at the later time), where the presence's differences at the count side's times are derived
/// from the counts before and after each time (as `presence_total`). Output: (product tag, row).
fn semi_total<'s>(left: Arr<'s, T, Row>, cnt: Arr<'s, T, ()>, name: &str) -> C<'s, T> {
    use differential_dataflow::trace::{BatchReader, Cursor, TraceReader};
    use timely::dataflow::channels::pact::Pipeline;
    use timely::dataflow::operators::generic::Operator;
    use timely::progress::Antichain;
    let (t1, t2) = (left.trace.clone(), cnt.trace.clone());
    left.stream
        .binary_frontier::<_, timely::container::CapacityContainerBuilder<Vec<((W, Row), T, R)>>, _, _, _, _>(cnt.stream, Pipeline, Pipeline, &format!("{name}:semi-total"), move |_, _| {
            // Bounds of the batches accepted from each input (joined against the other's trace).
            let mut ack1 = Antichain::from_elem(0 as T);
            let mut ack2 = Antichain::from_elem(0 as T);
            let (mut t1o, mut t2o) = (Some(t1), Some(t2));
            move |(in1, f1), (in2, f2), output| {
                let (mut per, mut upds, mut steps) = (Vec::new(), Vec::new(), Vec::new());
                let mut lefts: Vec<(T, R)> = Vec::new();
                // A left batch against the presence history of the accepted counts.
                in1.for_each(|cap, data| {
                    let cap = cap.retain(0);
                    let t2 = t2o.as_mut().expect("semijoin: count trace dropped before the left input ended");
                    for b1 in data.drain(..) {
                        let fresh = PartialOrder::less_equal(&ack1, b1.lower());
                        assert!(fresh || PartialOrder::less_equal(b1.upper(), &ack1), "semijoin: left batch spans the accepted frontier");
                        if fresh && !b1.is_empty() {
                            let mut session = output.session(&cap);
                            let (mut tc, ts) = differential_dataflow::trace::cursor::cursor_list(t2.batches_through(ack2.borrow()).expect("semijoin: counts through the accepted frontier"));
                            let (mut bc, bs) = differential_dataflow::trace::cursor::cursor_list(vec![b1.clone()]);
                            while let Some(key) = bc.get_key(&bs) {
                                // The presence's history at the key: its differences at the times
                                // of the accepted counts (times before the compaction frontier are
                                // accumulated into it, which is at or below every left time).
                                upds.clear();
                                tc.seek_key(&ts, key);
                                if tc.get_key(&ts) == Some(key) {
                                    while let Some((w, ())) = tc.get_val(&ts) {
                                        tc.map_times(&ts, |t, d| upds.push((*t, *w, *d)));
                                        tc.step_val(&ts);
                                    }
                                }
                                per.clear();
                                steps.clear();
                                presence_steps(&mut per, &mut upds, &mut steps);
                                if !steps.is_empty() {
                                    while let Some((w1, a)) = bc.get_val(&bs) {
                                        bc.map_times(&bs, |t1, d1| {
                                            for &(t2, w2, f) in &steps {
                                                if let Some(w) = tags::product(*w1, w2) {
                                                    session.give(((w, a.clone()), (*t1).max(t2), d1 * f));
                                                }
                                            }
                                        });
                                        bc.step_val(&bs);
                                    }
                                }
                                bc.step_key(&bs);
                            }
                        }
                        if fresh {
                            ack1.clone_from(b1.upper());
                        }
                    }
                });
                // A count batch: the presence's differences at its times, against the accepted left.
                in2.for_each(|cap, data| {
                    let cap = cap.retain(0);
                    let t1 = t1o.as_mut().expect("semijoin: left trace dropped before the count input ended");
                    let t2 = t2o.as_mut().expect("semijoin: count trace dropped");
                    for b2 in data.drain(..) {
                        let fresh = PartialOrder::less_equal(&ack2, b2.lower());
                        assert!(fresh || PartialOrder::less_equal(b2.upper(), &ack2), "semijoin: count batch spans the accepted frontier");
                        if fresh && !b2.is_empty() {
                            let mut session = output.session(&cap);
                            let (mut lc, ls) = differential_dataflow::trace::cursor::cursor_list(t1.batches_through(ack1.borrow()).expect("semijoin: left through the accepted frontier"));
                            let (mut pc, ps) = differential_dataflow::trace::cursor::cursor_list(t2.batches_through(b2.lower().borrow()).expect("semijoin: counts before the batch"));
                            let (mut bc, bs) = differential_dataflow::trace::cursor::cursor_list(vec![b2.clone()]);
                            while let Some(key) = bc.get_key(&bs) {
                                per.clear();
                                cursor_counts(&mut pc, &ps, key, &mut per);
                                upds.clear();
                                while let Some((w, ())) = bc.get_val(&bs) {
                                    bc.map_times(&bs, |t, d| upds.push((*t, *w, *d)));
                                    bc.step_val(&bs);
                                }
                                steps.clear();
                                presence_steps(&mut per, &mut upds, &mut steps);
                                if !steps.is_empty() {
                                    lc.seek_key(&ls, key);
                                    if lc.get_key(&ls) == Some(key) {
                                        while let Some((w1, a)) = lc.get_val(&ls) {
                                            lefts.clear();
                                            lc.map_times(&ls, |t1, d1| lefts.push((*t1, *d1)));
                                            for &(t1, d1) in &lefts {
                                                for &(t2, w2, f) in &steps {
                                                    if let Some(w) = tags::product(*w1, w2) {
                                                        session.give(((w, a.clone()), t1.max(t2), d1 * f));
                                                    }
                                                }
                                            }
                                            lc.step_val(&ls);
                                        }
                                    }
                                }
                                bc.step_key(&bs);
                            }
                        }
                        if fresh {
                            ack2.clone_from(b2.upper());
                        }
                    }
                });
                // Trace maintenance (as `join_core`): the left trace compacts logically to the
                // count input's frontier; the counts to the left input's frontier, and to their
                // own (a count batch reads the counts before it); physically, each to the batches
                // accepted from its own input.
                if let Some(t1) = t1o.as_mut() {
                    if f2.is_empty() {
                        t1o = None;
                    } else {
                        t1.set_logical_compaction(f2.frontier());
                        t1.set_physical_compaction(ack1.borrow());
                    }
                }
                if let Some(t2) = t2o.as_mut() {
                    if f1.is_empty() && f2.is_empty() {
                        t2o = None;
                    } else {
                        let mut lo = Antichain::new();
                        for t in f1.frontier().iter().chain(f2.frontier().iter()) {
                            lo.insert(*t);
                        }
                        t2.set_logical_compaction(lo.borrow());
                        t2.set_physical_compaction(ack2.borrow());
                    }
                }
            }
        })
        .as_collection()
}

/// Group recompute: per key, logic over each tag's view, emitted as differences. Returns the
/// output and the refusal records (`(t, [construct])`, differences like the output's): a group
/// whose logic refuses in view t yields no rows there and one live refusal.
fn group<'s>(input: Arr<'s, T, Row>, logic: Rc<dyn Fn(&Row, &[(&Row, R)]) -> Vec<(Row, R)>>, sentinel: bool, name: &str) -> (C<'s, T>, C<'s, T>) {
    let at = name.to_owned();
    let all = input
        .reduce_abelian::<_, ValBuilder<Row, (W, Row), T, R>, ValSpine<Row, (W, Row), T, R>, _, _>(
            &format!("{name}:group-out"),
            move |k, vals: &[(&(W, Row), R)], out: &mut Vec<((W, Row), R)>| {
                let mut tags_at: Vec<W> = Vec::new();
                for ((w, _), _) in vals {
                    if tags_at.last() != Some(w) {
                        tags_at.push(*w);
                    }
                }
                let view = |t: W| -> Vec<(&Row, R)> {
                    let ch = tags::chain(t);
                    let mut m: BTreeMap<&Row, R> = BTreeMap::new();
                    for ((w, r), d) in vals {
                        if ch.contains(w) {
                            let d = if crate::mutant::on(5) { (*d).max(0) } else { *d };
                            *m.entry(r).or_default() += d;
                        }
                    }
                    m.into_iter().filter(|(r, c)| *c != 0 && !(sentinel && r.is_empty())).collect()
                };
                // A view's output rows followed by its refusal records (tag bit REFUSED).
                let eval = |members: Vec<(&Row, R)>| -> Vec<(Row, R, bool)> {
                    if members.is_empty() && !sentinel {
                        return Vec::new();
                    }
                    let (rows, refused) = captured(|| {
                        if members.iter().any(|(_, c)| *c < 0) {
                            refuse("negative group member (inconsistent delta)");
                            return Vec::new();
                        }
                        logic(k, &members)
                    });
                    if !refused.is_empty() {
                        return refused.iter().map(|w| (refusal_row(&at, w), 1, true)).collect();
                    }
                    rows.into_iter().map(|(r, c)| (r, c, false)).collect()
                };
                let tagged = |t: W, flag: bool| if flag { t | REFUSED } else { t };
                let base = eval(view(BASE));
                for (r, c, f) in &base {
                    out.push(((tagged(BASE, *f), r.clone()), *c));
                }
                for &t in &tags_at {
                    if t == BASE {
                        continue;
                    }
                    for (r, c, f) in eval(view(t)) {
                        out.push(((tagged(t, f), r), c));
                    }
                    let p = tags::parent(t);
                    let below = if p == BASE { base.clone() } else { eval(view(p)) };
                    for (r, c, f) in below {
                        out.push(((tagged(t, f), r), -c));
                    }
                }
                differential_dataflow::consolidation::consolidate(out);
            },
            |vec, key, upds| {
                vec.clear();
                vec.extend(upds.drain(..).map(|(v, t, r)| ((key.clone(), v), t, r)));
            },
        );
    register(&all.trace);
    split_refusals(all.as_collection(|_k, (w, r)| (*w, r.clone())))
}

/// Splits records tagged `t | REFUSED` (refusals of view t) from the data.
fn split_refusals<'s>(all: C<'s, T>) -> (C<'s, T>, C<'s, T>) {
    let data = all.clone().filter(|(w, _)| w & REFUSED == 0);
    let refused = all.filter(|(w, _)| w & REFUSED != 0).map(|(w, r)| (w & !REFUSED, r));
    (data, refused)
}

// --------------------------------------------------------------------------------------------
// Group logics

fn vals_of(members: &[(&Row, R)]) -> Vec<(Vec<V>, R)> {
    members.iter().map(|(r, c)| (val::row_to_v(r), *c)).collect()
}

/// GROUP BY logic per group. Ordered aggregates (`group_concat`, `json_group_array`) fold the
/// group's rows in `order` (stable over equal rows) through the reference evaluator's own fold;
/// a tie between distinct rows on equal order keys is a runtime refusal (the reference raises
/// `tie:` on the same groups), never broken by representation order.
pub(super) fn agg_logic(keys: Key, aggs: Vec<AggCall<RepoSource>>, width: usize, order: Vec<(X, bool)>) -> Rc<dyn Fn(&Row, &[(&Row, R)]) -> Vec<(Row, R)>> {
    let key_cols: Vec<usize> = keys.iter().filter_map(|(k, _)| if let rerail_facts::ir::Expr::Col { depth: 0, col } = k { Some(*col as usize) } else { None }).collect();
    let ordered = aggs.iter().any(|a| matches!(a.f, AggFn::GroupConcat | AggFn::JsonGroupArray));
    let desc: Vec<bool> = order.iter().map(|x| x.1).collect();
    Rc::new(move |_k, members| {
        let rows = vals_of(members);
        // The ordered aggregates' sequence: member indices sorted by the order keys.
        let mut seq: Vec<usize> = (0..rows.len()).collect();
        if ordered {
            let mut keyed: Vec<(Vec<V>, usize)> = Vec::with_capacity(rows.len());
            for (i, (r, _)) in rows.iter().enumerate() {
                let mut k = Vec::with_capacity(order.len());
                for (e, _) in &order {
                    match ev(e, r) {
                        Some(v) => k.push(v),
                        None => return Vec::new(),
                    }
                }
                keyed.push((k, i));
            }
            // Mutant 42: the group's rows are folded in representation order (the sort skipped).
            if !crate::mutant::on(42) {
                keyed.sort_by(|a, b| order_cmp(&a.0, &b.0, &desc));
            }
            for w in keyed.windows(2) {
                if order_cmp(&w[0].0, &w[1].0, &desc) == Ordering::Equal && members[w[0].1].0 != members[w[1].1].0 {
                    refuse("ordered aggregate (group_concat/json_group_array) over an order with ties between distinct rows (input-order dependent)");
                    return Vec::new();
                }
            }
            seq = keyed.into_iter().map(|(_, i)| i).collect();
        }
        let mut out: Vec<Val> = vec![Val::NULL; width];
        for &c in &key_cols {
            let first = members.first().map(|(r, _)| r[c]).unwrap_or(Val::NULL);
            if members.iter().any(|(r, _)| r[c] != first) {
                refuse("GROUP BY key with several representations in one group (1 vs 1.0)");
                return Vec::new();
            }
            out[c] = first;
        }
        for a in &aggs {
            if matches!(a.f, AggFn::GroupConcat | AggFn::JsonGroupArray) {
                let mut items: Vec<(Option<V>, Option<V>)> = Vec::new();
                for &i in &seq {
                    let (r, c) = &rows[i];
                    let mut one = |e: &Option<X>| -> Result<Option<V>, ()> {
                        match e {
                            None => Ok(None),
                            Some(e) => ev(e, r).map(Some).ok_or(()),
                        }
                    };
                    let (Ok(v), Ok(s)) = (one(&a.arg), one(&a.sep)) else { return Vec::new() };
                    for _ in 0..*c {
                        items.push((v.clone(), s.clone()));
                    }
                }
                match rerail_facts::reference::fold_aggregate(a, items) {
                    Ok(v) => out.push(val::from_v(&v)),
                    Err(m) => {
                        refuse(&m);
                        return Vec::new();
                    }
                }
                continue;
            }
            let mut vals: Vec<(V, R)> = Vec::new();
            for (r, c) in &rows {
                let v = match &a.arg {
                    None => V::Int(1),
                    Some(e) => match ev(e, r) {
                        Some(v) => v,
                        None => return Vec::new(),
                    },
                };
                vals.push((v, *c));
            }
            if a.distinct {
                let mut seen = rustc_hash::FxHashSet::default();
                vals.retain(|(v, _)| seen.insert(sv::dkey(v)));
                for x in vals.iter_mut() {
                    x.1 = 1;
                }
            }
            let nn = || vals.iter().filter(|(v, _)| !v.is_null());
            let v = match a.f {
                AggFn::CountStar => V::Int(vals.iter().map(|(_, c)| *c as i64).sum()),
                AggFn::Count => V::Int(nn().map(|(_, c)| *c as i64).sum()),
                AggFn::Min | AggFn::Max => {
                    let mut best: Option<&V> = None;
                    for (v, _) in nn() {
                        let o = best.map(|b| sv::cmp(v, b));
                        let better = match (o, a.f) {
                            (None, _) => true,
                            (Some(Ordering::Less), AggFn::Min) | (Some(Ordering::Greater), AggFn::Max) => true,
                            _ => false,
                        };
                        if better {
                            best = Some(v);
                        }
                    }
                    if let Some(b) = best {
                        if nn().any(|(v, _)| sv::cmp(v, b) == Ordering::Equal && val::from_v(v) != val::from_v(b)) {
                            refuse("min/max tie between values of different representation");
                            return Vec::new();
                        }
                    }
                    best.cloned().unwrap_or(V::Null)
                }
                AggFn::Sum | AggFn::Total => {
                    let reals: R = nn().filter(|(v, _)| !matches!(v, V::Int(_))).map(|(_, c)| *c).sum();
                    if reals > 1 {
                        refuse("sum/total over several non-integer values (input-order rounding)");
                        return Vec::new();
                    }
                    let mut si: i64 = 0;
                    let mut sf: f64 = 0.0;
                    let mut any = false;
                    for (v, c) in nn() {
                        any = true;
                        match sv::to_numeric(v) {
                            Some(V::Int(i)) => match i.checked_mul(*c as i64).and_then(|x| si.checked_add(x)) {
                                Some(s) => si = s,
                                None => {
                                    refuse("integer overflow in sum");
                                    return Vec::new();
                                }
                            },
                            Some(V::Real(f)) => sf += f * *c as f64,
                            _ => sf += 0.0,
                        }
                    }
                    if a.f == AggFn::Total {
                        V::Real(si as f64 + sf)
                    } else if !any {
                        V::Null
                    } else if reals > 0 {
                        V::Real(si as f64 + sf)
                    } else {
                        V::Int(si)
                    }
                }
                AggFn::GroupConcat | AggFn::JsonGroupArray => unreachable!("refused in norm"),
            };
            out.push(val::from_v(&v));
        }
        vec![(Row::from(out), 1)]
    })
}

fn order_cmp(a: &[V], b: &[V], desc: &[bool]) -> Ordering {
    for (i, d) in desc.iter().enumerate() {
        let o = sv::cmp(&a[i], &b[i]);
        if o != Ordering::Equal {
            return if *d { o.reverse() } else { o };
        }
    }
    Ordering::Equal
}

pub(super) fn window_logic(order: Vec<(X, bool)>, kind: WindowKind<RepoSource>) -> Rc<dyn Fn(&Row, &[(&Row, R)]) -> Vec<(Row, R)>> {
    Rc::new(move |_k, members| {
        let rows = vals_of(members);
        match &kind {
            WindowKind::RowNumber => {
                let desc: Vec<bool> = order.iter().map(|x| x.1).collect();
                let mut keyed: Vec<(Vec<V>, usize)> = Vec::with_capacity(rows.len());
                for (i, (r, _)) in rows.iter().enumerate() {
                    let mut k = Vec::with_capacity(order.len());
                    for (e, _) in &order {
                        match ev(e, r) {
                            Some(v) => k.push(v),
                            None => return Vec::new(),
                        }
                    }
                    keyed.push((k, i));
                }
                keyed.sort_by(|a, b| order_cmp(&a.0, &b.0, &desc));
                for w in keyed.windows(2) {
                    // Mutant 9: tie detection disabled (the tie is broken by representation order).
                    if order_cmp(&w[0].0, &w[1].0, &desc) == Ordering::Equal && members[w[0].1].0 != members[w[1].1].0 && !crate::mutant::on(9) {
                        refuse("row_number() over an order with ties between distinct rows (input-order dependent)");
                        return Vec::new();
                    }
                }
                let mut out = Vec::new();
                let mut n: i64 = 0;
                for (_, i) in keyed {
                    for _ in 0..members[i].1 {
                        n += 1;
                        out.push((Row::with(members[i].0, Val::int(n)), 1));
                    }
                }
                out
            }
            WindowKind::Min(e) | WindowKind::Max(e) => {
                let is_max = matches!(kind, WindowKind::Max(_));
                let mut best: Option<V> = None;
                for (r, _) in &rows {
                    let v = match ev(e, r) {
                        Some(v) => v,
                        None => return Vec::new(),
                    };
                    if v.is_null() {
                        continue;
                    }
                    let better = match &best {
                        None => true,
                        Some(b) => {
                            let o = sv::cmp(&v, b);
                            (is_max && o == Ordering::Greater) || (!is_max && o == Ordering::Less)
                        }
                    };
                    if better {
                        best = Some(v);
                    }
                }
                let b = best.as_ref().map(val::from_v).unwrap_or(Val::NULL);
                members.iter().map(|(r, c)| (Row::with(r, b), *c)).collect()
            }
        }
    })
}

pub(super) fn distinct_prefix_logic(prefix: usize) -> Rc<dyn Fn(&Row, &[(&Row, R)]) -> Vec<(Row, R)>> {
    Rc::new(move |_k, members| {
        let first = members[0].0;
        if members.iter().any(|(r, _)| r[prefix..] != first[prefix..] || r[..prefix] != first[..prefix]) {
            refuse("DISTINCT on a prefix whose trailing (ORDER BY) columns differ (first-occurrence dependent)");
            return Vec::new();
        }
        vec![(first.clone(), 1)]
    })
}

// --------------------------------------------------------------------------------------------
// The dataflow

pub struct Built {
    pub files: InputSession<T, (W, (Sym, Cid)), R>,
    pub outputs: Vec<(String, Out)>,
    /// Per plan: its live runtime refusals, `(tag, [construct])` differences like the outputs'.
    pub refusals: Vec<(String, Out)>,
    /// The lowered graphs (one per distinct `Rc<Graph>`) and the plans each serves, in order.
    pub groups: Vec<(Rc<Graph>, Vec<String>)>,
    /// Per group: per node, the plans (bit i: the group's i-th plan) whose outputs read it.
    pub node_plans: Vec<Vec<u64>>,
    /// Timely operator identifiers built per group: `[lo, hi)` → group index.
    pub op_groups: Vec<(usize, usize, usize)>,
}

/// A stage's stored row: each column under its declared affinity (CTAS).
pub(super) fn affinity(row: &[Val], affs: &[rerail_facts::ir::Aff]) -> Row {
    row.iter()
        .enumerate()
        .map(|(i, v)| match affs.get(i) {
            Some(a) if *a != rerail_facts::ir::Aff::None => val::from_v(&sv::apply_affinity(val::to_v(*v), *a)),
            _ => *v,
        })
        .collect()
}

struct Lower<'s> {
    scope: Scope<'s, T>,
    g: Rc<Graph>,
    files: VecCollection<'s, T, (W, (Sym, Cid)), R>,
    src: Source,
    /// The plan's name (profiles).
    name: String,
    done: Vec<Option<C<'s, T>>>,
    /// `Some(graph offset)`: count every operator output's live records per tag.
    track: Option<usize>,
    /// Arrangements and presences by (node, key), shared by every operator that reads them.
    arrs: FxHashMap<(NodeId, String), Arr<'s, T, Row>>,
    press: FxHashMap<(NodeId, String), Arr<'s, T, ()>>,
    /// Presence output arrangements over `press`, for the fixpoint bodies that join with them.
    press_out: FxHashMap<(NodeId, String), Arr<'s, T, ()>>,
    /// Refusal records raised by the plan's operators (tag, [construct]), per node.
    refused: Vec<(NodeId, C<'s, T>)>,
    /// Refusals raised while lowering a node (inside a recursion), per node.
    built: Vec<(NodeId, Vec<String>)>,
}

/// The nodes of `g` that read the fixpoint variable `var`, directly or below: one pass over the
/// nodes in id order (an operator's inputs are always added before it, so ids are topological).
/// Linear in the graph; a recursive walk without memo visits every path of the shared DAG, which
/// is exponential in the depth of stage reuse.
///
/// Construction work (docs/facts/PHASE-3.md G3b.2): each call records `(graph nodes, node visits)`
/// in `CONSTRUCTION`, read into `greplay`'s results (`construction`). Mutant 51 re-seeds the
/// historical regression: every node's answer by the former unmemoised `reads_var` walk, its
/// calls counted (stopped at `WALK_CAP`; the answers are the linear pass's, which they equal).
pub(super) fn var_readers(g: &Graph, var: u32) -> Vec<bool> {
    let mut rd = vec![false; g.nodes.len()];
    for (n, op) in g.nodes.iter().enumerate() {
        let ins = op_inputs(op);
        assert!(ins.iter().all(|i| *i < n), "node {n}: inputs not topological");
        rd[n] = match op {
            Op::Var(v) => *v == var,
            _ => ins.iter().any(|i| rd[*i]),
        };
    }
    let mut visits = g.nodes.len() as u64;
    if crate::mutant::on(51) {
        fn reads_var(g: &Graph, n: NodeId, var: u32, calls: &mut u64) -> bool {
            *calls += 1;
            if *calls > WALK_CAP {
                return false;
            }
            match &g.nodes[n] {
                Op::Var(v) => *v == var,
                op => op_inputs(op).into_iter().any(|i| reads_var(g, i, var, calls)),
            }
        }
        visits = 0;
        for n in 0..g.nodes.len() {
            reads_var(g, n, var, &mut visits);
        }
        visits = visits.min(WALK_CAP);
    }
    CONSTRUCTION.lock().push((g.nodes.len() as u64, visits));
    rd
}

/// Mutant 51's walk stops after this many calls (the linker plan's took 1.06e17): about a second.
const WALK_CAP: u64 = 1 << 28;

/// `(graph nodes, node visits)` per `var_readers` call since the process started (every worker's).
static CONSTRUCTION: parking_lot::Mutex<Vec<(u64, u64)>> = parking_lot::Mutex::new(Vec::new());

/// The construction work recorded so far (`var_readers`).
pub(super) fn construction() -> Vec<(u64, u64)> {
    CONSTRUCTION.lock().clone()
}

/// An operator's input nodes.
pub(super) fn op_inputs(op: &Op) -> Vec<NodeId> {
    match op {
        Op::Var(_) | Op::Exported(_) | Op::Const(_) => Vec::new(),
        Op::Map { input, .. } | Op::Filter { input, .. } | Op::Distinct { input } | Op::Group { input, .. } | Op::Window { input, .. } | Op::DistinctPrefix { input, .. } | Op::Affinity { input, .. } => vec![*input],
        Op::Join { l, r, .. } | Op::Semi { l, r, .. } => vec![*l, *r],
        Op::Concat { inputs } => inputs.clone(),
        Op::Fix { init, step, .. } => init.iter().chain(step).copied().collect(),
    }
}

/// Calls the former unmemoised `reads_var(n)` made (profiles: the cost of the old walk), with
/// the readers known: 1 + its children's calls up to the first child that reads the variable.
fn walk_calls(g: &Graph, n: NodeId, rd: &[bool], memo: &mut FxHashMap<NodeId, f64>) -> f64 {
    if let Some(c) = memo.get(&n) {
        return *c;
    }
    let mut c = 1.0;
    if !matches!(g.nodes[n], Op::Var(_)) {
        for i in op_inputs(&g.nodes[n]) {
            c += walk_calls(g, i, rd, memo);
            if rd[i] {
                break;
            }
        }
    }
    memo.insert(n, c);
    c
}

/// Delta-mode nodes a fixpoint body reads (the boundary of the recursive part), each once.
fn boundary(g: &Graph, rd: &[bool], n: NodeId, seen: &mut Vec<bool>, out: &mut Vec<NodeId>) {
    if seen[n] {
        return;
    }
    seen[n] = true;
    if !rd[n] {
        out.push(n);
        return;
    }
    if !matches!(g.nodes[n], Op::Var(_)) {
        for i in op_inputs(&g.nodes[n]) {
            boundary(g, rd, i, seen, out);
        }
    }
}

/// The operator's kind (profiles).
fn op_kind(op: &Op) -> &'static str {
    match op {
        Op::Exported(_) => "exported",
        Op::Const(_) => "const",
        Op::Var(_) => "var",
        Op::Map { .. } => "map",
        Op::Filter { .. } => "filter",
        Op::Join { lk, .. } if lk.is_empty() => "join(cross)",
        Op::Join { .. } => "join",
        Op::Semi { anti: true, .. } => "antijoin",
        Op::Semi { .. } => "semijoin",
        Op::Distinct { .. } => "distinct",
        Op::Concat { .. } => "concat",
        Op::Group { .. } => "group",
        Op::Window { .. } => "window",
        Op::DistinctPrefix { .. } => "distinct-prefix",
        Op::Affinity { .. } => "affinity",
        Op::Fix { .. } => "fix",
    }
}

impl<'s> Lower<'s> {
    fn node(&mut self, n: NodeId) -> C<'s, T> {
        if let Some(c) = &self.done[n] {
            return c.clone();
        }
        // Refusals raised while lowering n itself (nested nodes capture their own): attributed to
        // n, so every plan whose outputs read n holds them.
        let (c, built) = captured(|| {
            if !super::prof::on() {
                return self.node_raw(n);
            }
            let lo = super::prof::begin(self.scope.worker());
            let c = self.node_raw(n);
            super::prof::end(self.scope.worker(), lo, &self.name, n, "delta", op_kind(&self.g.nodes[n]), &self.g.labels[n]);
            c
        });
        if !built.is_empty() {
            self.built.push((n, built));
        }
        c
    }

    fn node_raw(&mut self, n: NodeId) -> C<'s, T> {
        let g = self.g.clone();
        let c: C<'s, T> = match &g.nodes[n] {
            Op::Exported(id) => {
                let (id, src) = (*id, self.src.clone());
                self.files.clone().flat_map(move |(w, (p, c))| {
                    let rows = src(id, p, c);
                    (0..rows.len()).map(move |i| (w, rows[i].clone()))
                })
            }
            Op::Const(rows) => {
                let upd: Vec<((W, Row), T, R)> = rows.iter().map(|r| ((BASE, r.clone()), 0, 1)).collect();
                upd.to_stream(self.scope).as_collection()
            }
            Op::Var(_) => panic!("fixpoint variable outside its fixpoint"),
            Op::Map { input, exprs } => {
                let exprs = exprs.clone();
                let at = format!("n{n}");
                let mut out = Vec::with_capacity(exprs.len());
                let all = self.node(*input).flat_map(move |(w, row)| {
                    let (ok, refused) = captured(|| map_row(&exprs, &row, &mut out));
                    match refused.first() {
                        Some(r) => Some((w | REFUSED, refusal_row(&at, r))),
                        None => ok.then(|| (w, Row::new(&out))),
                    }
                });
                let (data, refused) = split_refusals(all);
                self.refused.push((n, refused));
                data
            }
            Op::Filter { input, pred } => {
                let pred = pred.clone();
                let at = format!("n{n}");
                let all = self.node(*input).flat_map(move |(w, row)| {
                    let (keep, refused) = captured(|| ev(&pred, &val::row_to_v(&row)).and_then(|v| sv::truth(&v)) == Some(true));
                    match refused.first() {
                        Some(r) => Some((w | REFUSED, refusal_row(&at, r))),
                        None => keep.then_some((w, row)),
                    }
                });
                let (data, refused) = split_refusals(all);
                self.refused.push((n, refused));
                data
            }
            Op::Affinity { input, affs } => {
                let affs = affs.clone();
                self.node(*input).map(move |(w, row)| (w, affinity(&row, &affs)))
            }
            Op::Join { l, r, lk, rk } => {
                let la = self.arr(*l, lk);
                let ra = self.arr(*r, rk);
                la.join_core(ra, |_k, (w1, a), (w2, b)| {
                    tags::product(*w1, *w2).map(|w| (w, Row::concat(a, b)))
                })
            }
            Op::Semi { l, r, lk, rk, anti } => {
                let left = self.node(*l);
                let la = self.arr(*l, lk);
                let cnt = self.pres(*r, rk);
                let hit = semi_total(la, cnt, &format!("n{n}"));
                if *anti { left.concat(hit.negate()) } else { hit }
            }
            Op::Distinct { input } => {
                let keys = self.node(*input).map(|(w, row)| (row, (w, ())));
                presence_total(counts(keys, &format!("n{n}:distinct")), &format!("n{n}:distinct"))
            }
            Op::Concat { inputs } => {
                let mut it = inputs.clone().into_iter();
                let first = self.node(it.next().unwrap());
                it.fold(first, |a, i| a.concat(self.node(i)))
            }
            Op::Group { input, keys, aggs, keyless, width, order } => {
                let mut c = self.node(*input);
                if *keyless {
                    // One group even over no rows: a sentinel member under the base.
                    let s: Vec<((W, Row), T, R)> = vec![((BASE, Row::default()), 0, 1)];
                    c = c.concat(s.to_stream(self.scope).as_collection());
                }
                let kk = keys.clone();
                let arr = c
                    .flat_map(move |(w, row)| {
                        if row.is_empty() {
                            return Some((Row::default(), (w, row)));
                        }
                        let mut vr = None;
                        key_of(&row, &kk, &mut vr).map(|k| (k, (w, row)))
                    })
                    .arrange_by_key_named(&format!("n{n}:group-in"));
                register(&arr.trace);
                let (data, refused) = group(arr, agg_logic(keys.clone(), aggs.clone(), *width, order.clone()), *keyless, &format!("n{n}"));
                self.refused.push((n, refused));
                data
            }
            Op::Window { input, part, order, kind } => {
                let arr = self.arr(*input, part);
                let (data, refused) = group(arr, window_logic(order.clone(), kind.clone()), false, &format!("n{n}"));
                self.refused.push((n, refused));
                data
            }
            Op::DistinctPrefix { input, prefix } => {
                let p = *prefix;
                let key: Key = (0..p).map(|c| (super::norm::col(c), KeyMode::Group)).collect();
                let arr = self.arr(*input, &key);
                let (data, refused) = group(arr, distinct_prefix_logic(p), false, &format!("n{n}"));
                self.refused.push((n, refused));
                data
            }
            Op::Fix { var, init, step } => self.fix(n, *var, init.clone(), step.clone()),
        };
        let c = match self.track {
            Some(off) => track(c, off + n),
            None => c,
        };
        self.done[n] = Some(c.clone());
        c
    }

    /// The arrangement of node `n`'s records by `key`, one per (node, key) and shared by every
    /// operator that needs it (joins, semijoins, windows, and the fixpoints' boundary joins).
    fn arr(&mut self, n: NodeId, key: &Key) -> Arr<'s, T, Row> {
        let k = (n, format!("{key:?}"));
        if let Some(a) = self.arrs.get(&k) {
            return a.clone();
        }
        let a = keyed(self.node(n), key.clone(), &format!("n{n}:by{}", self.arrs.len()));
        self.arrs.insert(k, a.clone());
        a
    }

    /// Per-tag counts of node `n`'s keys under `key` (the presence's one arrangement), one per
    /// (node, key).
    fn pres(&mut self, n: NodeId, key: &Key) -> Arr<'s, T, ()> {
        let k = (n, format!("{key:?}"));
        if let Some(a) = self.press.get(&k) {
            return a.clone();
        }
        let kk = key.clone();
        let keys = self.node(n).flat_map(move |(w, row)| {
            let mut vr = None;
            key_of(&row, &kk, &mut vr).map(|k| (k, (w, ())))
        });
        let a = counts(keys, &format!("n{n}:pres{}", self.press.len()));
        self.press.insert(k, a.clone());
        a
    }

    /// The presence output arrangement over `pres(n, key)`: only a fixpoint body joins with an
    /// arranged presence (its times are partially ordered, so `semi_total` does not apply).
    fn pres_out(&mut self, n: NodeId, key: &Key) -> Arr<'s, T, ()> {
        let k = (n, format!("{key:?}"));
        if let Some(a) = self.press_out.get(&k) {
            return a.clone();
        }
        let cnt = self.pres(n, key);
        let a = presence_out(cnt, &format!("n{n}:pres"));
        self.press_out.insert(k, a.clone());
        a
    }

    /// The delta-mode arrangements a fixpoint body joins with (view × delta), built here and
    /// entered into the iteration, so the body shares them with the operators outside it.
    fn boundary_arrs(&mut self, rd: &[bool], n: NodeId, arrs: &mut Vec<((NodeId, String), Arr<'s, T, Row>)>, press: &mut Vec<((NodeId, String), Arr<'s, T, ()>)>, seen: &mut Vec<bool>) {
        let g = self.g.clone();
        if !rd[n] || seen[n] {
            return;
        }
        seen[n] = true;
        match &g.nodes[n] {
            Op::Join { l, r, lk, rk } => {
                for (s, k) in [(*l, lk), (*r, rk)] {
                    if rd[s] {
                        self.boundary_arrs(rd, s, arrs, press, seen);
                    } else {
                        let a = self.arr(s, k);
                        arrs.push(((s, format!("{k:?}")), a));
                    }
                }
            }
            Op::Semi { l, r, lk, rk, .. } => {
                if rd[*l] {
                    self.boundary_arrs(rd, *l, arrs, press, seen);
                } else {
                    let a = self.arr(*l, lk);
                    arrs.push(((*l, format!("{lk:?}")), a));
                }
                if rd[*r] {
                    self.boundary_arrs(rd, *r, arrs, press, seen);
                } else {
                    let a = self.pres_out(*r, rk);
                    press.push(((*r, format!("{rk:?}")), a));
                }
            }
            Op::Map { input, .. } | Op::Filter { input, .. } | Op::Distinct { input } | Op::Group { input, .. } | Op::Window { input, .. } | Op::DistinctPrefix { input, .. } | Op::Affinity { input, .. } => self.boundary_arrs(rd, *input, arrs, press, seen),
            Op::Concat { inputs } => inputs.iter().for_each(|i| self.boundary_arrs(rd, *i, arrs, press, seen)),
            Op::Fix { init, step, .. } => init.iter().chain(step).for_each(|i| self.boundary_arrs(rd, *i, arrs, press, seen)),
            Op::Var(_) | Op::Exported(_) | Op::Const(_) => {}
        }
    }

    /// Set fixpoint in delta mode: differential `iterate` (semi-naive) over tagged records, the
    /// body lowered with the same base + delta expansions as outside (`IterLower`), the distinct
    /// of each round the count-based presence. With base records only (a base load) every product
    /// is base and presence is a threshold: a plain semi-naive evaluation. A worktree's delta
    /// iterates only its own signed differences against the entered base arrangements, so
    /// per-worktree state is proportional to the divergence of its fixpoint, not to its size.
    fn fix(&mut self, n: NodeId, var: u32, init: Vec<NodeId>, step: Vec<NodeId>) -> C<'s, T> {
        let g = self.g.clone();
        let rd = Rc::new(var_readers(&g, var));
        if super::prof::on() {
            // The former walk called reads_var at least once per view node and per input of it
            // (ViewLower::is_view), each call visiting every path below the node.
            let mut memo = FxHashMap::default();
            let calls: f64 = (0..g.nodes.len()).filter(|n| rd[*n]).map(|v| walk_calls(&g, v, &rd, &mut memo) + op_inputs(&g.nodes[v]).iter().map(|c| walk_calls(&g, *c, &rd, &mut memo)).sum::<f64>()).sum();
            let view_nodes = rd.iter().filter(|x| **x).count();
            super::prof::note(&format!("{}:{}#fix{var}", self.name, g.labels[step[0]]), serde_json::json!({"nodes": g.nodes.len(), "view_nodes": view_nodes, "old_reads_var_calls_lower_bound": calls}));
        }
        // Delta-mode inputs of the body.
        let mut bnd = Vec::new();
        let mut seen = vec![false; g.nodes.len()];
        for &s in &step {
            boundary(&g, &rd, s, &mut seen, &mut bnd);
        }
        let bnd_cols: Vec<(NodeId, C<'s, T>)> = bnd.iter().map(|&b| (b, self.node(b))).collect();
        let (mut outer_arrs, mut outer_press, mut seen) = (Vec::new(), Vec::new(), vec![false; g.nodes.len()]);
        for &s in &step {
            self.boundary_arrs(&rd, s, &mut outer_arrs, &mut outer_press, &mut seen);
        }
        let init_d: C<'s, T> = {
            let mut it = init.iter();
            let first = self.node(*it.next().unwrap());
            it.fold(first, |a, i| a.concat(self.node(*i)))
        };
        let seeds = init_d;
        // The boundary relations' arrangements are the delta-mode ones built above, entered.
        let g2 = g.clone();
        let gname = self.name.clone();
        let result = seeds.clone().iterate(|inner, rec| {
            let mut env: FxHashMap<NodeId, C<'_, IT>> = FxHashMap::default();
            for (b, c) in &bnd_cols {
                env.insert(*b, c.clone().enter(inner));
            }
            let earrs = outer_arrs.iter().map(|(k, a)| (k.clone(), a.clone().enter(inner))).collect();
            let epress = outer_press.iter().map(|(k, a)| (k.clone(), a.clone().enter(inner))).collect();
            let mut vl = IterLower { g: g2.clone(), gname: gname.clone(), rd: rd.clone(), rec: rec.clone(), env, earrs, epress, done: FxHashMap::default() };
            let mut out = seeds.clone().enter(inner);
            let fix_label = format!("{gname}:{}#fix{var}", g2.labels[step[0]]);
            for (ai, &s) in step.iter().enumerate() {
                let mut d = vl.node(s);
                if crate::mutant::on(6) {
                    d = TFilter::filter(d.inner, |(_, t, _): &((W, Row), IT, R)| t.inner < 2).as_collection();
                }
                if super::prof::on() {
                    let (fl, arm) = (fix_label.clone(), format!("arm{ai}:n{s}"));
                    d = d.inspect(move |(_, t, r)| super::prof::iter(&fl, &arm, t.inner, *r as i64));
                }
                out = out.concat(d);
            }
            let out = presence(out.map(|(w, row)| (row, (w, ()))), &format!("n{n}:iterate-distinct")).as_collection(|row, (w, ())| (*w, row.clone()));
            if super::prof::on() {
                let fl = fix_label.clone();
                out.clone().inspect(move |(_, t, r)| super::prof::iter(&fl, "distinct", t.inner, *r as i64));
            }
            out
        });
        result
    }
}

type EArr<'i, V2> = Arranged<'i, TraceEnter<TraceAgent<ValSpine<Row, (W, V2), T, R>>, IT>>;

/// Delta-mode lowering inside a fixpoint: records `(t, row)` are signed differences of view t
/// from its parent's, as outside; the operators are the same expansions at the iteration's time.
struct IterLower<'i> {
    g: Rc<Graph>,
    gname: String,
    /// Nodes reading this fixpoint's variable (`var_readers`).
    rd: Rc<Vec<bool>>,
    rec: C<'i, IT>,
    /// Delta-mode boundary collections, entered.
    env: FxHashMap<NodeId, C<'i, IT>>,
    /// Delta-mode boundary arrangements (by node and key) and presences, entered.
    earrs: FxHashMap<(NodeId, String), EArr<'i, Row>>,
    epress: FxHashMap<(NodeId, String), EArr<'i, ()>>,
    done: FxHashMap<NodeId, C<'i, IT>>,
}

impl<'i> IterLower<'i> {
    fn is_view(&self, n: NodeId) -> bool {
        self.rd[n]
    }

    fn node(&mut self, n: NodeId) -> C<'i, IT> {
        if let Some(c) = self.done.get(&n) {
            return c.clone();
        }
        if !self.is_view(n) {
            return self.env[&n].clone();
        }
        if !super::prof::on() {
            return self.node_raw(n);
        }
        let w = self.rec.scope().worker();
        let lo = super::prof::begin(w);
        let c = self.node_raw(n);
        super::prof::end(w, lo, &self.gname, n, "iter", op_kind(&self.g.nodes[n]), &self.g.labels[n]);
        c
    }

    fn node_raw(&mut self, n: NodeId) -> C<'i, IT> {
        let g = self.g.clone();
        let c = match &g.nodes[n] {
            Op::Var(_) => self.rec.clone(),
            Op::Map { input, exprs } => {
                let exprs = exprs.clone();
                let mut out = Vec::with_capacity(exprs.len());
                self.node(*input).flat_map(move |(w, row)| {
                    if !map_row(&exprs, &row, &mut out) {
                        return None;
                    }
                    Some((w, Row::new(&out)))
                })
            }
            Op::Filter { input, pred } => {
                let pred = pred.clone();
                self.node(*input).filter(move |(_, row)| ev(&pred, &val::row_to_v(row)).and_then(|v| sv::truth(&v)) == Some(true))
            }
            Op::Affinity { input, affs } => {
                let affs = affs.clone();
                self.node(*input).map(move |(w, row)| (w, affinity(&row, &affs)))
            }
            Op::Concat { inputs } => {
                let mut it = inputs.clone().into_iter();
                let first = self.node(it.next().unwrap());
                it.fold(first, |a, i| a.concat(self.node(i)))
            }
            Op::Distinct { input } => {
                let keys = self.node(*input).map(|(w, row)| (row, (w, ())));
                presence(keys, &format!("n{n}:iter-distinct")).as_collection(|k, (w, ())| (*w, k.clone()))
            }
            Op::Join { l, r, lk, rk } => {
                let (lv, rv) = (self.is_view(*l), self.is_view(*r));
                // The bilinear expansion, as outside: a pair (t1, t2) yields product(t1, t2).
                let pair = |_k: &Row, (w1, a): &(W, Row), (w2, b): &(W, Row)| tags::product(*w1, *w2).map(|w| (w, Row::concat(a, b)));
                match (lv, rv) {
                    (true, true) => {
                        let la = keyed(self.node(*l), lk.clone(), &format!("n{n}:iter-join-l"));
                        let ra = keyed(self.node(*r), rk.clone(), &format!("n{n}:iter-join-r"));
                        la.join_core(ra, pair)
                    }
                    (true, false) => {
                        let la = keyed(self.node(*l), lk.clone(), &format!("n{n}:iter-join-l"));
                        let ra = self.earrs[&(*r, format!("{rk:?}"))].clone();
                        la.join_core(ra, pair)
                    }
                    (false, true) => {
                        let la = self.earrs[&(*l, format!("{lk:?}"))].clone();
                        let ra = keyed(self.node(*r), rk.clone(), &format!("n{n}:iter-join-r"));
                        la.join_core(ra, pair)
                    }
                    (false, false) => unreachable!("join of two non-recursive relations inside the recursion"),
                }
            }
            Op::Semi { l, r, lk, rk, anti } => {
                let lv = self.is_view(*l);
                let rv = self.is_view(*r);
                if *anti && rv {
                    refuse(&format!("{}: antijoin by a recursive relation inside the recursion (not stratified)", g.labels[n]));
                    return self.rec.clone().filter(|_| false);
                }
                let left = self.node(*l);
                let hit_of = |_k: &Row, (w1, a): &(W, Row), (w2, ()): &(W, ())| tags::product(*w1, *w2).map(|w| (w, a.clone()));
                // The right side's count-based presence: built here over the recursive relation,
                // or the delta-mode one shared with the operators outside, entered.
                let hit = match (lv, rv) {
                    (true, true) => {
                        let rk = rk.clone();
                        let keys = self.node(*r).flat_map(move |(w, row)| {
                            let mut vr = None;
                            key_of(&row, &rk, &mut vr).map(|k| (k, (w, ())))
                        });
                        let pres = presence(keys, &format!("n{n}:iter-semi-r"));
                        keyed(left.clone(), lk.clone(), &format!("n{n}:iter-semi-l")).join_core(pres, hit_of)
                    }
                    (true, false) => {
                        let pres = self.epress[&(*r, format!("{rk:?}"))].clone();
                        keyed(left.clone(), lk.clone(), &format!("n{n}:iter-semi-l")).join_core(pres, hit_of)
                    }
                    (false, true) => {
                        let rk = rk.clone();
                        let keys = self.node(*r).flat_map(move |(w, row)| {
                            let mut vr = None;
                            key_of(&row, &rk, &mut vr).map(|k| (k, (w, ())))
                        });
                        let pres = presence(keys, &format!("n{n}:iter-semi-r"));
                        self.earrs[&(*l, format!("{lk:?}"))].clone().join_core(pres, hit_of)
                    }
                    (false, false) => unreachable!("semijoin of two non-recursive relations inside the recursion"),
                };
                if *anti { left.concat(hit.negate()) } else { hit }
            }
            Op::Group { .. } | Op::Window { .. } | Op::DistinctPrefix { .. } => {
                refuse(&format!("{}: aggregate, window or DISTINCT-prefix over a recursive relation (not stratified)", g.labels[n]));
                self.rec.clone().filter(|_| false)
            }
            Op::Fix { .. } => {
                refuse(&format!("{}: nested recursion", g.labels[n]));
                self.rec.clone().filter(|_| false)
            }
            Op::Exported(_) | Op::Const(_) => unreachable!("delta-mode node in view mode"),
        };
        self.done.insert(n, c.clone());
        c
    }
}

/// Maintains a collection's consolidated contents in an `Out` map.
fn capture<'s>(c: C<'s, T>, probe: &Handle<T>) -> Out {
    let out: Out = Rc::default();
    let o = out.clone();
    c.consolidate()
        .inspect(move |(k, _t, d)| {
            let mut o = o.borrow_mut();
            let e = o.entry(k.clone()).or_default();
            *e += d;
            if *e == 0 {
                o.remove(k);
            }
        })
        .probe_with(probe);
    out
}

/// The plans (bits) reading each node of `g`: plan i's outputs `outs[i]` and everything below.
fn reach(g: &Graph, outs: &[Vec<NodeId>]) -> Vec<u64> {
    assert!(outs.len() <= 64, "more than 64 plans share one graph");
    let mut bits = vec![0u64; g.nodes.len()];
    for (i, os) in outs.iter().enumerate() {
        let mut stack: Vec<NodeId> = os.clone();
        while let Some(n) = stack.pop() {
            if bits[n] & (1 << i) == 0 {
                bits[n] |= 1 << i;
                stack.extend(op_inputs(&g.nodes[n]));
            }
        }
    }
    bits
}

/// Build one dataflow evaluating every plan's outputs over the tagged `files` input. Plans whose
/// entries hold the same `Rc<Graph>` (normalised through one context, `norm::Plans`) are lowered
/// together: a node is one collection and an arrangement per key, whichever plans read it. Each
/// plan's outputs are those `Graph::owners` assigns it (all of them when a graph has no owners).
pub fn build(worker: &mut timely::worker::Worker, graphs: Vec<(String, Rc<Graph>)>, src: Source, probe: &Handle<T>, track_on: bool) -> Built {
    let mut groups: Vec<(Rc<Graph>, Vec<String>)> = Vec::new();
    for (name, g) in graphs {
        match groups.iter_mut().find(|(h, _)| Rc::ptr_eq(h, &g)) {
            Some((_, names)) => names.push(name),
            None => groups.push((g, vec![name])),
        }
    }
    worker.dataflow::<T, _, _>(|scope| {
        let (files_in, files) = scope.new_collection::<(W, (Sym, Cid)), R>();
        let mut outputs = Vec::new();
        let mut refusals = Vec::new();
        let (mut node_plans, mut op_groups) = (Vec::new(), Vec::new());
        for (gi, (g, names)) in groups.iter().enumerate() {
            let lo = scope.worker().peek_identifier();
            let n = g.nodes.len();
            let mut lw = Lower { scope, g: g.clone(), files: files.clone(), src: src.clone(), name: names.join("+"), done: vec![None; n], track: track_on.then_some(gi * 1_000_000), arrs: FxHashMap::default(), press: FxHashMap::default(), press_out: FxHashMap::default(), refused: Vec::new(), built: Vec::new() };
            let mut outs: Vec<Vec<NodeId>> = Vec::new();
            for name in names {
                let mine: Vec<&(String, NodeId, Vec<String>)> = g.outputs.iter().enumerate().filter(|(i, _)| g.owners.is_empty() || g.owners[*i] == *name).map(|(_, o)| o).collect();
                for (clause, node, _) in &mine {
                    let c = lw.node(*node);
                    outputs.push((format!("{name}:{clause}"), capture(c, probe)));
                }
                outs.push(mine.iter().map(|o| o.1).collect());
            }
            let bits = reach(g, &outs);
            for (i, name) in names.iter().enumerate() {
                let on = |n: &NodeId| bits[*n] & (1 << i) != 0;
                // A construct refused while lowering (inside a recursion: a non-stratified
                // antijoin, an aggregate over the recursive relation) leaves part of a plan
                // empty: the plan's findings are withheld everywhere by a base refusal record,
                // never reported partial.
                let at_build: std::collections::BTreeSet<&String> = lw.built.iter().filter(|(n, _)| on(n)).flat_map(|(_, w)| w).collect();
                let base: Vec<((W, Row), T, R)> = at_build.iter().map(|w| ((BASE, refusal_row(name, w)), 0, 1)).collect();
                let refused = lw.refused.iter().filter(|(n, _)| on(n)).fold(base.to_stream(scope).as_collection(), |a, (_, c)| a.concat(c.clone()));
                refusals.push((name.clone(), capture(refused, probe)));
            }
            drop(lw);
            node_plans.push(bits);
            op_groups.push((lo, scope.worker().peek_identifier(), gi));
        }
        Built { files: files_in, outputs, refusals, groups, node_plans, op_groups }
    })
}

/// A worktree's full output from the tagged output map: Σ over its chain, as a multiset.
pub fn full(out: &Out, w: W) -> Result<BTreeMap<Row, R>, String> {
    let ch = tags::chain(w);
    let mut m: BTreeMap<Row, R> = BTreeMap::new();
    for ((t, r), d) in out.borrow().iter() {
        if ch.contains(t) {
            *m.entry(r.clone()).or_default() += d;
        }
    }
    m.retain(|_, c| *c != 0);
    if let Some((r, c)) = m.iter().find(|(_, c)| **c < 0) {
        return Err(format!("negative multiplicity {c} for {r:?}"));
    }
    Ok(m)
}
