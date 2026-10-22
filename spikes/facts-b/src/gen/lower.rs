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
/// a bare value was ambiguous, or a scalar raised. Counted per construct; any nonzero count means
/// the affected findings are withheld, never guessed.
#[derive(Default, Debug, Clone)]
pub struct Refusals(pub BTreeMap<String, u64>);

thread_local! {
    static REFUSALS: RefCell<Refusals> = RefCell::new(Refusals::default());
}

pub fn refusals() -> Refusals {
    REFUSALS.with(|r| r.borrow().clone())
}

fn refuse(what: &str) {
    REFUSALS.with(|r| *r.borrow_mut().0.entry(what.to_owned()).or_default() += 1);
}

pub type Out = Rc<RefCell<FxHashMap<(W, Row), R>>>;

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

fn keyed<'s, Tm: timely::progress::Timestamp + differential_dataflow::lattice::Lattice>(c: C<'s, Tm>, key: Key) -> Arr<'s, Tm, Row> {
    c.flat_map(move |(w, row)| {
        let mut vr = None;
        key_of(&row, &key, &mut vr).map(|k| (k, (w, row)))
    })
    .arrange_by_key()
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
fn presence<'s, Tm: timely::progress::Timestamp + differential_dataflow::lattice::Lattice>(keys: VecCollection<'s, Tm, (Row, (W, ())), R>) -> Arr<'s, Tm, ()> {
    keys.arrange_by_key().reduce_abelian::<_, ValBuilder<Row, (W, ()), Tm, R>, ValSpine<Row, (W, ()), Tm, R>, _, _>(
        "presence",
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

/// Group recompute: per key, logic over each tag's view, emitted as differences.
fn group<'s>(input: Arr<'s, T, Row>, logic: Rc<dyn Fn(&Row, &[(&Row, R)]) -> Vec<(Row, R)>>, sentinel: bool) -> C<'s, T> {
    input
        .reduce_abelian::<_, ValBuilder<Row, (W, Row), T, R>, ValSpine<Row, (W, Row), T, R>, _, _>(
            "group",
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
                let eval = |members: Vec<(&Row, R)>| -> Vec<(Row, R)> {
                    if members.is_empty() && !sentinel {
                        return Vec::new();
                    }
                    if members.iter().any(|(_, c)| *c < 0) {
                        refuse("negative group member (inconsistent delta)");
                        return Vec::new();
                    }
                    logic(k, &members)
                };
                let base = eval(view(BASE));
                for (r, c) in &base {
                    out.push(((BASE, r.clone()), *c));
                }
                for &t in &tags_at {
                    if t == BASE {
                        continue;
                    }
                    for (r, c) in eval(view(t)) {
                        out.push(((t, r), c));
                    }
                    let p = tags::parent(t);
                    let below = if p == BASE { base.clone() } else { eval(view(p)) };
                    for (r, c) in below {
                        out.push(((t, r), -c));
                    }
                }
                differential_dataflow::consolidation::consolidate(out);
            },
            |vec, key, upds| {
                vec.clear();
                vec.extend(upds.drain(..).map(|(v, t, r)| ((key.clone(), v), t, r)));
            },
        )
        .as_collection(|_k, (w, r)| (*w, r.clone()))
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
                    if order_cmp(&w[0].0, &w[1].0, &desc) == Ordering::Equal && members[w[0].1].0 != members[w[1].1].0 {
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
}

struct Lower<'s> {
    scope: Scope<'s, T>,
    g: Rc<Graph>,
    files: VecCollection<'s, T, (W, (Sym, Cid)), R>,
    src: Source,
    done: Vec<Option<C<'s, T>>>,
}

/// Does node `n` read the fixpoint variable `var` (directly or below)?
fn reads_var(g: &Graph, n: NodeId, var: u32) -> bool {
    match &g.nodes[n] {
        Op::Var(v) => *v == var,
        Op::Exported(_) | Op::Const(_) => false,
        Op::Map { input, .. } | Op::Filter { input, .. } | Op::Distinct { input } | Op::Group { input, .. } | Op::Window { input, .. } | Op::DistinctPrefix { input, .. } => reads_var(g, *input, var),
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
        Op::Map { input, .. } | Op::Filter { input, .. } | Op::Distinct { input } | Op::Group { input, .. } | Op::Window { input, .. } | Op::DistinctPrefix { input, .. } => boundary(g, *input, var, out),
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
                self.node(*input).flat_map(move |(w, row)| {
                    let vr = val::row_to_v(&row);
                    let mut out = Vec::with_capacity(exprs.len());
                    for e in &exprs {
                        if let rerail_facts::ir::Expr::Col { depth: 0, col } = e {
                            out.push(row[*col as usize]);
                            continue;
                        }
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
            Op::Join { l, r, lk, rk } => {
                let la = keyed(self.node(*l), lk.clone());
                let ra = keyed(self.node(*r), rk.clone());
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
                let la = keyed(left.clone(), lk.clone());
                let rk = rk.clone();
                let keys = self.node(*r).flat_map(move |(w, row)| {
                    let mut vr = None;
                    key_of(&row, &rk, &mut vr).map(|k| (k, (w, ())))
                });
                let pres = presence(keys);
                let hit = la.join_core(pres, |_k, (w1, a), (w2, ())| tags::product(*w1, *w2).map(|w| (w, a.clone())));
                if *anti { left.concat(hit.negate()) } else { hit }
            }
            Op::Distinct { input } => {
                let keys = self.node(*input).map(|(w, row)| (row, (w, ())));
                presence(keys).as_collection(|k, (w, ())| (*w, k.clone()))
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
                    .arrange_by_key();
                group(arr, agg_logic(keys.clone(), aggs.clone(), *width), *keyless)
            }
            Op::Window { input, part, order, kind } => {
                let arr = keyed(self.node(*input), part.clone());
                group(arr, window_logic(order.clone(), kind.clone()), false)
            }
            Op::DistinctPrefix { input, prefix } => {
                let p = *prefix;
                let key: Key = (0..p).map(|c| (super::norm::col(c), KeyMode::Group)).collect();
                let arr = keyed(self.node(*input), key);
                group(arr, distinct_prefix_logic(p), false)
            }
            Op::Fix { var, init, step } => self.fix(*var, init.clone(), step.clone()),
        };
        self.done[n] = Some(c.clone());
        c
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
        let init_d: C<'s, T> = {
            let mut it = init.iter();
            let first = self.node(*it.next().unwrap());
            it.fold(first, |a, i| a.concat(self.node(*i)))
        };
        // Touched views: tags with a nonzero delta record in any input of the fixpoint.
        let touched = bnd_cols
            .iter()
            .map(|(_, c)| c.clone())
            .fold(init_d.clone(), |a, c| a.concat(c))
            .filter(|(w, _)| *w != BASE)
            .threshold(|_, c| if *c != 0 { 1 } else { 0 })
            .map(|(w, _)| w)
            .threshold(|_, c| if *c > 0 { 1 } else { 0 });
        // View seeds: the base view reads base records; a touched view v reads its chain.
        let base_seed = init_d.clone().filter(|(w, _)| *w == BASE);
        let chain_pairs = touched.clone().flat_map(|v| tags::chain(v).into_iter().map(move |t| (vec![Val::Int(t as i64)], (BASE, Val::Int(v as i64)))));
        let by_tag = init_d.clone().map(|(w, row)| (vec![Val::Int(w as i64)], (BASE, row))).arrange_by_key();
        let view_seed = chain_pairs
            .map(|(k, (b, v))| (k, (b, vec![v])))
            .arrange_by_key()
            .join_core(by_tag, |_t, (_, v), (_, row)| match v[0] {
                Val::Int(v) => Some((v as W, row.clone())),
                _ => None,
            });
        let seeds = base_seed.concat(view_seed);
        // Arrangements of the boundary relations by the keys the body joins them on are built
        // inside the iteration from the entered collections.
        let g2 = g.clone();
        let result = seeds.clone().iterate(|inner, rec| {
            let mut env: FxHashMap<NodeId, C<'_, IT>> = FxHashMap::default();
            for (b, c) in &bnd_cols {
                env.insert(*b, c.clone().enter(inner));
            }
            let mut vl = ViewLower { g: g2.clone(), var, rec: rec.clone(), env, done: FxHashMap::default() };
            let mut out = seeds.clone().enter(inner);
            for &s in &step {
                let mut d = vl.node(s);
                if crate::mutant::on(6) {
                    d = TFilter::filter(d.inner, |(_, t, _): &((W, Row), IT, R)| t.inner < 2).as_collection();
                }
                out = out.concat(d);
            }
            out.threshold(|_, c| if *c > 0 { 1 } else { 0 })
        });
        // Back to delta mode: base view under 0; a touched view v: R_v − R_pv, pv its parent view
        // if touched, else the base view.
        let par = touched.clone().map(|v| (vec![Val::Int(tags::parent(v) as i64)], (BASE, vec![Val::Int(v as i64)])));
        let tset = touched.clone().map(|v| (vec![Val::Int(v as i64)], (BASE, ()))).arrange_by_key();
        let par_arr = par.clone().arrange_by_key();
        let with_parent = par_arr.join_core(tset, |p, (_, v), _| Some((p.clone(), v.clone())));
        let without = par.concat(with_parent.clone().map(|(p, v)| (p, (BASE, v))).negate()).map(|(_, (_, v))| (vec![Val::Int(0)], v));
        let pv_pairs = with_parent.concat(without).map(|(pv, v)| (pv, (BASE, v))).arrange_by_key();
        let res_by_view = result.clone().map(|(v, row)| (vec![Val::Int(v as i64)], (BASE, row))).arrange_by_key();
        let minus = res_by_view.join_core(pv_pairs, |_pv, (_, row), (_, v)| match v[0] {
            Val::Int(v) => Some((v as W, row.clone())),
            _ => None,
        });
        result.concat(minus.negate())
    }
}

/// View-mode lowering inside a fixpoint: records `(view, row)` are full rows of a view.
struct ViewLower<'i> {
    g: Rc<Graph>,
    var: u32,
    rec: C<'i, IT>,
    /// Delta-mode boundary collections, entered.
    env: FxHashMap<NodeId, C<'i, IT>>,
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
            Op::Concat { inputs } => {
                let mut it = inputs.clone().into_iter();
                let first = self.node(it.next().unwrap());
                it.fold(first, |a, i| a.concat(self.node(i)))
            }
            Op::Distinct { input } => self.node(*input).threshold(|_, c| if *c > 0 { 1 } else { 0 }),
            Op::Join { l, r, lk, rk } => {
                let (lv, rv) = (self.is_view(*l), self.is_view(*r));
                let la = keyed(self.node(*l), lk.clone());
                let ra = keyed(self.node(*r), rk.clone());
                // view×view: same view; view×delta: the delta record must be visible in the view.
                la.join_core(ra, move |_k, (w1, a), (w2, b)| {
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
                })
            }
            Op::Semi { l, r, lk, rk, anti } => {
                let lv = self.is_view(*l);
                let rv = self.is_view(*r);
                if !lv && *anti {
                    refuse("antijoin of a non-recursive relation by a recursive one inside the recursion (not stratified)");
                    return self.rec.clone().filter(|_| false);
                }
                let left = self.node(*l);
                let la = keyed(left.clone(), lk.clone());
                let rk = rk.clone();
                let keys = self.node(*r).flat_map(move |(w, row)| {
                    let mut vr = None;
                    key_of(&row, &rk, &mut vr).map(|k| (k, (w, ())))
                });
                // A view right side: presence per (key, view). A delta right side: count-based
                // presence deltas per tag; summed over the tags visible in a view they give the
                // view's presence (0 or 1).
                let pres = if rv { keys.threshold(|_, c| if *c > 0 { 1 } else { 0 }).arrange_by_key() } else { presence(keys) };
                let hit = la.join_core(pres, move |_k, (w1, a), (w2, ())| {
                    let w = match (lv, rv) {
                        (true, true) => (w1 == w2).then_some(*w1),
                        (true, false) => tags::visible(*w2, *w1).then_some(*w1),
                        (false, true) => tags::visible(*w1, *w2).then_some(*w2),
                        (false, false) => unreachable!(),
                    }?;
                    Some((w, a.clone()))
                });
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

/// Build one dataflow evaluating every graph's outputs over the tagged `files` input.
pub fn build(worker: &mut timely::worker::Worker, graphs: Vec<(String, Rc<Graph>)>, src: Source, probe: &Handle<T>) -> Built {
    worker.dataflow::<T, _, _>(|scope| {
        let (files_in, files) = scope.new_collection::<(W, (Sym, Cid)), R>();
        let mut outputs = Vec::new();
        for (name, g) in graphs {
            let n = g.nodes.len();
            let mut lw = Lower { scope, g: g.clone(), files: files.clone(), src: src.clone(), done: vec![None; n] };
            for (clause, node, _) in &g.outputs {
                let c = lw.node(*node);
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
                outputs.push((format!("{name}:{clause}"), out));
            }
        }
        Built { files: files_in, outputs }
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
