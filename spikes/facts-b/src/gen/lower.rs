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
//! - fixpoint: per view, semi-naive (differential `iterate`) over view-tagged records, materialised
//!   only for the views whose delta touches the fixpoint's inputs; results re-enter delta mode as
//!   R_v − R_parent-view.
use super::norm::{Graph, Key, KeyMode, NodeId, Op, X};
use super::tags::{self, W, BASE};
use super::val::{self, Row, Val};
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
    vec![Val::Text(val::sym(&format!("{at}: {what}")))]
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
        let size = (std::mem::size_of::<((W, Row), T, R)>() + row.capacity() * std::mem::size_of::<Val>()) as u32;
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

fn canon(v: Val) -> Val {
    match v {
        Val::Real(b) => {
            let f = f64::from_bits(b);
            if f.fract() == 0.0 && f.abs() < 9.2e18 { Val::Int(f as i64) } else { v }
        }
        x => x,
    }
}

/// A row's key under `key`; `None` when an equality key is NULL (it matches nothing).
fn key_of(row: &[Val], key: &Key, vrow: &mut Option<Vec<V>>) -> Option<Row> {
    let mut k = Vec::with_capacity(key.len());
    for (x, mode) in key {
        let fast = match x {
            rerail_facts::ir::Expr::Col { depth: 0, col } => Some(row[*col as usize]),
            _ => None,
        };
        let v = match (fast, mode) {
            (Some(v), KeyMode::Group) => canon(v),
            (Some(Val::Null), KeyMode::Eq(_)) => return None,
            (Some(v), KeyMode::Eq(rerail_facts::ir::Aff::None)) => canon(v),
            _ => {
                let r = vrow.get_or_insert_with(|| val::row_to_v(row));
                let v = ev(x, r)?;
                match mode {
                    KeyMode::Group => val::group_key(&v),
                    KeyMode::Eq(aff) => val::eq_key(&sv::compare_affinity(&v, *aff))?,
                }
            }
        };
        k.push(v);
    }
    Some(k)
}

fn keyed<'s, Tm: timely::progress::Timestamp + differential_dataflow::lattice::Lattice>(c: C<'s, Tm>, key: Key, name: &str) -> Arr<'s, Tm, Row> {
    c.flat_map(move |(w, row)| {
        let mut vr = None;
        key_of(&row, &key, &mut vr).map(|k| (k, (w, row)))
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

/// Presence of keys (count-based): `key -> (tag, ())` with ±1 deltas.
fn presence<'s, Tm: timely::progress::Timestamp + differential_dataflow::lattice::Lattice>(keys: VecCollection<'s, Tm, (Row, (W, ())), R>, name: &str) -> Arr<'s, Tm, ()> {
    keys.arrange_by_key_named(&format!("{name}:presence-in")).reduce_abelian::<_, ValBuilder<Row, (W, ()), Tm, R>, ValSpine<Row, (W, ()), Tm, R>, _, _>(
        &format!("{name}:presence-out"),
        |_k, vals: &[(&(W, ()), R)], out: &mut Vec<((W, ()), R)>| {
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
        },
        |vec, key, upds| {
            vec.clear();
            vec.extend(upds.drain(..).map(|(v, t, r)| ((key.clone(), v), t, r)));
        },
    )
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
        )
        .as_collection(|_k, (w, r)| (*w, r.clone()));
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

fn agg_logic(keys: Key, aggs: Vec<AggCall<RepoSource>>, width: usize) -> Rc<dyn Fn(&Row, &[(&Row, R)]) -> Vec<(Row, R)>> {
    let key_cols: Vec<usize> = keys.iter().filter_map(|(k, _)| if let rerail_facts::ir::Expr::Col { depth: 0, col } = k { Some(*col as usize) } else { None }).collect();
    Rc::new(move |_k, members| {
        let rows = vals_of(members);
        let mut out: Row = vec![Val::Null; width];
        for &c in &key_cols {
            let first = members.first().map(|(r, _)| r[c]).unwrap_or(Val::Null);
            if members.iter().any(|(r, _)| r[c] != first) {
                refuse("GROUP BY key with several representations in one group (1 vs 1.0)");
                return Vec::new();
            }
            out[c] = first;
        }
        for a in &aggs {
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
        vec![(out, 1)]
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
                        let mut r = members[i].0.clone();
                        r.push(Val::Int(n));
                        out.push((r, 1));
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
                let b = best.as_ref().map(val::from_v).unwrap_or(Val::Null);
                members
                    .iter()
                    .map(|(r, c)| {
                        let mut r = (*r).clone();
                        r.push(b);
                        (r, *c)
                    })
                    .collect()
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
    done: Vec<Option<C<'s, T>>>,
    /// `Some(graph offset)`: count every operator output's live records per tag.
    track: Option<usize>,
    /// Arrangements and presences by (node, key), shared by every operator that reads them.
    arrs: FxHashMap<(NodeId, String), Arr<'s, T, Row>>,
    press: FxHashMap<(NodeId, String), Arr<'s, T, ()>>,
    /// Refusal records raised by the plan's operators (tag, [construct]).
    refused: Vec<C<'s, T>>,
}

/// Does node `n` read the fixpoint variable `var` (directly or below)?
fn reads_var(g: &Graph, n: NodeId, var: u32) -> bool {
    match &g.nodes[n] {
        Op::Var(v) => *v == var,
        Op::Exported(_) | Op::Const(_) => false,
        Op::Map { input, .. } | Op::Filter { input, .. } | Op::Distinct { input } | Op::Group { input, .. } | Op::Window { input, .. } | Op::DistinctPrefix { input, .. } | Op::Affinity { input, .. } => reads_var(g, *input, var),
        Op::Join { l, r, .. } | Op::Semi { l, r, .. } => reads_var(g, *l, var) || reads_var(g, *r, var),
        Op::Concat { inputs } => inputs.iter().any(|i| reads_var(g, *i, var)),
        Op::Fix { init, step, .. } => init.iter().chain(step).any(|i| reads_var(g, *i, var)),
    }
}

/// Delta-mode nodes a fixpoint body reads (the boundary of the recursive part).
fn boundary(g: &Graph, n: NodeId, var: u32, out: &mut Vec<NodeId>) {
    if !reads_var(g, n, var) {
        if !out.contains(&n) {
            out.push(n);
        }
        return;
    }
    match &g.nodes[n] {
        Op::Var(_) => {}
        Op::Map { input, .. } | Op::Filter { input, .. } | Op::Distinct { input } | Op::Group { input, .. } | Op::Window { input, .. } | Op::DistinctPrefix { input, .. } | Op::Affinity { input, .. } => boundary(g, *input, var, out),
        Op::Join { l, r, .. } | Op::Semi { l, r, .. } => {
            boundary(g, *l, var, out);
            boundary(g, *r, var, out)
        }
        Op::Concat { inputs } => inputs.iter().for_each(|i| boundary(g, *i, var, out)),
        Op::Fix { init, step, .. } => init.iter().chain(step).for_each(|i| boundary(g, *i, var, out)),
        Op::Exported(_) | Op::Const(_) => {}
    }
}

impl<'s> Lower<'s> {
    fn node(&mut self, n: NodeId) -> C<'s, T> {
        if let Some(c) = &self.done[n] {
            return c.clone();
        }
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
                let all = self.node(*input).flat_map(move |(w, row)| {
                    let vr = val::row_to_v(&row);
                    let mut out = Vec::with_capacity(exprs.len());
                    let (ok, refused) = captured(|| {
                        for e in &exprs {
                            if let rerail_facts::ir::Expr::Col { depth: 0, col } = e {
                                out.push(row[*col as usize]);
                                continue;
                            }
                            match ev(e, &vr) {
                                Some(v) => out.push(val::from_v(&v)),
                                None => return false,
                            }
                        }
                        true
                    });
                    match refused.first() {
                        Some(r) => Some((w | REFUSED, refusal_row(&at, r))),
                        None => ok.then_some((w, out)),
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
                la.join_core(ra, |_k, (w1, a), (w2, b)| {
                    tags::product(*w1, *w2).map(|w| {
                        let mut r = a.clone();
                        r.extend_from_slice(b);
                        (w, r)
                    })
                })
            }
            Op::Semi { l, r, lk, rk, anti } => {
                let left = self.node(*l);
                let la = self.arr(*l, lk);
                let pres = self.pres(*r, rk);
                let hit = la.join_core(pres, |_k, (w1, a), (w2, ())| tags::product(*w1, *w2).map(|w| (w, a.clone())));
                if *anti { left.concat(hit.negate()) } else { hit }
            }
            Op::Distinct { input } => {
                let keys = self.node(*input).map(|(w, row)| (row, (w, ())));
                presence(keys, &format!("n{n}:distinct")).as_collection(|k, (w, ())| (*w, k.clone()))
            }
            Op::Concat { inputs } => {
                let mut it = inputs.clone().into_iter();
                let first = self.node(it.next().unwrap());
                it.fold(first, |a, i| a.concat(self.node(i)))
            }
            Op::Group { input, keys, aggs, keyless, width } => {
                let mut c = self.node(*input);
                if *keyless {
                    // One group even over no rows: a sentinel member under the base.
                    let s: Vec<((W, Row), T, R)> = vec![((BASE, Vec::new()), 0, 1)];
                    c = c.concat(s.to_stream(self.scope).as_collection());
                }
                let kk = keys.clone();
                let arr = c
                    .flat_map(move |(w, row)| {
                        if row.is_empty() {
                            return Some((Vec::new(), (w, row)));
                        }
                        let mut vr = None;
                        key_of(&row, &kk, &mut vr).map(|k| (k, (w, row)))
                    })
                    .arrange_by_key_named(&format!("n{n}:group-in"));
                let (data, refused) = group(arr, agg_logic(keys.clone(), aggs.clone(), *width), *keyless, &format!("n{n}"));
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
        let a = keyed(self.node(n), key.clone(), &format!("n{n}:by{}", self.arrs.len()));
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
    fn boundary_arrs(&mut self, var: u32, n: NodeId, arrs: &mut Vec<((NodeId, String), Arr<'s, T, Row>)>, press: &mut Vec<((NodeId, String), Arr<'s, T, ()>)>, seen: &mut Vec<NodeId>) {
        let g = self.g.clone();
        if !reads_var(&g, n, var) || seen.contains(&n) {
            return;
        }
        seen.push(n);
        match &g.nodes[n] {
            Op::Join { l, r, lk, rk } => {
                for (s, k) in [(*l, lk), (*r, rk)] {
                    if reads_var(&g, s, var) {
                        self.boundary_arrs(var, s, arrs, press, seen);
                    } else {
                        let a = self.arr(s, k);
                        arrs.push(((s, format!("{k:?}")), a));
                    }
                }
            }
            Op::Semi { l, r, lk, rk, .. } => {
                if reads_var(&g, *l, var) {
                    self.boundary_arrs(var, *l, arrs, press, seen);
                } else {
                    let a = self.arr(*l, lk);
                    arrs.push(((*l, format!("{lk:?}")), a));
                }
                if reads_var(&g, *r, var) {
                    self.boundary_arrs(var, *r, arrs, press, seen);
                } else {
                    let a = self.pres(*r, rk);
                    press.push(((*r, format!("{rk:?}")), a));
                }
            }
            Op::Map { input, .. } | Op::Filter { input, .. } | Op::Distinct { input } | Op::Group { input, .. } | Op::Window { input, .. } | Op::DistinctPrefix { input, .. } | Op::Affinity { input, .. } => self.boundary_arrs(var, *input, arrs, press, seen),
            Op::Concat { inputs } => inputs.iter().for_each(|i| self.boundary_arrs(var, *i, arrs, press, seen)),
            Op::Fix { init, step, .. } => init.iter().chain(step).for_each(|i| self.boundary_arrs(var, *i, arrs, press, seen)),
            Op::Var(_) | Op::Exported(_) | Op::Const(_) => {}
        }
    }

    /// Set fixpoint, materialised per touched view.
    fn fix(&mut self, var: u32, init: Vec<NodeId>, step: Vec<NodeId>) -> C<'s, T> {
        let g = self.g.clone();
        // Delta-mode inputs of the body.
        let mut bnd = Vec::new();
        for &s in &step {
            boundary(&g, s, var, &mut bnd);
        }
        let bnd_cols: Vec<(NodeId, C<'s, T>)> = bnd.iter().map(|&b| (b, self.node(b))).collect();
        let (mut outer_arrs, mut outer_press, mut seen) = (Vec::new(), Vec::new(), Vec::new());
        for &s in &step {
            self.boundary_arrs(var, s, &mut outer_arrs, &mut outer_press, &mut seen);
        }
        let init_d: C<'s, T> = {
            let mut it = init.iter();
            let first = self.node(*it.next().unwrap());
            it.fold(first, |a, i| a.concat(self.node(*i)))
        };
        // Views materialised: every tag holding a delta of the worktree's file set (a superset of
        // the tags whose delta reaches the fixpoint's inputs; a view equal to its parent's
        // contributes R_v − R_pv = 0, so the superset is exact).
        let touched = self
            .files
            .clone()
            .filter(|(w, _)| *w != BASE)
            .threshold_named(&format!("fix{var}:touched-1"), |_, c| if *c != 0 { 1 } else { 0 })
            .map(|(w, _)| w)
            .threshold_named(&format!("fix{var}:touched-2"), |_, c| if *c > 0 { 1 } else { 0 });
        // View seeds: the base view reads base records; a touched view v reads its chain.
        let base_seed = init_d.clone().filter(|(w, _)| *w == BASE);
        let chain_pairs = touched.clone().flat_map(|v| tags::chain(v).into_iter().map(move |t| (vec![Val::Int(t as i64)], (BASE, Val::Int(v as i64)))));
        let by_tag = init_d.clone().map(|(w, row)| (vec![Val::Int(w as i64)], (BASE, row))).arrange_by_key_named(&format!("fix{var}:init-by-tag"));
        let view_seed = chain_pairs
            .map(|(k, (b, v))| (k, (b, vec![v])))
            .arrange_by_key_named(&format!("fix{var}:chain"))
            .join_core(by_tag, |_t, (_, v), (_, row)| match v[0] {
                Val::Int(v) => Some((v as W, row.clone())),
                _ => None,
            });
        let seeds = base_seed.concat(view_seed);
        // The boundary relations' arrangements are the delta-mode ones built above, entered.
        let g2 = g.clone();
        let result = seeds.clone().iterate(|inner, rec| {
            let mut env: FxHashMap<NodeId, C<'_, IT>> = FxHashMap::default();
            for (b, c) in &bnd_cols {
                env.insert(*b, c.clone().enter(inner));
            }
            let earrs = outer_arrs.iter().map(|(k, a)| (k.clone(), a.clone().enter(inner))).collect();
            let epress = outer_press.iter().map(|(k, a)| (k.clone(), a.clone().enter(inner))).collect();
            let mut vl = ViewLower { g: g2.clone(), var, rec: rec.clone(), env, earrs, epress, done: FxHashMap::default() };
            let mut out = seeds.clone().enter(inner);
            for &s in &step {
                let mut d = vl.node(s);
                if crate::mutant::on(6) {
                    d = TFilter::filter(d.inner, |(_, t, _): &((W, Row), IT, R)| t.inner < 2).as_collection();
                }
                out = out.concat(d);
            }
            out.threshold_named(&format!("fix{var}:iterate-distinct"), |_, c| if *c > 0 { 1 } else { 0 })
        });
        // Back to delta mode: base view under 0; a touched view v: R_v − R_pv, pv its parent view
        // if touched, else the base view.
        let par = touched.clone().map(|v| (vec![Val::Int(tags::parent(v) as i64)], (BASE, vec![Val::Int(v as i64)])));
        let tset = touched.clone().map(|v| (vec![Val::Int(v as i64)], (BASE, ()))).arrange_by_key_named(&format!("fix{var}:tset"));
        let par_arr = par.clone().arrange_by_key_named(&format!("fix{var}:parent"));
        let with_parent = par_arr.join_core(tset, |p, (_, v), _| Some((p.clone(), v.clone())));
        let without = par.concat(with_parent.clone().map(|(p, v)| (p, (BASE, v))).negate()).map(|(_, (_, v))| (vec![Val::Int(0)], v));
        let pv_pairs = with_parent.concat(without).map(|(pv, v)| (pv, (BASE, v))).arrange_by_key_named(&format!("fix{var}:pv-pairs"));
        let res_by_view = result.clone().map(|(v, row)| (vec![Val::Int(v as i64)], (BASE, row))).arrange_by_key_named(&format!("fix{var}:result-by-view"));
        let minus = res_by_view.join_core(pv_pairs, |_pv, (_, row), (_, v)| match v[0] {
            Val::Int(v) => Some((v as W, row.clone())),
            _ => None,
        });
        result.concat(minus.negate())
    }
}

type EArr<'i, V2> = Arranged<'i, TraceEnter<TraceAgent<ValSpine<Row, (W, V2), T, R>>, IT>>;

/// View-mode lowering inside a fixpoint: records `(view, row)` are full rows of a view.
struct ViewLower<'i> {
    g: Rc<Graph>,
    var: u32,
    rec: C<'i, IT>,
    /// Delta-mode boundary collections, entered.
    env: FxHashMap<NodeId, C<'i, IT>>,
    /// Delta-mode boundary arrangements (by node and key) and presences, entered.
    earrs: FxHashMap<(NodeId, String), EArr<'i, Row>>,
    epress: FxHashMap<(NodeId, String), EArr<'i, ()>>,
    done: FxHashMap<NodeId, C<'i, IT>>,
}

impl<'i> ViewLower<'i> {
    fn is_view(&self, n: NodeId) -> bool {
        reads_var(&self.g, n, self.var)
    }

    fn node(&mut self, n: NodeId) -> C<'i, IT> {
        if let Some(c) = self.done.get(&n) {
            return c.clone();
        }
        if !self.is_view(n) {
            return self.env[&n].clone();
        }
        let g = self.g.clone();
        let c = match &g.nodes[n] {
            Op::Var(_) => self.rec.clone(),
            Op::Map { input, exprs } => {
                let exprs = exprs.clone();
                self.node(*input).flat_map(move |(w, row)| {
                    let vr = val::row_to_v(&row);
                    let mut out = Vec::with_capacity(exprs.len());
                    for e in &exprs {
                        match ev(e, &vr) {
                            Some(v) => out.push(val::from_v(&v)),
                            None => return None,
                        }
                    }
                    Some((w, out))
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
            Op::Distinct { input } => self.node(*input).threshold_named(&format!("n{n}:view-distinct"), |_, c| if *c > 0 { 1 } else { 0 }),
            Op::Join { l, r, lk, rk } => {
                let (lv, rv) = (self.is_view(*l), self.is_view(*r));
                // view×view: same view; view×delta: the delta record must be visible in the view.
                let pair = move |w1: &W, a: &Row, w2: &W, b: &Row| -> Option<(W, Row)> {
                    let w = match (lv, rv) {
                        (true, true) => (w1 == w2).then_some(*w1),
                        (true, false) => tags::visible(*w2, *w1).then_some(*w1),
                        (false, true) => tags::visible(*w1, *w2).then_some(*w2),
                        (false, false) => unreachable!(),
                    }?;
                    if lv && rv && *w1 == *w2 && *w1 != BASE && crate::mutant::on(4) {
                        return None;
                    }
                    let mut r = a.clone();
                    r.extend_from_slice(b);
                    Some((w, r))
                };
                match (lv, rv) {
                    (true, true) => {
                        let la = keyed(self.node(*l), lk.clone(), &format!("n{n}:view-join-l"));
                        let ra = keyed(self.node(*r), rk.clone(), &format!("n{n}:view-join-r"));
                        la.join_core(ra, move |_k, (w1, a), (w2, b)| pair(w1, a, w2, b))
                    }
                    (true, false) => {
                        let la = keyed(self.node(*l), lk.clone(), &format!("n{n}:view-join-l"));
                        let ra = self.earrs[&(*r, format!("{rk:?}"))].clone();
                        la.join_core(ra, move |_k, (w1, a), (w2, b)| pair(w1, a, w2, b))
                    }
                    (false, true) => {
                        let la = self.earrs[&(*l, format!("{lk:?}"))].clone();
                        let ra = keyed(self.node(*r), rk.clone(), &format!("n{n}:view-join-r"));
                        la.join_core(ra, move |_k, (w1, a), (w2, b)| pair(w1, a, w2, b))
                    }
                    (false, false) => unreachable!("delta-mode join in view mode"),
                }
            }
            Op::Semi { l, r, lk, rk, anti } => {
                let lv = self.is_view(*l);
                let rv = self.is_view(*r);
                if !lv && *anti {
                    refuse("antijoin of a non-recursive relation by a recursive one inside the recursion (not stratified)");
                    return self.rec.clone().filter(|_| false);
                }
                let left = self.node(*l);
                let vis = move |w1: &W, w2: &W| -> Option<W> {
                    match (lv, rv) {
                        (true, true) => (w1 == w2).then_some(*w1),
                        (true, false) => tags::visible(*w2, *w1).then_some(*w1),
                        (false, true) => tags::visible(*w1, *w2).then_some(*w2),
                        (false, false) => unreachable!(),
                    }
                };
                // A view right side: presence per (key, view), built here. A delta right side:
                // the count-based delta presence (shared with the delta-mode operators); summed
                // over the tags visible in a view it gives the view's presence (0 or 1).
                let view_pres = |s: &mut Self| {
                    let rk = rk.clone();
                    s.node(*r)
                        .flat_map(move |(w, row)| {
                            let mut vr = None;
                            key_of(&row, &rk, &mut vr).map(|k| (k, (w, ())))
                        })
                        .threshold_named(&format!("n{n}:view-semi-r"), |_, c| if *c > 0 { 1 } else { 0 })
                        .arrange_by_key_named(&format!("n{n}:view-semi-r-arr"))
                };
                let hit = match (lv, rv) {
                    (true, true) => {
                        let la = keyed(left.clone(), lk.clone(), &format!("n{n}:view-semi-l"));
                        la.join_core(view_pres(self), move |_k, (w1, a), (w2, ())| vis(w1, w2).map(|w| (w, a.clone())))
                    }
                    (true, false) => {
                        let la = keyed(left.clone(), lk.clone(), &format!("n{n}:view-semi-l"));
                        let pres = self.epress[&(*r, format!("{rk:?}"))].clone();
                        la.join_core(pres, move |_k, (w1, a), (w2, ())| vis(w1, w2).map(|w| (w, a.clone())))
                    }
                    (false, true) => {
                        let la = self.earrs[&(*l, format!("{lk:?}"))].clone();
                        la.join_core(view_pres(self), move |_k, (w1, a), (w2, ())| vis(w1, w2).map(|w| (w, a.clone())))
                    }
                    (false, false) => unreachable!("delta-mode semijoin in view mode"),
                };
                if *anti { left.concat(hit.negate()) } else { hit }
            }
            Op::Group { .. } | Op::Window { .. } | Op::DistinctPrefix { .. } => {
                refuse("aggregate, window or DISTINCT-prefix over a recursive relation (not stratified)");
                self.rec.clone().filter(|_| false)
            }
            Op::Fix { .. } => {
                refuse("nested recursion");
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
            let mut lw = Lower { scope, g: g.clone(), files: files.clone(), src: src.clone(), done: vec![None; n], track: track_on.then_some(gi * 100_000), arrs: FxHashMap::default(), press: FxHashMap::default(), refused: Vec::new() };
            for (clause, node, _) in &g.outputs {
                let c = lw.node(*node);
                outputs.push((format!("{name}:{clause}"), capture(c, probe)));
            }
            let none: Vec<((W, Row), T, R)> = Vec::new();
            let refused = lw.refused.drain(..).fold(none.to_stream(scope).as_collection(), |a, c| a.concat(c));
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
