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

fn refuse(what: &str) {
    REFUSALS.with(|r| *r.borrow_mut().0.entry(what.to_owned()).or_default() += 1);
    CAPTURE.with(|c| {
        if let Some(v) = c.borrow_mut().as_mut() {
            v.push(what.to_owned());
        }
    });
}

/// Runs `f`, returning its result and the distinct refusals it raised.
fn captured<X>(f: impl FnOnce() -> X) -> (X, Vec<String>) {
    let outer = CAPTURE.with(|c| c.borrow_mut().replace(Vec::new()));
    let x = f();
    let mut got = CAPTURE.with(|c| std::mem::replace(&mut *c.borrow_mut(), outer)).unwrap_or_default();
    got.sort();
    got.dedup();
    (x, got)
}

fn refusal_row(at: &str, what: &str) -> Row {
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

// --------------------------------------------------------------------------------------------
// Per-record evaluation

fn ev(e: &X, row: &[V]) -> Option<V> {
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
fn map_row(exprs: &[X], row: &[Val], out: &mut Vec<Val>) -> bool {
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
fn key_of(row: &[Val], key: &Key, vrow: &mut Option<Vec<V>>) -> Option<Row> {
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

/// Columns of a node's output its arrangements keep (`None`: all of them). The others are read
/// by no consumer (`demand`); they come back as NULL when an arranged row is widened.
type Cols = Option<Rc<[usize]>>;

fn narrow(row: &Row, cols: &Cols) -> Row {
    match cols {
        None => row.clone(),
        Some(c) => c.iter().map(|i| row[*i]).collect(),
    }
}

/// Appends an arranged row widened back to its node's `width` (unkept columns NULL).
fn widen_into(out: &mut Vec<Val>, row: &[Val], cols: &Cols, width: usize) {
    match cols {
        None => out.extend_from_slice(row),
        Some(c) => {
            let s = out.len();
            out.resize(s + width, Val::NULL);
            for (j, i) in c.iter().enumerate() {
                out[s + i] = row[j];
            }
        }
    }
}

fn widen(row: &Row, cols: &Cols, width: usize) -> Row {
    match cols {
        None => row.clone(),
        Some(_) => {
            let mut v = Vec::with_capacity(width);
            widen_into(&mut v, row, cols, width);
            Row::new(&v)
        }
    }
}

/// A join's output row from two arranged (possibly narrowed) rows.
fn joined(a: &Row, ca: &Cols, wa: usize, b: &Row, cb: &Cols, wb: usize) -> Row {
    if ca.is_none() && cb.is_none() {
        return Row::concat(a, b);
    }
    let mut v = Vec::with_capacity(wa + wb);
    widen_into(&mut v, a, ca, wa);
    widen_into(&mut v, b, cb, wb);
    Row::new(&v)
}

/// Column demand: per node, the output columns some consumer reads (backwards over the
/// topological node order). Consumers with whole-row semantics (outputs, DISTINCT, fixpoints,
/// aggregates, windows, DISTINCT prefixes) read every column; a map reads what any of its
/// expressions reads (an unread expression may still refuse at run time); a join passes its
/// output demand to its sides plus its keys; a semijoin's right side is read at its keys only.
fn demand(g: &Graph) -> Vec<Cols> {
    let n = g.nodes.len();
    let mut need: Vec<Vec<bool>> = g.widths.iter().map(|w| vec![false; *w]).collect();
    let all = |need: &mut Vec<Vec<bool>>, i: NodeId| need[i].iter_mut().for_each(|x| *x = true);
    let reads = |need: &mut Vec<Vec<bool>>, i: NodeId, e: &X| {
        if super::norm::has_subquery(e) {
            need[i].iter_mut().for_each(|x| *x = true);
            return;
        }
        let mut c = Vec::new();
        super::norm::cols_read(e, &mut c);
        for c in c {
            need[i][c] = true;
        }
    };
    for (_, o, _) in &g.outputs {
        all(&mut need, *o);
    }
    for i in (0..n).rev() {
        let mine = need[i].clone();
        match &g.nodes[i] {
            Op::Exported(_) | Op::Const(_) | Op::Var(_) => {}
            Op::Map { input, exprs } => exprs.iter().for_each(|e| reads(&mut need, *input, e)),
            Op::Filter { input, pred } => {
                mine.iter().enumerate().filter(|(_, x)| **x).for_each(|(c, _)| need[*input][c] = true);
                reads(&mut need, *input, pred);
            }
            Op::Affinity { input, .. } => mine.iter().enumerate().filter(|(_, x)| **x).for_each(|(c, _)| need[*input][c] = true),
            Op::Join { l, r, lk, rk } => {
                let wl = g.widths[*l];
                for (c, x) in mine.iter().enumerate() {
                    if *x {
                        if c < wl {
                            need[*l][c] = true
                        } else {
                            need[*r][c - wl] = true
                        }
                    }
                }
                lk.iter().for_each(|(e, _)| reads(&mut need, *l, e));
                rk.iter().for_each(|(e, _)| reads(&mut need, *r, e));
            }
            Op::Semi { l, r, lk, rk, .. } => {
                mine.iter().enumerate().filter(|(_, x)| **x).for_each(|(c, _)| need[*l][c] = true);
                lk.iter().for_each(|(e, _)| reads(&mut need, *l, e));
                rk.iter().for_each(|(e, _)| reads(&mut need, *r, e));
            }
            Op::Concat { inputs } => {
                for j in inputs {
                    mine.iter().enumerate().filter(|(_, x)| **x).for_each(|(c, _)| need[*j][c] = true);
                }
            }
            Op::Distinct { input } | Op::Group { input, .. } | Op::Window { input, .. } | Op::DistinctPrefix { input, .. } => all(&mut need, *input),
            Op::Fix { init, step, .. } => init.iter().chain(step).for_each(|j| all(&mut need, *j)),
        }
    }
    need.into_iter()
        .map(|v| {
            if !cut("cols") || v.iter().all(|x| *x) {
                None
            } else {
                Some(v.iter().enumerate().filter(|(_, x)| **x).map(|(c, _)| c).collect::<Vec<_>>().into())
            }
        })
        .collect()
}

fn keyed<'s, Tm: timely::progress::Timestamp + differential_dataflow::lattice::Lattice>(c: C<'s, Tm>, key: Key, cols: Cols, name: &str) -> Arr<'s, Tm, Row> {
    c.flat_map(move |(w, row)| {
        let mut vr = None;
        key_of(&row, &key, &mut vr).map(|k| (k, (w, narrow(&row, &cols))))
    })
    .arrange_by_key_named(name)
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

/// Presence per key from its per-tag counts: the base emits [c_0 > 0]; tag t emits
/// [below + d_t > 0] − [below > 0].
fn presence_logic(_k: &Row, vals: &[(&(W, ()), R)], out: &mut Vec<((W, ()), R)>) {
    let per = per_tag(vals);
    let at = |t: W| per.iter().find(|(w, _)| *w == t).map(|(_, c)| *c).unwrap_or(0);
    if at(BASE) > 0 {
        out.push(((BASE, ()), 1));
    }
    for &(t, d) in &per {
        if t == BASE {
            continue;
        }
        let ch = tags::chain(t);
        let below: R = ch[..ch.len() - 1].iter().map(|x| at(*x)).sum();
        let f = flip(below, below + d);
        if f != 0 {
            out.push(((t, ()), f));
        }
    }
}

/// Presence of keys (count-based): `key -> (tag, ())` with ±1 deltas.
fn presence<'s, Tm: timely::progress::Timestamp + differential_dataflow::lattice::Lattice>(keys: VecCollection<'s, Tm, (Row, (W, ())), R>, name: &str) -> Arr<'s, Tm, ()> {
    keys.arrange_by_key_named(&format!("{name}:presence-in")).reduce_abelian::<_, ValBuilder<Row, (W, ()), Tm, R>, ValSpine<Row, (W, ()), Tm, R>, _, _>(
        &format!("{name}:presence-out"),
        presence_logic,
        |vec, key, upds| {
            vec.clear();
            vec.extend(upds.drain(..).map(|(v, t, r)| ((key.clone(), v), t, r)));
        },
    )
}

/// The same presence outside fixpoints, as a stream: one arrangement (its input counts).
fn presence_stream<'s>(keys: VecCollection<'s, T, (Row, (W, ())), R>, name: &str) -> VecCollection<'s, T, (Row, (W, ())), R> {
    if !cut("total") {
        return presence(keys, name).as_collection(|k, v| (k.clone(), *v));
    }
    reduce_total(keys.arrange_by_key_named(&format!("{name}:presence-in")), name, presence_logic)
}

/// Explore slice X1: the cuts switched on (`FACTS_X1_CUTS`, comma-separated; default all).
fn cut(name: &str) -> bool {
    thread_local! {
        static CUTS: Vec<String> = std::env::var("FACTS_X1_CUTS").map(|s| s.split(',').map(|x| x.trim().to_owned()).collect()).unwrap_or_else(|_| vec!["all".into()]);
    }
    CUTS.with(|c| c.iter().any(|x| x == name || x == "all"))
}

/// Reduce over a totally ordered time (outside fixpoints) without an output arrangement: per key
/// changed at time t, the output is `logic(input through t) − logic(input before t)`, both read
/// from the input arrangement alone (DD's `threshold_total`, generalised to values). The output
/// is a stream; a consumer that needs it arranged arranges it.
fn reduce_total<'s, V2, O>(input: Arr<'s, T, V2>, name: &str, logic: impl Fn(&Row, &[(&(W, V2), R)], &mut Vec<((W, O), R)>) + 'static) -> VecCollection<'s, T, (Row, (W, O)), R>
where
    V2: differential_dataflow::ExchangeData + Ord + Clone,
    O: differential_dataflow::ExchangeData + Ord + Clone,
{
    use differential_dataflow::trace::{BatchReader, Cursor, TraceReader};
    use timely::dataflow::channels::pact::Pipeline;
    use timely::dataflow::operators::Operator;
    use timely::progress::Antichain;
    let mut trace = input.trace.clone();
    input
        .stream
        .unary_frontier(Pipeline, &format!("{name}:reduce-total"), move |_, _| {
            let mut lower = Antichain::from_elem(0 as T);
            let mut upper = Antichain::from_elem(0 as T);
            let mut state: BTreeMap<(W, V2), R> = BTreeMap::new();
            let mut ups: Vec<(T, (W, V2), R)> = Vec::new();
            let (mut before, mut after): (Vec<((W, O), R)>, Vec<((W, O), R)>) = (Vec::new(), Vec::new());
            move |(inp, _frontier), output| {
                lower.clone_from(&upper);
                let mut batches = Vec::new();
                let mut cap = None;
                inp.for_each(|c, bs| {
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
                    let (mut tc, ts) = trace.cursor_through(lower.borrow()).expect("reduce_total: trace compacted past its lower");
                    while let Some(key) = bc.get_key(&bs) {
                        state.clear();
                        tc.seek_key(&ts, key);
                        if tc.get_key(&ts) == Some(key) {
                            while let Some(v) = tc.get_val(&ts) {
                                let mut c: R = 0;
                                tc.map_times(&ts, |_, d| c += *d);
                                if c != 0 {
                                    state.insert(v.clone(), c);
                                }
                                tc.step_val(&ts);
                            }
                        }
                        ups.clear();
                        while let Some(v) = bc.get_val(&bs) {
                            bc.map_times(&bs, |t, d| ups.push((*t, v.clone(), *d)));
                            bc.step_val(&bs);
                        }
                        ups.sort_by_key(|u| u.0);
                        let k: &Row = key;
                        before.clear();
                        // As `reduce`: a key without input has no output (its logic is not run).
                        if !state.is_empty() {
                            logic(k, &state.iter().map(|(v, c)| (v, *c)).collect::<Vec<_>>(), &mut before);
                        }
                        let mut i = 0;
                        while i < ups.len() {
                            let t = ups[i].0;
                            while i < ups.len() && ups[i].0 == t {
                                let (_, v, d) = &ups[i];
                                let e = state.entry(v.clone()).or_default();
                                *e += *d;
                                if *e == 0 {
                                    state.remove(v);
                                }
                                i += 1;
                            }
                            after.clear();
                            if !state.is_empty() {
                                logic(k, &state.iter().map(|(v, c)| (v, *c)).collect::<Vec<_>>(), &mut after);
                            }
                            let mut diff: Vec<((W, O), R)> = after.iter().cloned().chain(before.iter().map(|(x, r)| (x.clone(), -r))).collect();
                            differential_dataflow::consolidation::consolidate(&mut diff);
                            for (x, r) in diff {
                                session.give(((k.clone(), x), t, r));
                            }
                            std::mem::swap(&mut before, &mut after);
                        }
                        bc.step_key(&bs);
                    }
                }
                trace.advance_upper(&mut upper);
                trace.set_logical_compaction(upper.borrow());
                trace.set_physical_compaction(upper.borrow());
            }
        })
        .as_collection()
}

/// Group recompute: per key, logic over each tag's view, emitted as differences. Returns the
/// output and the refusal records (`(t, [construct])`, differences like the output's): a group
/// whose logic refuses in view t yields no rows there and one live refusal.
fn group<'s>(input: Arr<'s, T, Row>, logic: Rc<dyn Fn(&Row, &[(&Row, R)]) -> Vec<(Row, R)>>, sentinel: bool, name: &str) -> (C<'s, T>, C<'s, T>) {
    let at = name.to_owned();
    let f = move |k: &Row, vals: &[(&(W, Row), R)], out: &mut Vec<((W, Row), R)>| {
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
    };
    let all: C<'s, T> = if cut("total") {
        reduce_total(input, name, f).map(|(_k, (w, r))| (w, r))
    } else {
        input
            .reduce_abelian::<_, ValBuilder<Row, (W, Row), T, R>, ValSpine<Row, (W, Row), T, R>, _, _>(&format!("{name}:group-out"), f, |vec, key, upds| {
                vec.clear();
                vec.extend(upds.drain(..).map(|(v, t, r)| ((key.clone(), v), t, r)));
            })
            .as_collection(|_k, (w, r)| (*w, r.clone()))
    };
    split_refusals(all)
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
fn agg_logic(keys: Key, aggs: Vec<AggCall<RepoSource>>, width: usize, order: Vec<(X, bool)>) -> Rc<dyn Fn(&Row, &[(&Row, R)]) -> Vec<(Row, R)>> {
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

fn window_logic(order: Vec<(X, bool)>, kind: WindowKind<RepoSource>) -> Rc<dyn Fn(&Row, &[(&Row, R)]) -> Vec<(Row, R)>> {
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

fn distinct_prefix_logic(prefix: usize) -> Rc<dyn Fn(&Row, &[(&Row, R)]) -> Vec<(Row, R)>> {
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
}

/// A stage's stored row: each column under its declared affinity (CTAS).
fn affinity(row: &[Val], affs: &[rerail_facts::ir::Aff]) -> Row {
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
    /// Refusal records raised by the plan's operators (tag, [construct]).
    refused: Vec<C<'s, T>>,
    /// Per node, the columns its arrangements keep (`demand`).
    cols: Rc<Vec<Cols>>,
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
fn var_readers(g: &Graph, var: u32) -> Vec<bool> {
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
fn op_inputs(op: &Op) -> Vec<NodeId> {
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
        if !super::prof::on() {
            return self.node_raw(n);
        }
        let lo = super::prof::begin(self.scope.worker());
        let c = self.node_raw(n);
        super::prof::end(self.scope.worker(), lo, &self.name, n, "delta", op_kind(&self.g.nodes[n]), &self.g.labels[n]);
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
                self.refused.push(refused);
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
                self.refused.push(refused);
                data
            }
            Op::Affinity { input, affs } => {
                let affs = affs.clone();
                self.node(*input).map(move |(w, row)| (w, affinity(&row, &affs)))
            }
            Op::Join { l, r, lk, rk } => {
                let la = self.arr(*l, lk);
                let ra = self.arr(*r, rk);
                let (cl, cr, wl, wr) = (self.cols[*l].clone(), self.cols[*r].clone(), g.widths[*l], g.widths[*r]);
                la.join_core(ra, move |_k, (w1, a), (w2, b)| {
                    tags::product(*w1, *w2).map(|w| (w, joined(a, &cl, wl, b, &cr, wr)))
                })
            }
            Op::Semi { l, r, lk, rk, anti } => {
                let (cl, wl) = (self.cols[*l].clone(), g.widths[*l]);
                let left = self.node(*l);
                let la = self.arr(*l, lk);
                let pres = self.pres(*r, rk);
                let c2 = cl.clone();
                let hit = la.join_core(pres, move |_k, (w1, a), (w2, ())| tags::product(*w1, *w2).map(|w| (w, widen(a, &c2, wl))));
                // The antijoin's left rows masked like the arranged ones, so that hits cancel.
                let left = if cl.is_some() { left.map(move |(w, row)| (w, widen(&narrow(&row, &cl), &cl, wl))) } else { left };
                if *anti { left.concat(hit.negate()) } else { hit }
            }
            Op::Distinct { input } => {
                let keys = self.node(*input).map(|(w, row)| (row, (w, ())));
                presence_stream(keys, &format!("n{n}:distinct")).map(|(k, (w, ()))| (w, k))
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
                let (data, refused) = group(arr, agg_logic(keys.clone(), aggs.clone(), *width, order.clone()), *keyless, &format!("n{n}"));
                self.refused.push(refused);
                data
            }
            Op::Window { input, part, order, kind } => {
                let arr = self.arr(*input, part);
                let (data, refused) = group(arr, window_logic(order.clone(), kind.clone()), false, &format!("n{n}"));
                self.refused.push(refused);
                data
            }
            Op::DistinctPrefix { input, prefix } => {
                let p = *prefix;
                let key: Key = (0..p).map(|c| (super::norm::col(c), KeyMode::Group)).collect();
                let arr = self.arr(*input, &key);
                let (data, refused) = group(arr, distinct_prefix_logic(p), false, &format!("n{n}"));
                self.refused.push(refused);
                data
            }
            Op::Fix { var, init, step } => self.fix(*var, init.clone(), step.clone()),
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
        let a = keyed(self.node(n), key.clone(), self.cols[n].clone(), &format!("n{n}:by{}", self.arrs.len()));
        self.arrs.insert(k, a.clone());
        a
    }

    /// Count-based presence of node `n`'s keys under `key`, one per (node, key).
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
        let a = presence(keys, &format!("n{n}:pres{}", self.press.len()));
        self.press.insert(k, a.clone());
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
                    let a = self.pres(*r, rk);
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
    fn fix(&mut self, var: u32, init: Vec<NodeId>, step: Vec<NodeId>) -> C<'s, T> {
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
        let cols = self.cols.clone();
        let result = seeds.clone().iterate(|inner, rec| {
            let mut env: FxHashMap<NodeId, C<'_, IT>> = FxHashMap::default();
            for (b, c) in &bnd_cols {
                env.insert(*b, c.clone().enter(inner));
            }
            let earrs = outer_arrs.iter().map(|(k, a)| (k.clone(), a.clone().enter(inner))).collect();
            let epress = outer_press.iter().map(|(k, a)| (k.clone(), a.clone().enter(inner))).collect();
            let mut vl = IterLower { g: g2.clone(), gname: gname.clone(), rd: rd.clone(), rec: rec.clone(), env, earrs, epress, done: FxHashMap::default(), cols: cols.clone() };
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
            let out = presence(out.map(|(w, row)| (row, (w, ()))), &format!("fix{var}:iterate-distinct")).as_collection(|row, (w, ())| (*w, row.clone()));
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
    cols: Rc<Vec<Cols>>,
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
                let (cl, cr, wl, wr) = (self.cols[*l].clone(), self.cols[*r].clone(), g.widths[*l], g.widths[*r]);
                // The bilinear expansion, as outside: a pair (t1, t2) yields product(t1, t2).
                let (c1, c2) = (cl.clone(), cr.clone());
                let pair = move |_k: &Row, (w1, a): &(W, Row), (w2, b): &(W, Row)| tags::product(*w1, *w2).map(|w| (w, joined(a, &c1, wl, b, &c2, wr)));
                match (lv, rv) {
                    (true, true) => {
                        let la = keyed(self.node(*l), lk.clone(), cl, &format!("n{n}:iter-join-l"));
                        let ra = keyed(self.node(*r), rk.clone(), cr, &format!("n{n}:iter-join-r"));
                        la.join_core(ra, pair)
                    }
                    (true, false) => {
                        let la = keyed(self.node(*l), lk.clone(), cl, &format!("n{n}:iter-join-l"));
                        let ra = self.earrs[&(*r, format!("{rk:?}"))].clone();
                        la.join_core(ra, pair)
                    }
                    (false, true) => {
                        let la = self.earrs[&(*l, format!("{lk:?}"))].clone();
                        let ra = keyed(self.node(*r), rk.clone(), cr, &format!("n{n}:iter-join-r"));
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
                let (cl, wl) = (self.cols[*l].clone(), g.widths[*l]);
                let left = self.node(*l);
                let c2 = cl.clone();
                let hit_of = move |_k: &Row, (w1, a): &(W, Row), (w2, ()): &(W, ())| tags::product(*w1, *w2).map(|w| (w, widen(a, &c2, wl)));
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
                        keyed(left.clone(), lk.clone(), cl.clone(), &format!("n{n}:iter-semi-l")).join_core(pres, hit_of)
                    }
                    (true, false) => {
                        let pres = self.epress[&(*r, format!("{rk:?}"))].clone();
                        keyed(left.clone(), lk.clone(), cl.clone(), &format!("n{n}:iter-semi-l")).join_core(pres, hit_of)
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
                let left = if cl.is_some() { left.map(move |(w, row)| (w, widen(&narrow(&row, &cl), &cl, wl))) } else { left };
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

/// Build one dataflow evaluating every graph's outputs over the tagged `files` input.
pub fn build(worker: &mut timely::worker::Worker, graphs: Vec<(String, Rc<Graph>)>, src: Source, probe: &Handle<T>, track_on: bool) -> Built {
    worker.dataflow::<T, _, _>(|scope| {
        let (files_in, files) = scope.new_collection::<(W, (Sym, Cid)), R>();
        let mut outputs = Vec::new();
        let mut refusals = Vec::new();
        for (name, g) in graphs {
            let n = g.nodes.len();
            let gi = outputs.len();
            let cols = demand(&g);
            let (narrowed, kept, width): (usize, usize, usize) = cols.iter().enumerate().filter_map(|(i, c)| c.as_ref().map(|c| (1, c.len(), g.widths[i]))).fold((0, 0, 0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2));
            eprintln!("demand {name}: {narrowed} of {n} nodes narrowed, {kept} of {width} columns kept");
            let mut lw = Lower { scope, g: g.clone(), files: files.clone(), src: src.clone(), name: name.clone(), done: vec![None; n], track: track_on.then_some(gi * 100_000), arrs: FxHashMap::default(), press: FxHashMap::default(), refused: Vec::new(), cols: Rc::new(cols) };
            // A construct refused while lowering (inside a recursion: a non-stratified antijoin,
            // an aggregate over the recursive relation) leaves part of the plan empty: the plan's
            // findings are withheld everywhere by a base refusal record, never reported partial.
            let (_, at_build) = captured(|| {
                for (clause, node, _) in &g.outputs {
                    let c = lw.node(*node);
                    outputs.push((format!("{name}:{clause}"), capture(c, probe)));
                }
            });
            let built: Vec<((W, Row), T, R)> = at_build.iter().map(|w| ((BASE, refusal_row(&name, w)), 0, 1)).collect();
            let refused = lw.refused.drain(..).fold(built.to_stream(scope).as_collection(), |a, c| a.concat(c));
            refusals.push((name.clone(), capture(refused, probe)));
        }
        Built { files: files_in, outputs, refusals }
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
