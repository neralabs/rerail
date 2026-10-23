//! Normalisation: a repository plan in the IR (`ir::Plan<RepoSource>`) becomes a DAG of delta
//! operators the lowering knows the base + delta form of. Every IR construct either maps to one of
//! the operators below, with its semantics preserved, or is refused with the construct named.
//!
//! | IR | delta operator |
//! |---|---|
//! | `Scan(Exported)` | `Exported`: tagged rows from the member contents |
//! | constant subtree (no scan, no binding) | `Const`: evaluated once, base tag |
//! | `Project`, `Filter` (subquery-free) | `Map`, `Filter`: per record, tag kept |
//! | `Join` inner, equality conjuncts of `on` | `Join` (bilinear) + residual `Filter` |
//! | `Join` left | inner `Join` ∪ NULL-extended `Semi(anti)` of the left side by full row |
//! | `[NOT] EXISTS`, `[NOT] IN (subquery)` under a conjunction, equality-correlated | `Semi` (count-based presence) |
//! | scalar subquery over an aggregate, equality-correlated | `Group` on the correlation keys + left join, empty-group value |
//! | scalar subquery `ORDER BY … LIMIT 1`, equality-correlated | `Window` row_number on the correlation keys + left join |
//! | `Aggregate` | `Group` (group recompute per touched key); bare non-key columns refused |
//! | `Window` | `Window` (partition recompute; a tie between distinct rows is a runtime refusal) |
//! | `Distinct` full row / set `Union` | `Distinct` (count-based presence) |
//! | `Distinct` on a prefix | `DistinctPrefix` (a group whose trailing columns disagree is a runtime refusal) |
//! | `Sort` without limit | identity (order is observed only through the above, which check totality) |
//! | `Sort` with limit/offset | `Window` over the whole relation + range filter |
//! | `With` plain | shared node |
//! | `With` fixpoint, `UNION` | `Fix` (semi-naive, per touched view) |
//! | `With` fixpoint, `UNION ALL` | refused: bag recursion |
//! | reachability stage, directed | `Fix` over the edge relation, per path |
use super::val::{self, Row};
use rerail_facts::ir::{self, AggCall, Aff, Binding, CmpOp, Expr, Rel, RepoSource, Sink, StageDef, WindowKind};
use rustc_hash::FxHashMap;
use rerail_facts::sql::value::V;

pub type X = Expr<RepoSource>;
pub type NodeId = usize;

/// How a key column compares: SQL `=` (NULL matches nothing; comparison affinity applied) or
/// grouping identity (NULLs equal; DISTINCT, GROUP BY, the left join's row identity).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyMode {
    Eq(Aff),
    Group,
}

pub type Key = Vec<(X, KeyMode)>;

#[derive(Debug)]
pub enum Op {
    Exported(u32),
    Const(Vec<Row>),
    /// The fixpoint variable (the current level of `Fix` `var`).
    Var(u32),
    Map { input: NodeId, exprs: Vec<X> },
    Filter { input: NodeId, pred: X },
    /// Output: left ++ right.
    Join { l: NodeId, r: NodeId, lk: Key, rk: Key },
    /// Left rows with (or, `anti`, without) a right row on the keys. Output: left.
    Semi { l: NodeId, r: NodeId, lk: Key, rk: Key, anti: bool },
    Distinct { input: NodeId },
    Concat { inputs: Vec<NodeId> },
    /// GROUP BY: output = representative row (key columns only; other columns NULL and never
    /// read: checked) ++ aggregates. `keyless`: one group even over no rows. `order`: the order
    /// ordered aggregates fold a group's rows in (a tie between distinct rows is refused at run time).
    Group { input: NodeId, keys: Key, aggs: Vec<AggCall<RepoSource>>, keyless: bool, width: usize, order: Vec<(X, bool)> },
    /// One window function over partitions `part`, ordered by `order` (a total order is checked
    /// per partition at run time): output = input ++ the function's column.
    Window { input: NodeId, part: Key, order: Vec<(X, bool)>, kind: WindowKind<RepoSource> },
    DistinctPrefix { input: NodeId, prefix: usize },
    /// Storage affinity of a stage's columns (CTAS), per record.
    Affinity { input: NodeId, affs: Vec<Aff> },
    /// Set fixpoint: result = distinct(init ∪ step(result)).
    Fix { var: u32, init: Vec<NodeId>, step: Vec<NodeId> },
}

#[derive(Default, Debug)]
pub struct Graph {
    pub nodes: Vec<Op>,
    pub widths: Vec<usize>,
    /// Columns of a node's output that are not determined by the plan (bare aggregate columns);
    /// reading one is refused.
    pub poison: Vec<Vec<usize>>,
    /// Final relation of each sink: (clause, node, column names).
    pub outputs: Vec<(String, NodeId, Vec<String>)>,
    /// The stage each node was first built for (`clause/stage`): profiles and EXPLAIN.
    pub labels: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Refused {
    pub at: String,
    pub construct: String,
}

type R<T> = Result<T, Refused>;

fn refuse<T>(at: &str, construct: impl Into<String>) -> R<T> {
    Err(Refused { at: at.to_owned(), construct: construct.into() })
}

#[derive(Clone)]
enum Local {
    Node(NodeId),
    Var(u32, usize),
}

struct Ctx<'a> {
    g: Graph,
    stages: Vec<NodeId>,
    exported: &'a [ir::ExportedRelation],
    locals: Vec<(u32, Local)>,
    at: String,
    /// Hash-consing: operator signature → node.
    seen: FxHashMap<String, NodeId>,
}

pub fn col(c: usize) -> X {
    Expr::Col { depth: 0, col: c as u16 }
}

// --------------------------------------------------------------------------------------------
// Expression analysis

/// Does `e` read a row `k` or more subquery levels out (relative to the expression's operator)?
fn expr_free(e: &X, k: u16) -> bool {
    expr_any(e, k, &|_, _| true)
}

/// Does a relation's expressions read a row `k` or more levels out (k = 1: the row enclosing the
/// subquery the relation is)?
fn rel_free(r: &Rel<RepoSource>, k: u16) -> bool {
    rel_any(r, k, &|_, _| true)
}

/// Does `e` read a column `col` of a row `k + up` levels out with `p(up, col)`?
fn expr_any(e: &X, k: u16, p: &dyn Fn(u16, u16) -> bool) -> bool {
    let f = |x: &X| expr_any(x, k, p);
    match e {
        Expr::Lit(_) => false,
        Expr::Col { depth, col } => *depth >= k && p(*depth - k, *col),
        Expr::Not(x) | Expr::Neg(x) | Expr::Plus(x) | Expr::Cast(x, _) => f(x),
        Expr::IsNull { e, .. } => f(e),
        Expr::Arith(_, a, b) | Expr::Concat(a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Cmp(_, a, b, _) => f(a) || f(b),
        Expr::Is { left, right, .. } => f(left) || f(right),
        Expr::Like { text, pattern, .. } => f(text) || f(pattern),
        Expr::Func(_, a) => a.iter().any(f),
        Expr::Case { operand, whens, else_ } => operand.as_ref().is_some_and(|o| f(o)) || whens.iter().any(|(w, t, _)| f(w) || f(t)) || else_.as_ref().is_some_and(|x| f(x)),
        Expr::InList { e, list, .. } => f(e) || list.iter().any(|(x, _)| f(x)),
        Expr::InRel { e, rel, .. } => f(e) || rel_any(rel, k + 1, p),
        Expr::Exists { rel, .. } => rel_any(rel, k + 1, p),
        Expr::Scalar(rel) => rel_any(rel, k + 1, p),
    }
}

fn rel_any(r: &Rel<RepoSource>, k: u16, p: &dyn Fn(u16, u16) -> bool) -> bool {
    let e = |x: &X| expr_any(x, k, p);
    let r_ = |x: &Rel<RepoSource>| rel_any(x, k, p);
    match r {
        Rel::Scan { .. } | Rel::Local { .. } | Rel::Unit => false,
        Rel::JsonEach { input, arg, on, .. } => r_(input) || e(arg) || on.as_ref().is_some_and(e),
        Rel::Join { left, right, on, .. } => r_(left) || r_(right) || on.as_ref().is_some_and(e),
        Rel::Filter { input, pred } => r_(input) || e(pred),
        Rel::Project { input, exprs, .. } => r_(input) || exprs.iter().any(e),
        Rel::Aggregate { input, keys, aggs } => r_(input) || keys.iter().any(e) || aggs.iter().any(|a| a.arg.as_ref().is_some_and(e) || a.sep.as_ref().is_some_and(e)),
        Rel::Window { input, funcs } => r_(input) || funcs.iter().any(|f| f.partition.iter().any(e) || f.order.iter().any(|(x, _)| e(x)) || matches!(&f.kind, WindowKind::Min(x) | WindowKind::Max(x) if e(x))),
        Rel::Distinct { input, .. } => r_(input),
        Rel::Union { left, right, .. } => r_(left) || r_(right),
        Rel::Sort { input, limit, offset, .. } => r_(input) || limit.as_ref().is_some_and(e) || offset.as_ref().is_some_and(e),
        Rel::With { bindings, body } => {
            r_(body)
                || bindings.iter().any(|(_, b)| match b {
                    Binding::Plain(x) => r_(x),
                    Binding::Fix { init, step, .. } => init.iter().chain(step).any(r_),
                })
        }
    }
}

fn has_subquery(e: &X) -> bool {
    match e {
        Expr::InRel { .. } | Expr::Exists { .. } | Expr::Scalar(_) => true,
        Expr::Lit(_) | Expr::Col { .. } => false,
        Expr::Not(x) | Expr::Neg(x) | Expr::Plus(x) | Expr::Cast(x, _) => has_subquery(x),
        Expr::IsNull { e, .. } => has_subquery(e),
        Expr::Arith(_, a, b) | Expr::Concat(a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Cmp(_, a, b, _) => has_subquery(a) || has_subquery(b),
        Expr::Is { left, right, .. } => has_subquery(left) || has_subquery(right),
        Expr::Like { text, pattern, .. } => has_subquery(text) || has_subquery(pattern),
        Expr::Func(_, a) => a.iter().any(has_subquery),
        Expr::Case { operand, whens, else_ } => operand.as_ref().is_some_and(|o| has_subquery(o)) || whens.iter().any(|(w, t, _)| has_subquery(w) || has_subquery(t)) || else_.as_ref().is_some_and(|x| has_subquery(x)),
        Expr::InList { e, list, .. } => has_subquery(e) || list.iter().any(|(x, _)| has_subquery(x)),
    }
}

/// Columns of the depth-0 row an expression reads.
fn cols_read(e: &X, out: &mut Vec<usize>) {
    fn go(e: &X, d: u16, out: &mut Vec<usize>) {
        match e {
            Expr::Lit(_) => {}
            Expr::Col { depth, col } => {
                if *depth == d {
                    out.push(*col as usize)
                }
            }
            Expr::Not(x) | Expr::Neg(x) | Expr::Plus(x) | Expr::Cast(x, _) => go(x, d, out),
            Expr::IsNull { e, .. } => go(e, d, out),
            Expr::Arith(_, a, b) | Expr::Concat(a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Cmp(_, a, b, _) => {
                go(a, d, out);
                go(b, d, out)
            }
            Expr::Is { left, right, .. } => {
                go(left, d, out);
                go(right, d, out)
            }
            Expr::Like { text, pattern, .. } => {
                go(text, d, out);
                go(pattern, d, out)
            }
            Expr::Func(_, a) => a.iter().for_each(|x| go(x, d, out)),
            Expr::Case { operand, whens, else_ } => {
                if let Some(o) = operand {
                    go(o, d, out)
                }
                for (w, t, _) in whens {
                    go(w, d, out);
                    go(t, d, out)
                }
                if let Some(x) = else_ {
                    go(x, d, out)
                }
            }
            Expr::InList { e, list, .. } => {
                go(e, d, out);
                list.iter().for_each(|(x, _)| go(x, d, out))
            }
            // Subqueries are decorrelated before this is asked.
            Expr::InRel { e, .. } => go(e, d, out),
            Expr::Exists { .. } | Expr::Scalar(_) => {}
        }
    }
    go(e, 0, out)
}

/// Rewrite column references: `f(depth, col) -> Some(new expr)` replaces a column; subquery
/// bodies are left alone (decorrelation removes them first).
fn map_cols(e: &X, f: &dyn Fn(u16, u16) -> Option<X>) -> X {
    let b = |x: &X| Box::new(map_cols(x, f));
    match e {
        Expr::Col { depth, col } => f(*depth, *col).unwrap_or_else(|| e.clone()),
        Expr::Lit(_) | Expr::InRel { .. } | Expr::Exists { .. } | Expr::Scalar(_) => e.clone(),
        Expr::Not(x) => Expr::Not(b(x)),
        Expr::Neg(x) => Expr::Neg(b(x)),
        Expr::Plus(x) => Expr::Plus(b(x)),
        Expr::Cast(x, a) => Expr::Cast(b(x), *a),
        Expr::IsNull { e, negated } => Expr::IsNull { e: b(e), negated: *negated },
        Expr::Arith(o, x, y) => Expr::Arith(*o, b(x), b(y)),
        Expr::Concat(x, y) => Expr::Concat(b(x), b(y)),
        Expr::And(x, y) => Expr::And(b(x), b(y)),
        Expr::Or(x, y) => Expr::Or(b(x), b(y)),
        Expr::Cmp(o, x, y, a) => Expr::Cmp(*o, b(x), b(y), *a),
        Expr::Is { left, right, negated } => Expr::Is { left: b(left), right: b(right), negated: *negated },
        Expr::Like { text, pattern, negated } => Expr::Like { text: b(text), pattern: b(pattern), negated: *negated },
        Expr::Func(fu, a) => Expr::Func(*fu, a.iter().map(|x| map_cols(x, f)).collect()),
        Expr::Case { operand, whens, else_ } => Expr::Case { operand: operand.as_ref().map(|o| b(o)), whens: whens.iter().map(|(w, t, a)| (map_cols(w, f), map_cols(t, f), *a)).collect(), else_: else_.as_ref().map(|x| b(x)) },
        Expr::InList { e, list, negated } => Expr::InList { e: b(e), list: list.iter().map(|(x, a)| (map_cols(x, f), *a)).collect(), negated: *negated },
    }
}

fn conjuncts(e: &X, out: &mut Vec<X>) {
    match e {
        Expr::And(a, b) => {
            conjuncts(a, out);
            conjuncts(b, out)
        }
        _ => out.push(e.clone()),
    }
}

fn and_all(mut v: Vec<X>) -> Option<X> {
    let first = if v.is_empty() { return None } else { v.remove(0) };
    Some(v.into_iter().fold(first, |a, b| Expr::And(Box::new(a), Box::new(b))))
}

fn only_cols_below(e: &X, lo: usize, hi: usize) -> bool {
    let mut c = Vec::new();
    cols_read(e, &mut c);
    !expr_free(e, 1) && c.iter().all(|&x| x >= lo && x < hi)
}

fn shift(e: &X, by: isize) -> X {
    map_cols(e, &|d, c| (d == 0).then(|| Expr::Col { depth: 0, col: (c as isize + by) as u16 }))
}

/// Does `e` call `rerail_refuse` (a refusal raised by evaluation: never moved)?
fn raises(e: &X) -> bool {
    let mut found = false;
    rerail_facts::front::sql::visit_expr(e, &mut |x| found |= matches!(x, Expr::Func(ir::Func::Refuse, _)));
    found
}

/// R15 (REWRITES.md): a conjunct reading one side of a join only is evaluated on that side.
/// - `WHERE` over a join: a left-only conjunct filters the left input (inner or LEFT join); a
///   right-only one the right input (inner joins only: under a LEFT join it would keep the NULL-
///   extended rows its WHERE drops).
/// - `ON`: a left- or right-only conjunct filters that input (inner joins); under a LEFT join only
///   a right-only one (it restricts the matches; a left-only one decides NULL-extension instead).
///
/// Side conditions: the conjunct is subquery-free, reads no outer row (a correlation stays where
/// decorrelation looks for it) and raises no named refusal. Returns `None` when nothing moves.
/// Exact: a conjunction over a join's rows is the same filter before or after the join for the
/// conjuncts determined by one side's columns; inner joins commute with filters on either input,
/// a LEFT join with filters on its preserved side and with `ON` restrictions of its right side.
pub fn push_filters(r: &Rel<RepoSource>) -> Option<Rel<RepoSource>> {
    let (join, pred) = match r {
        Rel::Filter { input, pred } if matches!(**input, Rel::Join { .. }) => (&**input, Some(pred)),
        Rel::Join { .. } => (r, None),
        _ => return None,
    };
    let Rel::Join { kind, left, right, on } = join else { unreachable!() };
    let inner = *kind == ir::JoinKind::Inner;
    let (wl, wr) = (left.schema().len(), right.schema().len());
    let movable = |c: &X| !has_subquery(c) && !raises(c);
    let (mut to_l, mut to_r) = (Vec::new(), Vec::new());
    let split = |cs: Vec<X>, to_l: &mut Vec<X>, to_r: &mut Vec<X>, left_ok: bool, right_ok: bool| -> Vec<X> {
        let mut keep = Vec::new();
        for c in cs {
            if movable(&c) && left_ok && only_cols_below(&c, 0, wl) {
                to_l.push(c);
            } else if movable(&c) && right_ok && only_cols_below(&c, wl, wl + wr) {
                to_r.push(shift(&c, -(wl as isize)));
            } else {
                keep.push(c);
            }
        }
        keep
    };
    let mut on_cs = Vec::new();
    if let Some(o) = on {
        conjuncts(o, &mut on_cs);
    }
    let on_keep = split(on_cs, &mut to_l, &mut to_r, inner, true);
    let mut where_keep = Vec::new();
    if let Some(p) = pred {
        let mut cs = Vec::new();
        conjuncts(p, &mut cs);
        // Mutant 24: a right-only WHERE conjunct moves below a LEFT join too.
        where_keep = split(cs, &mut to_l, &mut to_r, true, inner || crate::mutant::on(24));
    }
    if to_l.is_empty() && to_r.is_empty() {
        return None;
    }
    let wrap = |rel: &Rel<RepoSource>, cs: Vec<X>| match and_all(cs) {
        None => rel.clone(),
        Some(pred) => Rel::Filter { input: Box::new(rel.clone()), pred },
    };
    let j = Rel::Join { kind: *kind, left: Box::new(wrap(left, to_l)), right: Box::new(wrap(right, to_r)), on: and_all(on_keep) };
    Some(match and_all(where_keep) {
        None => j,
        Some(pred) => Rel::Filter { input: Box::new(j), pred },
    })
}

// --------------------------------------------------------------------------------------------
// Decorrelation rewrites (REWRITES.md, facts-b table, R20–R32): IR to IR, applied while
// normalising. Each keeps SQLite's semantics (bag, three-valued logic); the tests below compare the
// reference evaluator's multisets before and after on random tables.

type Rl = Rel<RepoSource>;

/// `e` (an expression of some operator) with every column reference to a row `r` levels out of
/// that operator's row (`r` = 0: the row itself), subqueries included, replaced by
/// `f(r, col) = (r', col')`. `lv`: the subquery depth `e` sits at below the operator.
fn remap_expr(e: &X, lv: u16, f: &dyn Fn(u16, u16) -> (u16, u16)) -> X {
    let b = |x: &X| Box::new(remap_expr(x, lv, f));
    match e {
        Expr::Col { depth, col } if *depth >= lv => {
            let (r, c) = f(*depth - lv, *col);
            Expr::Col { depth: lv + r, col: c }
        }
        Expr::Col { .. } | Expr::Lit(_) => e.clone(),
        Expr::Not(x) => Expr::Not(b(x)),
        Expr::Neg(x) => Expr::Neg(b(x)),
        Expr::Plus(x) => Expr::Plus(b(x)),
        Expr::Cast(x, a) => Expr::Cast(b(x), *a),
        Expr::IsNull { e, negated } => Expr::IsNull { e: b(e), negated: *negated },
        Expr::Arith(o, x, y) => Expr::Arith(*o, b(x), b(y)),
        Expr::Concat(x, y) => Expr::Concat(b(x), b(y)),
        Expr::And(x, y) => Expr::And(b(x), b(y)),
        Expr::Or(x, y) => Expr::Or(b(x), b(y)),
        Expr::Cmp(o, x, y, a) => Expr::Cmp(*o, b(x), b(y), *a),
        Expr::Is { left, right, negated } => Expr::Is { left: b(left), right: b(right), negated: *negated },
        Expr::Like { text, pattern, negated } => Expr::Like { text: b(text), pattern: b(pattern), negated: *negated },
        Expr::Func(fu, a) => Expr::Func(*fu, a.iter().map(|x| remap_expr(x, lv, f)).collect()),
        Expr::Case { operand, whens, else_ } => Expr::Case { operand: operand.as_ref().map(|o| b(o)), whens: whens.iter().map(|(w, t, a)| (remap_expr(w, lv, f), remap_expr(t, lv, f), *a)).collect(), else_: else_.as_ref().map(|x| b(x)) },
        Expr::InList { e, list, negated } => Expr::InList { e: b(e), list: list.iter().map(|(x, a)| (remap_expr(x, lv, f), *a)).collect(), negated: *negated },
        Expr::InRel { e, rel, negated, aff } => Expr::InRel { e: b(e), rel: Box::new(remap_rel(rel, lv + 1, f)), negated: *negated, aff: *aff },
        Expr::Exists { rel, negated } => Expr::Exists { rel: Box::new(remap_rel(rel, lv + 1, f)), negated: *negated },
        Expr::Scalar(rel) => Expr::Scalar(Box::new(remap_rel(rel, lv + 1, f))),
    }
}

/// Every expression of `r` (a relation at subquery depth `lv` below the operator) through `remap_expr`.
fn remap_rel(r: &Rl, lv: u16, f: &dyn Fn(u16, u16) -> (u16, u16)) -> Rl {
    let e = |x: &X| remap_expr(x, lv, f);
    let rb = |x: &Rl| Box::new(remap_rel(x, lv, f));
    match r {
        Rel::Scan { .. } | Rel::Local { .. } | Rel::Unit => r.clone(),
        Rel::JsonEach { input, arg, on, outer } => Rel::JsonEach { input: rb(input), arg: e(arg), on: on.as_ref().map(e), outer: *outer },
        Rel::Join { kind, left, right, on } => Rel::Join { kind: *kind, left: rb(left), right: rb(right), on: on.as_ref().map(e) },
        Rel::Filter { input, pred } => Rel::Filter { input: rb(input), pred: e(pred) },
        Rel::Project { input, exprs, schema } => Rel::Project { input: rb(input), exprs: exprs.iter().map(e).collect(), schema: schema.clone() },
        Rel::Aggregate { input, keys, aggs } => Rel::Aggregate {
            input: rb(input),
            keys: keys.iter().map(e).collect(),
            aggs: aggs.iter().map(|a| AggCall { f: a.f, arg: a.arg.as_ref().map(e), sep: a.sep.as_ref().map(e), distinct: a.distinct, total: a.total }).collect(),
        },
        Rel::Window { input, funcs } => Rel::Window {
            input: rb(input),
            funcs: funcs
                .iter()
                .map(|w| ir::WindowFn {
                    kind: match &w.kind {
                        WindowKind::RowNumber => WindowKind::RowNumber,
                        WindowKind::Min(x) => WindowKind::Min(e(x)),
                        WindowKind::Max(x) => WindowKind::Max(e(x)),
                    },
                    partition: w.partition.iter().map(e).collect(),
                    order: w.order.iter().map(|(x, d)| (e(x), *d)).collect(),
                    total: w.total,
                })
                .collect(),
        },
        Rel::Distinct { input, prefix } => Rel::Distinct { input: rb(input), prefix: *prefix },
        Rel::Union { all, left, right } => Rel::Union { all: *all, left: rb(left), right: rb(right) },
        Rel::Sort { input, keys, limit, offset, total } => Rel::Sort { input: rb(input), keys: keys.clone(), limit: limit.as_ref().map(e), offset: offset.as_ref().map(e), total: *total },
        Rel::With { bindings, body } => Rel::With {
            bindings: bindings
                .iter()
                .map(|(id, b)| {
                    (
                        *id,
                        match b {
                            Binding::Plain(x) => Binding::Plain(remap_rel(x, lv, f)),
                            Binding::Fix { init, step, set, schema } => Binding::Fix { init: init.iter().map(|x| remap_rel(x, lv, f)).collect(), step: step.iter().map(|x| remap_rel(x, lv, f)).collect(), set: *set, schema: schema.clone() },
                        },
                    )
                })
                .collect(),
            body: rb(body),
        },
    }
}

/// `e` with the operator's own columns `c` (row 0) moved to `f(c)`; outer rows unchanged.
fn remap0(e: &X, f: &dyn Fn(u16) -> u16) -> X {
    remap_expr(e, 0, &|r, c| if r == 0 { (0, f(c)) } else { (r, c) })
}

fn same(a: &X, b: &X) -> bool {
    format!("{a:?}") == format!("{b:?}")
}

/// The subqueries (`EXISTS`, scalar) of `e` outside any subquery, in order.
fn top_subqueries(e: &X, out: &mut Vec<X>) {
    match e {
        Expr::Exists { .. } | Expr::Scalar(_) => {
            if !out.iter().any(|x| same(x, e)) {
                out.push(e.clone())
            }
        }
        Expr::Lit(_) | Expr::Col { .. } => {}
        Expr::Not(x) | Expr::Neg(x) | Expr::Plus(x) | Expr::Cast(x, _) => top_subqueries(x, out),
        Expr::IsNull { e, .. } | Expr::InRel { e, .. } => top_subqueries(e, out),
        Expr::Arith(_, a, b) | Expr::Concat(a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Cmp(_, a, b, _) => {
            top_subqueries(a, out);
            top_subqueries(b, out)
        }
        Expr::Is { left, right, .. } => {
            top_subqueries(left, out);
            top_subqueries(right, out)
        }
        Expr::Like { text, pattern, .. } => {
            top_subqueries(text, out);
            top_subqueries(pattern, out)
        }
        Expr::Func(_, a) => a.iter().for_each(|x| top_subqueries(x, out)),
        Expr::Case { operand, whens, else_ } => {
            if let Some(o) = operand {
                top_subqueries(o, out)
            }
            for (w, t, _) in whens {
                top_subqueries(w, out);
                top_subqueries(t, out)
            }
            if let Some(x) = else_ {
                top_subqueries(x, out)
            }
        }
        Expr::InList { e, list, .. } => {
            top_subqueries(e, out);
            list.iter().for_each(|(x, _)| top_subqueries(x, out))
        }
    }
}

/// `e` with each occurrence of `pairs[i].0` outside subqueries replaced by `pairs[i].1`.
fn subst(e: &X, pairs: &[(X, X)]) -> X {
    if let Some((_, r)) = pairs.iter().find(|(p, _)| same(p, e)) {
        return r.clone();
    }
    let b = |x: &X| Box::new(subst(x, pairs));
    match e {
        Expr::Lit(_) | Expr::Col { .. } | Expr::Exists { .. } | Expr::Scalar(_) => e.clone(),
        Expr::Not(x) => Expr::Not(b(x)),
        Expr::Neg(x) => Expr::Neg(b(x)),
        Expr::Plus(x) => Expr::Plus(b(x)),
        Expr::Cast(x, a) => Expr::Cast(b(x), *a),
        Expr::IsNull { e, negated } => Expr::IsNull { e: b(e), negated: *negated },
        Expr::Arith(o, x, y) => Expr::Arith(*o, b(x), b(y)),
        Expr::Concat(x, y) => Expr::Concat(b(x), b(y)),
        Expr::And(x, y) => Expr::And(b(x), b(y)),
        Expr::Or(x, y) => Expr::Or(b(x), b(y)),
        Expr::Cmp(o, x, y, a) => Expr::Cmp(*o, b(x), b(y), *a),
        Expr::Is { left, right, negated } => Expr::Is { left: b(left), right: b(right), negated: *negated },
        Expr::Like { text, pattern, negated } => Expr::Like { text: b(text), pattern: b(pattern), negated: *negated },
        Expr::Func(fu, a) => Expr::Func(*fu, a.iter().map(|x| subst(x, pairs)).collect()),
        Expr::Case { operand, whens, else_ } => Expr::Case { operand: operand.as_ref().map(|o| b(o)), whens: whens.iter().map(|(w, t, a)| (subst(w, pairs), subst(t, pairs), *a)).collect(), else_: else_.as_ref().map(|x| b(x)) },
        Expr::InList { e, list, negated } => Expr::InList { e: b(e), list: list.iter().map(|(x, a)| (subst(x, pairs), *a)).collect(), negated: *negated },
        Expr::InRel { e, rel, negated, aff } => Expr::InRel { e: b(e), rel: rel.clone(), negated: *negated, aff: *aff },
    }
}

fn disjuncts(e: &X, out: &mut Vec<X>) {
    match e {
        Expr::Or(a, b) => {
            disjuncts(a, out);
            disjuncts(b, out)
        }
        _ => out.push(e.clone()),
    }
}

/// A subquery body under its projection and DISTINCT (existence does not read either).
fn peel(r: &Rl) -> &Rl {
    match r {
        Rel::Project { input, .. } | Rel::Distinct { input, .. } => peel(input),
        other => other,
    }
}

fn count_star() -> AggCall<RepoSource> {
    AggCall { f: ir::AggFn::CountStar, arg: None, sep: None, distinct: false, total: ir::Total::Proven }
}

/// An aggregate that observes its input's order.
fn ordered_agg(a: &AggCall<RepoSource>) -> bool {
    matches!(a.f, ir::AggFn::GroupConcat | ir::AggFn::JsonGroupArray)
}

/// An EXISTS body ready for decorrelation: correlations lifted to its top `WHERE` (R22), nested
/// EXISTS conjuncts reading the enclosing row joined in (R32).
fn prep_exists(rel: &Rl) -> Rl {
    let mut r = lift_correlations(rel).unwrap_or_else(|| rel.clone());
    while let Some(x) = flatten_exists(&r) {
        r = x;
    }
    r
}

/// `Project(join', left columns ++ right columns)`: drops the `k` columns a rewrite appended to a
/// join's left input.
fn unwiden_join(j: Rl, wl: usize, k: usize, wr: usize, schema: Vec<ir::Column>) -> Rl {
    Rel::Project { input: Box::new(j), exprs: (0..wl).chain(wl + k..wl + k + wr).map(col).collect(), schema }
}

/// R20: the subqueries of a join's `ON` that read the left row and nothing else outside
/// themselves (no right column, no row outside the join) → columns of a `Project` over the left
/// input; the `ON` reads those columns (an equality on one becomes a join key). Exact: the value
/// of such a subquery depends on the left row only, so computing it once per left row and reading
/// it for every pair gives every pair the same value; the projection keeps the left rows' bag.
pub fn hoist_on_subqueries(r: &Rl) -> Option<Rl> {
    let Rel::Join { kind, left, right, on: Some(on) } = r else { return None };
    let (wl, wr) = (left.schema().len(), right.schema().len());
    let mut found = Vec::new();
    top_subqueries(on, &mut found);
    let subs: Vec<X> = found
        .into_iter()
        .filter(|s| expr_any(s, 0, &|up, c| up == 0 && (c as usize) < wl) && !expr_any(s, 0, &|up, c| up == 0 && c as usize >= wl) && !expr_any(s, 0, &|up, _| up >= 1) && !raises(s))
        .collect();
    if subs.is_empty() {
        return None;
    }
    let k = subs.len();
    // Mutant 28: the right columns keep their positions (not shifted past the new left columns).
    let by = if crate::mutant::on(28) { 0 } else { k as u16 };
    let on2 = remap0(on, &|c| if c as usize >= wl { c + by } else { c });
    let on2 = subst(&on2, &subs.iter().enumerate().map(|(i, s)| (s.clone(), col(wl + i))).collect::<Vec<_>>());
    let mut ls = left.schema();
    ls.extend((0..k).map(|i| ir::Column::new(&format!("__on_subquery_{i}"), Aff::None)));
    let l2 = Rel::Project { input: left.clone(), exprs: (0..wl).map(col).chain(subs).collect(), schema: ls };
    Some(unwiden_join(Rel::Join { kind: *kind, left: Box::new(l2), right: right.clone(), on: Some(on2) }, wl, k, wr, r.schema()))
}

/// R28: an uncorrelated `EXISTS (body)` in a join's `ON` → `c > 0` (`NOT EXISTS`: `c = 0`) with
/// `c` = `count(*)` of the body, a one-row relation cross-joined into the left input. Exact: the
/// subquery reads no row, so its value is one constant for every pair; a keyless aggregate has
/// exactly one row, so the cross join keeps the left rows' bag, and EXISTS is `count(*) > 0`.
pub fn count_on_exists(r: &Rl) -> Option<Rl> {
    let Rel::Join { kind, left, right, on: Some(on) } = r else { return None };
    let (wl, wr) = (left.schema().len(), right.schema().len());
    let mut found = Vec::new();
    top_subqueries(on, &mut found);
    let mut bodies: Vec<Rl> = Vec::new();
    for s in &found {
        if let Expr::Exists { rel, .. } = s {
            if !expr_any(s, 0, &|_, _| true) && !raises(s) && !bodies.iter().any(|b| format!("{b:?}") == format!("{rel:?}")) {
                bodies.push((**rel).clone());
            }
        }
    }
    if bodies.is_empty() {
        return None;
    }
    let k = bodies.len();
    let mut l2 = (**left).clone();
    let mut pairs = Vec::new();
    for (i, b) in bodies.iter().enumerate() {
        let wb = b.schema().len();
        let agg = Rel::Aggregate { input: Box::new(b.clone()), keys: vec![], aggs: vec![count_star()] };
        let cnt = Rel::Project { input: Box::new(agg), exprs: vec![col(wb)], schema: vec![ir::Column::new("__exists_count", Aff::None)] };
        l2 = Rel::Join { kind: ir::JoinKind::Inner, left: Box::new(l2), right: Box::new(cnt), on: None };
        let c = || Box::new(col(wl + i));
        // Mutant 36: EXISTS read as count(*) >= 0 (always true).
        let gt = if crate::mutant::on(36) { CmpOp::Ge } else { CmpOp::Gt };
        let zero = || Box::new(Expr::Lit(ir::Lit::Int(0)));
        pairs.push((Expr::Exists { rel: Box::new(b.clone()), negated: false }, Expr::Cmp(gt, c(), zero(), (Aff::None, Aff::None))));
        pairs.push((Expr::Exists { rel: Box::new(b.clone()), negated: true }, Expr::Cmp(CmpOp::Eq, c(), zero(), (Aff::None, Aff::None))));
    }
    let on2 = subst(&remap0(on, &|c| if c as usize >= wl { c + k as u16 } else { c }), &pairs);
    Some(unwiden_join(Rel::Join { kind: *kind, left: Box::new(l2), right: right.clone(), on: Some(on2) }, wl, k, wr, r.schema()))
}

/// R22: inside a subquery body, the correlated conjuncts (reading the enclosing row; subquery-
/// free, no named refusal) of filters and inner-join `ON`s below its top `WHERE`, through inner
/// joins and filters → the top `WHERE` (created if absent), in its row's columns. Exact: an inner
/// join is a filter over the cross product, and conjuncts of filters over a product commute.
pub fn lift_correlations(body: &Rl) -> Option<Rl> {
    match body {
        Rel::Project { input, exprs, schema } => return lift_correlations(input).map(|i| Rel::Project { input: Box::new(i), exprs: exprs.clone(), schema: schema.clone() }),
        Rel::Distinct { input, prefix } => return lift_correlations(input).map(|i| Rel::Distinct { input: Box::new(i), prefix: *prefix }),
        _ => {}
    }
    let (inner, top) = match body {
        Rel::Filter { input, pred } => (&**input, Some(pred)),
        other => (other, None),
    };
    let mut lifted = Vec::new();
    let stripped = strip_correlations(inner, &mut lifted);
    if lifted.is_empty() {
        return None;
    }
    let mut cs = Vec::new();
    if let Some(p) = top {
        conjuncts(p, &mut cs);
    }
    cs.extend(lifted);
    Some(Rel::Filter { input: Box::new(stripped), pred: and_all(cs).unwrap() })
}

/// `r` without its liftable correlated conjuncts, which go to `out` over `r`'s output row.
fn strip_correlations(r: &Rl, out: &mut Vec<X>) -> Rl {
    let liftable = |c: &X| expr_free(c, 1) && !has_subquery(c) && !raises(c);
    match r {
        Rel::Filter { input, pred } => {
            let i = strip_correlations(input, out);
            let mut cs = Vec::new();
            conjuncts(pred, &mut cs);
            let (lift, keep): (Vec<X>, Vec<X>) = cs.into_iter().partition(|c| liftable(c));
            out.extend(lift);
            match and_all(keep) {
                None => i,
                Some(p) => Rel::Filter { input: Box::new(i), pred: p },
            }
        }
        // Mutant 30: LEFT joins are crossed too (their ON decides NULL extension, not membership).
        Rel::Join { kind, left, right, on } if *kind == ir::JoinKind::Inner || crate::mutant::on(30) => {
            let wl = left.schema().len() as u16;
            let l = strip_correlations(left, out);
            let mut ro = Vec::new();
            let rr = strip_correlations(right, &mut ro);
            out.extend(ro.iter().map(|c| remap0(c, &|x| x + wl)));
            let mut cs = Vec::new();
            if let Some(o) = on {
                conjuncts(o, &mut cs);
            }
            let (lift, keep): (Vec<X>, Vec<X>) = cs.into_iter().partition(|c| liftable(c));
            out.extend(lift);
            Rel::Join { kind: *kind, left: Box::new(l), right: Box::new(rr), on: and_all(keep) }
        }
        other => other.clone(),
    }
}

/// R24: `EXISTS (… WHERE r ∧ (d₁ ∨ … ∨ dₙ))` whose disjunction reads the enclosing row →
/// `EXISTS (… WHERE r ∧ d₁) ∨ … ∨ EXISTS (… WHERE r ∧ dₙ)`; `NOT EXISTS` → the conjunction of the
/// `NOT EXISTS`. Exact: a body row qualifies iff its `WHERE` is TRUE, and `r ∧ (d₁ ∨ d₂)` is TRUE
/// iff `r ∧ d₁` or `r ∧ d₂` is (three-valued `OR`/`AND`); EXISTS is two-valued.
pub fn split_exists(e: &X) -> Option<X> {
    let Expr::Exists { rel, negated } = e else { return None };
    let Rel::Filter { input, pred } = peel(rel) else { return None };
    let mut cs = Vec::new();
    conjuncts(pred, &mut cs);
    let i = cs.iter().position(|c| matches!(c, Expr::Or(..)) && expr_free(c, 1))?;
    let or = cs.remove(i);
    let mut ds = Vec::new();
    disjuncts(&or, &mut ds);
    // Mutant 32: the other conjuncts are dropped from the split bodies.
    let rest = if crate::mutant::on(32) { Vec::new() } else { cs };
    let mut arms = ds.into_iter().map(|d| {
        let mut c = rest.clone();
        c.push(d);
        Expr::Exists { rel: Box::new(Rel::Filter { input: input.clone(), pred: and_all(c).unwrap() }), negated: *negated }
    });
    let first = arms.next()?;
    Some(arms.fold(first, |a, b| if *negated { Expr::And(Box::new(a), Box::new(b)) } else { Expr::Or(Box::new(a), Box::new(b)) }))
}

/// R32: in an EXISTS body's `WHERE`, a conjunct `EXISTS (C WHERE q)` that reads the row outside
/// the body (C itself reading nothing outside) → `C` inner-joined into the body and `q` a conjunct
/// of its `WHERE` (C's columns after the body's; the body's row read directly). Exact for
/// existence: ∃b. p(b) ∧ ∃c. q(b, c) ⟺ ∃(b, c). p(b) ∧ q(b, c); only the body's multiplicity
/// changes, which EXISTS does not observe. Not for `NOT EXISTS` conjuncts.
pub fn flatten_exists(body: &Rl) -> Option<Rl> {
    let Rel::Filter { input, pred } = peel(body) else { return None };
    let mut cs = Vec::new();
    conjuncts(pred, &mut cs);
    let mut from = (**input).clone();
    let (mut keep, mut fired) = (Vec::new(), false);
    for c in cs {
        if let Expr::Exists { rel, negated } = &c {
            // Mutant 40: NOT EXISTS conjuncts are flattened too.
            if (!*negated || crate::mutant::on(40)) && expr_any(&c, 0, &|up, _| up >= 1) && !raises(&c) {
                let lifted = lift_correlations(rel).unwrap_or_else(|| (**rel).clone());
                let (ci, q) = match peel(&lifted) {
                    Rel::Filter { input, pred } => ((**input).clone(), Some(pred.clone())),
                    other => (other.clone(), None),
                };
                if !rel_free(&ci, 1) {
                    let w = from.schema().len() as u16;
                    from = Rel::Join { kind: ir::JoinKind::Inner, left: Box::new(from), right: Box::new(ci), on: None };
                    if let Some(q) = q {
                        // q's rows: 0 = C's row (now after the body's columns), 1 = the body's row
                        // (now the joined row), r ≥ 2 = one level less.
                        keep.push(remap_expr(&q, 0, &|r, x| match r {
                            0 => (0, x + w),
                            1 => (0, x),
                            r => (r - 1, x),
                        }));
                    }
                    fired = true;
                    continue;
                }
            }
        }
        keep.push(c);
    }
    if !fired {
        return None;
    }
    Some(match and_all(keep) {
        None => from,
        Some(p) => Rel::Filter { input: Box::new(from), pred: p },
    })
}

/// R26: `[Project] [HAVING] Aggregate` whose input is a derived table (`Project`, `Sort` without
/// LIMIT/OFFSET, uncorrelated `Filter`, any depth) over a correlated `Filter` → the correlated
/// conjuncts as a `Filter` at the aggregate's input, the columns they read carried up through the
/// projections; the aggregate's extra representative columns are skipped by the projection above.
/// Exact: a filter commutes with a projection that keeps the columns it reads and with a stable
/// sort (the kept rows keep their order and their sort keys, so ordered aggregates see the same
/// sequence); the aggregate's keys and arguments read the same columns.
pub fn pull_correlation(r: &Rl) -> Option<Rl> {
    let (proj, rest) = match r {
        Rel::Project { input, exprs, schema } => (Some((exprs, schema)), &**input),
        other => (None, other),
    };
    let (having, agg) = match rest {
        Rel::Filter { input, pred } if matches!(**input, Rel::Aggregate { .. }) => (Some(pred), &**input),
        other => (None, other),
    };
    let Rel::Aggregate { input, keys, aggs } = agg else { return None };
    if !matches!(**input, Rel::Project { .. } | Rel::Sort { .. }) {
        return None;
    }
    let (chain, _, corr) = carry(input)?;
    let w0 = input.schema().len();
    let k = chain.schema().len() - w0;
    let filtered = Rel::Filter { input: Box::new(chain), pred: and_all(corr).unwrap() };
    let bump = |e: &X| remap0(e, &|c| if c as usize >= w0 { c + k as u16 } else { c });
    let mut out = Rel::Aggregate { input: Box::new(filtered), keys: keys.clone(), aggs: aggs.clone() };
    if let Some(h) = having {
        out = Rel::Filter { input: Box::new(out), pred: bump(h) };
    }
    let (exprs, schema) = match proj {
        Some((e, s)) => (e.iter().map(bump).collect(), s.clone()),
        None => ((0..w0).chain(w0 + k..w0 + k + aggs.len()).map(col).collect(), agg.schema()),
    };
    Some(Rel::Project { input: Box::new(out), exprs, schema })
}

/// R26's derived table without its correlated filter: (relation, positions of the carried
/// columns in its output, the correlated conjuncts over its output row).
fn carry(r: &Rl) -> Option<(Rl, Vec<u16>, Vec<X>)> {
    match r {
        Rel::Filter { input, pred } if expr_free(pred, 1) && !rel_free(input, 1) => {
            let mut cs = Vec::new();
            conjuncts(pred, &mut cs);
            let (corr, rest): (Vec<X>, Vec<X>) = cs.into_iter().partition(|c| expr_free(c, 1));
            if corr.iter().any(|c| has_subquery(c) || raises(c)) {
                return None;
            }
            let mut used = Vec::new();
            for c in &corr {
                cols_read(c, &mut used);
            }
            used.sort();
            used.dedup();
            let base = match and_all(rest) {
                None => (**input).clone(),
                Some(p) => Rel::Filter { input: input.clone(), pred: p },
            };
            Some((base, used.into_iter().map(|c| c as u16).collect(), corr))
        }
        Rel::Filter { input, pred } if !expr_free(pred, 1) => {
            let (i, pos, corr) = carry(input)?;
            Some((Rel::Filter { input: Box::new(i), pred: pred.clone() }, pos, corr))
        }
        Rel::Project { input, exprs, schema } => {
            let (i, pos, corr) = carry(input)?;
            let m = exprs.len() as u16;
            let mut s = schema.clone();
            s.extend(pos.iter().map(|c| ir::Column::new(&format!("__carried_{c}"), Aff::None)));
            let at = |c: u16| m + pos.iter().position(|p| *p == c).unwrap() as u16;
            let corr = corr.iter().map(|x| remap0(x, &at)).collect();
            Some((Rel::Project { input: Box::new(i), exprs: exprs.iter().cloned().chain(pos.iter().map(|c| col(*c as usize))).collect(), schema: s }, (m..m + pos.len() as u16).collect(), corr))
        }
        // Mutant 34: a filter is pulled through a Sort with LIMIT/OFFSET too.
        Rel::Sort { input, keys, limit, offset, total } if (limit.is_none() && offset.is_none()) || crate::mutant::on(34) => {
            let (i, pos, corr) = carry(input)?;
            Some((Rel::Sort { input: Box::new(i), keys: keys.clone(), limit: limit.clone(), offset: offset.clone(), total: *total }, pos, corr))
        }
        _ => None,
    }
}

/// R30: the subqueries of a window's partition, order and argument expressions → columns of a
/// `Project` over the window's input, read by the window; a projection drops them after. Exact: a
/// window reads its expressions' values per input row; computing them as columns first gives
/// each row the same values (the projection keeps the input's bag and outer rows).
pub fn hoist_window_subqueries(r: &Rl) -> Option<Rl> {
    let Rel::Window { input, funcs } = r else { return None };
    let w = input.schema().len();
    let mut subs = Vec::new();
    for f in funcs {
        let arg = match &f.kind {
            WindowKind::Min(e) | WindowKind::Max(e) => Some(e),
            WindowKind::RowNumber => None,
        };
        for e in f.partition.iter().chain(f.order.iter().map(|x| &x.0)).chain(arg) {
            top_subqueries(e, &mut subs);
        }
    }
    subs.retain(|s| !raises(s));
    if subs.is_empty() {
        return None;
    }
    let k = subs.len();
    // Mutant 38: every hoisted subquery is read from the input's last column.
    let pairs: Vec<(X, X)> = subs.iter().enumerate().map(|(i, s)| (s.clone(), col(if crate::mutant::on(38) { w - 1 } else { w + i }))).collect();
    let mut s = input.schema();
    s.extend((0..k).map(|i| ir::Column::new(&format!("__window_subquery_{i}"), Aff::None)));
    let i2 = Rel::Project { input: input.clone(), exprs: (0..w).map(col).chain(subs).collect(), schema: s };
    let f2 = funcs
        .iter()
        .map(|f| ir::WindowFn {
            kind: match &f.kind {
                WindowKind::RowNumber => WindowKind::RowNumber,
                WindowKind::Min(e) => WindowKind::Min(subst(e, &pairs)),
                WindowKind::Max(e) => WindowKind::Max(subst(e, &pairs)),
            },
            partition: f.partition.iter().map(|e| subst(e, &pairs)).collect(),
            order: f.order.iter().map(|(e, d)| (subst(e, &pairs), *d)).collect(),
            total: f.total,
        })
        .collect();
    let nf = funcs.len();
    Some(Rel::Project { input: Box::new(Rel::Window { input: Box::new(i2), funcs: f2 }), exprs: (0..w).chain(w + k..w + k + nf).map(col).collect(), schema: r.schema() })
}

/// R34: an inner join with no equality between its sides but an `ON` conjunct `e IN (x₁, …, xₙ)`,
/// `e` reading one side and every `xᵢ` the other → `UNION ALL` over i of the join on
/// `rest ∧ e = xᵢ ∧ (e = x₁) IS NOT 1 ∧ … ∧ (e = xᵢ₋₁) IS NOT 1`: each arm a keyed join instead of
/// one cross join filtered per pair. Exact: `e IN (…)` is TRUE iff some `e = xᵢ` is TRUE (the
/// list's comparison affinities kept per member), and a pair belongs to exactly the arm of its
/// first TRUE member, so the arms partition the qualifying pairs; `IS NOT 1` is two-valued.
pub fn split_in_list_join(r: &Rl) -> Option<Rl> {
    let Rel::Join { kind: ir::JoinKind::Inner, left, right, on: Some(on) } = r else { return None };
    let (wl, wr) = (left.schema().len(), right.schema().len());
    let mut cs = Vec::new();
    conjuncts(on, &mut cs);
    let crosses = |a: &X, b: &X| (only_cols_below(a, 0, wl) && only_cols_below(b, wl, wl + wr)) || (only_cols_below(a, wl, wl + wr) && only_cols_below(b, 0, wl));
    if cs.iter().any(|c| matches!(c, Expr::Cmp(CmpOp::Eq, a, b, _) if crosses(a, b))) {
        return None;
    }
    let i = cs.iter().position(|c| matches!(c, Expr::InList { e, list, negated: false } if !has_subquery(c) && !raises(c) && list.iter().all(|(x, _)| crosses(e, x))))?;
    let Expr::InList { e, list, .. } = cs.remove(i) else { unreachable!() };
    let eq = |x: &X, a: (Aff, Aff)| Expr::Cmp(CmpOp::Eq, e.clone(), Box::new(x.clone()), a);
    let arms = list.iter().enumerate().map(|(k, (x, a))| {
        let mut c = cs.clone();
        c.push(eq(x, *a));
        // Mutant 44: the earlier members' exclusion is dropped (a pair matching two members twice).
        if !crate::mutant::on(44) {
            c.extend(list[..k].iter().map(|(y, b)| Expr::Is { left: Box::new(eq(y, *b)), right: Box::new(Expr::Lit(ir::Lit::Int(1))), negated: true }));
        }
        Rel::Join { kind: ir::JoinKind::Inner, left: left.clone(), right: right.clone(), on: and_all(c) }
    });
    let first = arms.clone().next()?;
    Some(arms.skip(1).fold(first, |a, b| Rel::Union { all: true, left: Box::new(a), right: Box::new(b) }))
}

// --------------------------------------------------------------------------------------------
// The constant evaluator: a subtree that reads no scan and no binding.

fn is_const(r: &Rel<RepoSource>) -> bool {
    match r {
        Rel::Unit => true,
        Rel::Scan { .. } | Rel::Local { .. } | Rel::With { .. } => false,
        Rel::JsonEach { input, arg, on, .. } => is_const(input) && !has_subquery(arg) && on.as_ref().is_none_or(|o| !has_subquery(o)),
        Rel::Project { input, exprs, .. } => is_const(input) && !exprs.iter().any(has_subquery),
        Rel::Filter { input, pred } => is_const(input) && !has_subquery(pred),
        Rel::Union { all: true, left, right } => is_const(left) && is_const(right),
        _ => false,
    }
}

fn rel_width(r: &Rel<RepoSource>, ctx: &Ctx) -> usize {
    match r {
        Rel::Scan { schema, .. } | Rel::Local { schema, .. } | Rel::Project { schema, .. } => schema.len(),
        Rel::Unit => 0,
        Rel::JsonEach { input, .. } => rel_width(input, ctx) + 8,
        Rel::Join { left, right, .. } => rel_width(left, ctx) + rel_width(right, ctx),
        Rel::Filter { input, .. } | Rel::Distinct { input, .. } | Rel::Sort { input, .. } => rel_width(input, ctx),
        Rel::Aggregate { input, aggs, .. } => rel_width(input, ctx) + aggs.len(),
        Rel::Window { input, funcs } => rel_width(input, ctx) + funcs.len(),
        Rel::Union { left, .. } => rel_width(left, ctx),
        Rel::With { body, .. } => rel_width(body, ctx),
    }
}

// --------------------------------------------------------------------------------------------
// Graph building

impl<'a> Ctx<'a> {
    /// Adds a node, or returns the identical node already added (operators are pure, so equal
    /// operator trees are one node: one collection, one arrangement per key).
    fn add(&mut self, op: Op, width: usize, poison: Vec<usize>) -> NodeId {
        let sig = format!("{op:?}|{width}|{poison:?}");
        if let Some(&n) = self.seen.get(&sig) {
            return n;
        }
        self.g.nodes.push(op);
        self.g.widths.push(width);
        self.g.poison.push(poison);
        self.g.labels.push(self.at.clone());
        let n = self.g.nodes.len() - 1;
        self.seen.insert(sig, n);
        n
    }

    fn width(&self, n: NodeId) -> usize {
        self.g.widths[n]
    }

    /// Refuse an expression over node `n`'s rows that reads a poisoned column.
    fn check_reads(&self, n: NodeId, e: &X) -> R<()> {
        let mut c = Vec::new();
        cols_read(e, &mut c);
        if let Some(p) = c.iter().find(|x| self.g.poison[n].contains(x)) {
            return refuse(&self.at, format!("bare column {p} of an aggregate (row-order dependent: SQLite's representative row)"));
        }
        Ok(())
    }

    fn rel(&mut self, r: &Rel<RepoSource>) -> R<NodeId> {
        if is_const(r) {
            let rows = super::scalar::constant(r).or_else(|m| refuse(&self.at, format!("constant relation: {m}")))?;
            let w = rel_width(r, self);
            let rows: Vec<Row> = rows.iter().map(|x| x.iter().map(val::from_v).collect()).collect();
            return Ok(self.add(Op::Const(rows), w, Vec::new()));
        }
        match r {
            Rel::Scan { src: ir::RepoSource::Exported(id), schema } => {
                if self.exported.iter().find(|e| e.id == *id).is_some_and(|e| e.ordered) {
                    // Order is checked where it is observed (window, limit, group_concat).
                }
                Ok(self.add(Op::Exported(*id), schema.len(), Vec::new()))
            }
            Rel::Scan { src: ir::RepoSource::Stage(s), .. } => self.stages.get(*s as usize).copied().ok_or_else(|| Refused { at: self.at.clone(), construct: format!("stage {s} read before it is defined") }),
            Rel::Local { id, schema } => match self.locals.iter().rev().find(|(i, _)| i == id).map(|x| x.1.clone()) {
                Some(Local::Node(n)) => Ok(n),
                Some(Local::Var(v, w)) => {
                    let _ = schema;
                    Ok(self.add(Op::Var(v), w, Vec::new()))
                }
                None => refuse(&self.at, format!("unbound local {id}")),
            },
            Rel::Unit => unreachable!("constant"),
            Rel::JsonEach { .. } => refuse(&self.at, "json_each over a non-constant relation"),
            Rel::Filter { input, pred } => {
                if let Some(p) = push_filters(r) {
                    return self.rel(&p);
                }
                let n = self.rel(input)?;
                self.filter(n, pred)
            }
            Rel::Project { input, exprs, schema } => {
                let n = self.rel(input)?;
                let (n, exprs) = self.hoist_scalars(n, exprs.clone())?;
                for e in &exprs {
                    self.check_reads(n, e)?;
                }
                Ok(self.add(Op::Map { input: n, exprs }, schema.len(), Vec::new()))
            }
            Rel::Join { kind, left, right, on } => {
                if let Some(p) = count_on_exists(r).or_else(|| hoist_on_subqueries(r)).or_else(|| split_in_list_join(r)).or_else(|| push_filters(r)) {
                    return self.rel(&p);
                }
                let l = self.rel(left)?;
                let r = self.rel(right)?;
                self.join(l, r, on.as_ref(), *kind == ir::JoinKind::Left)
            }
            Rel::Aggregate { input, keys, aggs } => {
                if !aggs.iter().any(ordered_agg) {
                    let n = self.rel(input)?;
                    return self.group(n, keys.clone(), aggs.clone(), Vec::new());
                }
                // Ordered aggregates: the input's rows carry the keys of the Sort they come from.
                let (n, desc) = self.rel_keyed(input)?;
                let w = self.width(n);
                let w0 = w - desc.len();
                let order = desc.iter().enumerate().map(|(j, d)| (col(w0 + j), *d)).collect();
                let g = self.group(n, keys.clone(), aggs.clone(), order)?;
                let poison = self.g.poison[g].iter().copied().filter(|c| *c < w0).collect();
                Ok(self.add(Op::Map { input: g, exprs: (0..w0).chain(w..w + aggs.len()).map(col).collect() }, w0 + aggs.len(), poison))
            }
            Rel::Window { input, funcs } => {
                if let Some(p) = hoist_window_subqueries(r) {
                    return self.rel(&p);
                }
                let mut n = self.rel(input)?;
                for f in funcs {
                    for e in f.partition.iter().chain(f.order.iter().map(|x| &x.0)).chain(match &f.kind {
                        WindowKind::Min(e) | WindowKind::Max(e) => Some(e),
                        WindowKind::RowNumber => None,
                    }) {
                        if has_subquery(e) {
                            return refuse(&self.at, "subquery in a window function");
                        }
                        self.check_reads(n, e)?;
                    }
                    let w = self.width(n) + 1;
                    let p = self.g.poison[n].clone();
                    let part: Key = f.partition.iter().map(|e| (e.clone(), KeyMode::Group)).collect();
                    n = self.add(Op::Window { input: n, part, order: f.order.clone(), kind: f.kind.clone() }, w, p);
                }
                Ok(n)
            }
            Rel::Distinct { input, prefix } => {
                let n = self.rel(input)?;
                let w = self.width(n);
                if !self.g.poison[n].is_empty() {
                    return refuse(&self.at, "DISTINCT over bare aggregate columns");
                }
                if *prefix as usize >= w {
                    Ok(self.add(Op::Distinct { input: n }, w, Vec::new()))
                } else {
                    Ok(self.add(Op::DistinctPrefix { input: n, prefix: *prefix as usize }, w, Vec::new()))
                }
            }
            Rel::Union { all, left, right } => {
                let l = self.rel(left)?;
                let r = self.rel(right)?;
                if !self.g.poison[l].is_empty() || !self.g.poison[r].is_empty() {
                    return refuse(&self.at, "UNION over bare aggregate columns");
                }
                let w = self.width(l);
                let c = self.add(Op::Concat { inputs: vec![l, r] }, w, Vec::new());
                Ok(if *all { c } else { self.add(Op::Distinct { input: c }, w, Vec::new()) })
            }
            Rel::Sort { input, keys, limit, offset, .. } => {
                let n = self.rel(input)?;
                if limit.is_none() && offset.is_none() {
                    return Ok(n);
                }
                let c = |e: &Option<X>, at: &str| -> R<i64> {
                    match e {
                        None => Ok(-1),
                        Some(x) => match super::scalar::eval(x, &[], &[]) {
                            Ok(V::Int(i)) => Ok(i),
                            _ => refuse(at, "non-constant LIMIT/OFFSET"),
                        },
                    }
                };
                let (lim, off) = (c(limit, &self.at)?, c(offset, &self.at)?.max(0));
                self.limit(n, keys, vec![], lim, off)
            }
            Rel::With { bindings, body } => {
                let mark = self.locals.len();
                for (id, b) in bindings {
                    let node = match b {
                        Binding::Plain(r) => self.rel(r)?,
                        Binding::Fix { init, step, set, schema } => {
                            if !set {
                                return refuse(&self.at, "recursive CTE with UNION ALL (bag fixpoint) in a repository plan: no exact incremental form");
                            }
                            self.fix(*id, init, step, schema.len())?
                        }
                    };
                    self.locals.push((*id, Local::Node(node)));
                }
                let n = self.rel(body)?;
                self.locals.truncate(mark);
                Ok(n)
            }
        }
    }

    fn fix(&mut self, id: u32, init: &[Rel<RepoSource>], step: &[Rel<RepoSource>], width: usize) -> R<NodeId> {
        let init: Vec<NodeId> = init.iter().map(|r| self.rel(r)).collect::<R<_>>()?;
        self.locals.push((id, Local::Var(id, width)));
        let step: Vec<NodeId> = step.iter().map(|r| self.rel(r)).collect::<R<_>>()?;
        self.locals.pop();
        for &n in init.iter().chain(&step) {
            if !self.g.poison[n].is_empty() {
                return refuse(&self.at, "recursion over bare aggregate columns");
            }
        }
        Ok(self.add(Op::Fix { var: id, init, step }, width, Vec::new()))
    }

    /// `ORDER BY keys LIMIT lim OFFSET off`, per partition `part` (empty: the whole relation):
    /// a window row_number over (part, keys) and a range filter, then the number dropped.
    fn limit(&mut self, n: NodeId, keys: &[(u16, bool)], part: Vec<X>, lim: i64, off: i64) -> R<NodeId> {
        let w = self.width(n);
        let order: Vec<(X, bool)> = keys.iter().map(|(c, d)| (col(*c as usize), *d)).collect();
        for (e, _) in &order {
            self.check_reads(n, e)?;
        }
        let part: Key = part.into_iter().map(|e| (e, KeyMode::Group)).collect();
        let win = self.add(Op::Window { input: n, part, order, kind: WindowKind::RowNumber }, w + 1, self.g.poison[n].clone());
        let rn = col(w);
        let lo = Expr::Cmp(CmpOp::Gt, Box::new(rn.clone()), Box::new(Expr::Lit(ir::Lit::Int(off))), (Aff::None, Aff::None));
        let pred = if lim < 0 { lo } else { Expr::And(Box::new(lo), Box::new(Expr::Cmp(CmpOp::Le, Box::new(rn), Box::new(Expr::Lit(ir::Lit::Int(off + lim))), (Aff::None, Aff::None)))) };
        let f = self.add(Op::Filter { input: win, pred }, w + 1, self.g.poison[n].clone());
        let p = self.g.poison[n].clone();
        Ok(self.add(Op::Map { input: f, exprs: (0..w).map(col).collect() }, w, p))
    }

    /// GROUP BY `keys`. `order`: for ordered aggregates (`group_concat`, `json_group_array`), the
    /// expressions over the input row that order a group's rows (the keys of the Sort below,
    /// `rel_keyed`); the lowering folds in that order and refuses a tie between distinct rows at
    /// run time (no order: any two distinct rows of a group tie).
    fn group(&mut self, n: NodeId, keys: Vec<X>, aggs: Vec<AggCall<RepoSource>>, order: Vec<(X, bool)>) -> R<NodeId> {
        let w = self.width(n);
        for e in keys.iter().chain(aggs.iter().filter_map(|a| a.arg.as_ref())).chain(aggs.iter().filter_map(|a| a.sep.as_ref())).chain(order.iter().map(|x| &x.0)) {
            if has_subquery(e) {
                return refuse(&self.at, "subquery in GROUP BY or an aggregate argument");
            }
            self.check_reads(n, e)?;
        }
        // Representative row: only the key columns are defined (a key that is a plain column).
        let key_cols: Vec<usize> = keys.iter().filter_map(|k| if let Expr::Col { depth: 0, col } = k { Some(*col as usize) } else { None }).collect();
        let poison: Vec<usize> = (0..w).filter(|c| !key_cols.contains(c)).collect();
        let keyless = keys.is_empty();
        let keys: Key = keys.into_iter().map(|e| (e, KeyMode::Group)).collect();
        Ok(self.add(Op::Group { input: n, keys, aggs: aggs.clone(), keyless, width: w, order }, w + aggs.len(), poison))
    }

    /// Group by correlation keys compared with `=` under their affinities (scalar subqueries).
    fn group_eq(&mut self, n: NodeId, keys: Key, aggs: Vec<AggCall<RepoSource>>, order: Vec<(X, bool)>) -> R<NodeId> {
        let w = self.width(n);
        // The keyless group checks the aggregates (refusals); the node built is keyed. Nodes are
        // shared (hash-consed), so the checked node is never patched in place.
        self.group(n, Vec::new(), aggs.clone(), order.clone())?;
        let poison: Vec<usize> = (0..w).filter(|c| !keys.iter().any(|(k, _)| matches!(k, Expr::Col { depth: 0, col } if *col as usize == *c))).collect();
        Ok(self.add(Op::Group { input: n, keyless: keys.is_empty(), keys, aggs: aggs.clone(), width: w, order }, w + aggs.len(), poison))
    }

    /// `r`'s rows each carrying, as trailing columns, the keys of the `Sort` they come from through
    /// `Project`/`Filter` (the reference evaluator's order for ordered aggregates), and the keys'
    /// directions; no such `Sort`: no keys.
    fn rel_keyed(&mut self, r: &Rel<RepoSource>) -> R<(NodeId, Vec<bool>)> {
        match r {
            Rel::Sort { keys, .. } => {
                let n = self.rel(r)?;
                let w = self.width(n);
                for (c, _) in keys {
                    self.check_reads(n, &col(*c as usize))?;
                }
                let p = self.g.poison[n].clone();
                let m = self.add(Op::Map { input: n, exprs: (0..w).map(col).chain(keys.iter().map(|(c, _)| col(*c as usize))).collect() }, w + keys.len(), p);
                Ok((m, keys.iter().map(|k| k.1).collect()))
            }
            Rel::Filter { input, pred } => {
                let (n, d) = self.rel_keyed(input)?;
                Ok((self.filter(n, pred)?, d))
            }
            Rel::Project { input, exprs, schema } => {
                let (n, d) = self.rel_keyed(input)?;
                let w = self.width(n);
                let (n2, exprs) = self.hoist_scalars(n, exprs.clone())?;
                for e in &exprs {
                    self.check_reads(n2, e)?;
                }
                // Scalars hoisted onto `n` are appended after its sort keys, which keep their place.
                let out = exprs.into_iter().chain((w - d.len()..w).map(col)).collect();
                Ok((self.add(Op::Map { input: n2, exprs: out }, schema.len() + d.len(), Vec::new()), d))
            }
            other => Ok((self.rel(other)?, Vec::new())),
        }
    }

    fn join(&mut self, l: NodeId, r: NodeId, on: Option<&X>, left_outer: bool) -> R<NodeId> {
        let (wl, wr) = (self.width(l), self.width(r));
        let mut cs = Vec::new();
        if let Some(o) = on {
            conjuncts(o, &mut cs);
        }
        if cs.iter().any(has_subquery) {
            return refuse(&self.at, "subquery in a JOIN's ON");
        }
        let (mut lk, mut rk, mut rest) = (Vec::new(), Vec::new(), Vec::new());
        for c in cs {
            if let Expr::Cmp(CmpOp::Eq, a, b, (fa, fb)) = &c {
                if only_cols_below(a, 0, wl) && only_cols_below(b, wl, wl + wr) {
                    lk.push(((**a).clone(), KeyMode::Eq(*fa)));
                    rk.push((shift(b, -(wl as isize)), KeyMode::Eq(*fb)));
                    continue;
                }
                if only_cols_below(b, 0, wl) && only_cols_below(a, wl, wl + wr) {
                    lk.push(((**b).clone(), KeyMode::Eq(*fb)));
                    rk.push((shift(a, -(wl as isize)), KeyMode::Eq(*fa)));
                    continue;
                }
            }
            rest.push(c);
        }
        for (e, _) in &lk {
            self.check_reads(l, e)?;
        }
        for (e, _) in &rk {
            self.check_reads(r, e)?;
        }
        let mut poison = self.g.poison[l].clone();
        poison.extend(self.g.poison[r].iter().map(|c| c + wl));
        let keys = (lk.clone(), rk.clone());
        let equi = rest.is_empty() && !lk.is_empty();
        let j = self.add(Op::Join { l, r, lk, rk }, wl + wr, poison.clone());
        let inner = match and_all(rest) {
            None => j,
            Some(p) => {
                self.check_reads(j, &p)?;
                self.add(Op::Filter { input: j, pred: p }, wl + wr, poison.clone())
            }
        };
        if !left_outer {
            return Ok(inner);
        }
        // LEFT JOIN: matched rows ∪ left rows with no match, NULL-extended.
        if !self.g.poison[l].is_empty() {
            return refuse(&self.at, "LEFT JOIN over bare aggregate columns");
        }
        let unmatched = if equi {
            // `ON` is equalities only: a left row is unmatched iff no right row has its keys
            // (an antijoin on the join's own keys and comparison; a NULL key matches nothing). The
            // right side is the join's input, so inside a recursion the antijoin is stratified
            // whenever that input is. Mutant 46: a semijoin instead (matched rows NULL-extended).
            self.add(Op::Semi { l, r, lk: keys.0, rk: keys.1, anti: !crate::mutant::on(46) }, wl, Vec::new())
        } else {
            // A residual condition: a left row matched once is matched for every copy (same row,
            // same matches), so "no match" is an antijoin of the left side against the matched
            // left parts by row identity.
            let lpart = self.add(Op::Map { input: inner, exprs: (0..wl).map(col).collect() }, wl, Vec::new());
            let ident: Key = (0..wl).map(|c| (col(c), KeyMode::Group)).collect();
            self.add(Op::Semi { l, r: lpart, lk: ident.clone(), rk: ident, anti: true }, wl, Vec::new())
        };
        let ext = self.add(Op::Map { input: unmatched, exprs: (0..wl).map(col).chain((0..wr).map(|_| Expr::Lit(ir::Lit::Null))).collect() }, wl + wr, Vec::new());
        Ok(self.add(Op::Concat { inputs: vec![inner, ext] }, wl + wr, poison))
    }

    fn filter(&mut self, mut n: NodeId, pred: &X) -> R<NodeId> {
        let mut cs = Vec::new();
        conjuncts(pred, &mut cs);
        let mut plain = Vec::new();
        let mut i = 0;
        while i < cs.len() {
            let c = cs[i].clone();
            i += 1;
            let (c, neg) = match c {
                Expr::Not(x) if matches!(*x, Expr::Exists { .. } | Expr::InRel { .. }) => (*x, true),
                x => (x, false),
            };
            match c {
                Expr::Exists { rel, negated } => {
                    let ex = Expr::Exists { rel: Box::new(prep_exists(&rel)), negated: negated != neg };
                    // R24: NOT EXISTS splits into conjuncts (antijoins), EXISTS into a disjunction
                    // (a plain conjunct: per-row counts).
                    if let Some(x) = split_exists(&ex) {
                        conjuncts(&x, &mut cs);
                        continue;
                    }
                    let Expr::Exists { rel, negated } = ex else { unreachable!() };
                    n = self.semi(n, &rel, None, negated)?
                }
                Expr::InRel { e, rel, negated, aff } => {
                    if has_subquery(&e) {
                        return refuse(&self.at, "subquery as the left operand of IN");
                    }
                    n = self.semi(n, &rel, Some((*e, aff)), negated != neg)?
                }
                other => plain.push(other),
            }
        }
        if plain.is_empty() {
            return Ok(n);
        }
        let w = self.width(n);
        let (n2, plain) = self.hoist_scalars(n, plain)?;
        if plain.iter().any(has_subquery) {
            return refuse(&self.at, "EXISTS/IN under OR, NOT or CASE (not a conjunct)");
        }
        let pred = and_all(plain).unwrap();
        self.check_reads(n2, &pred)?;
        let p = self.g.poison[n2].clone();
        let f = self.add(Op::Filter { input: n2, pred }, self.width(n2), p.clone());
        if self.width(n2) == w {
            return Ok(f);
        }
        Ok(self.add(Op::Map { input: f, exprs: (0..w).map(col).collect() }, w, p))
    }

    /// A subquery body whose rows (not only existence) are read: an uncorrelated relation as is,
    /// or a correlated WHERE over an uncorrelated input split into body and correlations.
    /// Correlation anywhere else is refused (projections and DISTINCT must be kept here).
    fn rows_of(&mut self, rel: &Rel<RepoSource>) -> R<(Rel<RepoSource>, Vec<(X, X, (Aff, Aff))>)> {
        if !rel_free(rel, 1) {
            return Ok((rel.clone(), Vec::new()));
        }
        let lifted = lift_correlations(rel);
        let rel = lifted.as_ref().unwrap_or(rel);
        if !matches!(rel, Rel::Filter { .. }) {
            return refuse(&self.at, "correlated scalar subquery whose correlation is not its top-level WHERE");
        }
        let (b, c, _) = self.decorrelate(rel)?;
        Ok((b, c))
    }

    /// Split a subquery body into its uncorrelated part and the equality correlations with the
    /// enclosing row: `(body, [(outer expr at depth 0 of the enclosing row, inner expr, affs)],
    /// projection of the first output column)`.
    fn decorrelate(&mut self, rel: &Rel<RepoSource>) -> R<(Rel<RepoSource>, Vec<(X, X, (Aff, Aff))>, Option<X>)> {
        let mut first: Option<X> = None;
        // R22: correlations below the top WHERE (inner-join ONs, nested filters) lifted to it.
        let mut cur = lift_correlations(rel).unwrap_or_else(|| rel.clone());
        // Projection / DISTINCT on top: irrelevant to existence; the first column feeds IN.
        loop {
            match cur {
                Rel::Project { input, exprs, .. } => {
                    if exprs.iter().any(|e| expr_free(e, 1)) {
                        return refuse(&self.at, "correlated subquery projecting an outer column");
                    }
                    first = Some(match first {
                        None => exprs[0].clone(),
                        Some(f) => map_cols(&f, &|d, c| (d == 0).then(|| exprs[c as usize].clone())),
                    });
                    cur = *input;
                }
                Rel::Distinct { input, .. } => cur = *input,
                _ => break,
            }
        }
        let mut corr = Vec::new();
        let body = match cur {
            Rel::Filter { input, .. } if rel_free(&input, 1) => return refuse(&self.at, "correlation below the subquery's WHERE (in a join or nested relation)"),
            Rel::Filter { input, pred } => {
                let mut cs = Vec::new();
                conjuncts(&pred, &mut cs);
                let mut keep = Vec::new();
                for c in cs {
                    if !expr_free(&c, 1) {
                        keep.push(c);
                        continue;
                    }
                    if let Expr::Cmp(CmpOp::Eq, a, b, (fa, fb)) = &c {
                        let outer_only = |x: &X| expr_free(x, 1) && !expr_free(x, 2) && { let mut v = Vec::new(); cols_read(x, &mut v); v.is_empty() };
                        let inner_only = |x: &X| !expr_free(x, 1);
                        let up = |x: &X| map_cols(x, &|d, col| (d == 1).then_some(Expr::Col { depth: 0, col }));
                        if outer_only(a) && inner_only(b) {
                            corr.push((up(a), (**b).clone(), (*fa, *fb)));
                            continue;
                        }
                        if outer_only(b) && inner_only(a) {
                            corr.push((up(b), (**a).clone(), (*fb, *fa)));
                            continue;
                        }
                    }
                    return refuse(&self.at, "correlated subquery with a non-equality correlation");
                }
                match and_all(keep) {
                    None => *input,
                    Some(p) => Rel::Filter { input, pred: p },
                }
            }
            other => {
                if rel_free(&other, 1) {
                    return refuse(&self.at, "correlated subquery whose correlation is not in its WHERE");
                }
                other
            }
        };
        Ok((body, corr, first))
    }

    /// `[NOT] EXISTS (rel)` or `e [NOT] IN (rel)` as a semijoin / antijoin of `n`.
    fn semi(&mut self, n: NodeId, rel: &Rel<RepoSource>, inl: Option<(X, (Aff, Aff))>, anti: bool) -> R<NodeId> {
        let (body, corr, first) = self.decorrelate(rel)?;
        let b = self.rel(&body)?;
        let mut lk: Key = Vec::new();
        let mut rk: Key = Vec::new();
        for (o, i, (fo, fi)) in &corr {
            self.check_reads(n, o)?;
            self.check_reads(b, i)?;
            lk.push((o.clone(), KeyMode::Eq(*fo)));
            rk.push((i.clone(), KeyMode::Eq(*fi)));
        }
        let w = self.width(n);
        let p = self.g.poison[n].clone();
        match inl {
            None => Ok(self.add(Op::Semi { l: n, r: b, lk, rk, anti }, w, p)),
            Some((e, (fe, fr))) => {
                let f = first.ok_or_else(|| Refused { at: self.at.clone(), construct: "IN over a subquery without a projection".into() })?;
                self.check_reads(n, &e)?;
                self.check_reads(b, &f)?;
                let (corr_l, corr_r) = (lk.clone(), rk.clone());
                lk.push((e.clone(), KeyMode::Eq(fe)));
                rk.push((f.clone(), KeyMode::Eq(fr)));
                if !anti {
                    // e IN s is true iff some s row equals e (NULLs never equal): a semijoin.
                    return Ok(self.add(Op::Semi { l: n, r: b, lk, rk, anti: false }, w, p));
                }
                // e NOT IN s (SQLite): with e not NULL, true iff no s row equals e and s holds no
                // NULL; with e NULL, true iff s is empty (`NULL NOT IN ()` = 1). s is the body
                // restricted to the row's correlation keys.
                let notnull = self.add(Op::Filter { input: n, pred: Expr::IsNull { e: Box::new(e.clone()), negated: true } }, w, p.clone());
                let a = self.add(Op::Semi { l: notnull, r: b, lk, rk, anti: true }, w, p.clone());
                let nulls = self.add(Op::Filter { input: b, pred: Expr::IsNull { e: Box::new(f), negated: false } }, self.width(b), self.g.poison[b].clone());
                let kept = self.add(Op::Semi { l: a, r: nulls, lk: corr_l.clone(), rk: corr_r.clone(), anti: true }, w, p.clone());
                let isnull = self.add(Op::Filter { input: n, pred: Expr::IsNull { e: Box::new(e), negated: false } }, w, p.clone());
                let empty = self.add(Op::Semi { l: isnull, r: b, lk: corr_l, rk: corr_r, anti: true }, w, p.clone());
                Ok(self.add(Op::Concat { inputs: vec![kept, empty] }, w, p))
            }
        }
    }

    /// Replace each scalar subquery in `exprs` by a column appended to `n` (a left join with the
    /// subquery's value per correlation key), returning the widened node and rewritten exprs.
    fn hoist_scalars(&mut self, mut n: NodeId, exprs: Vec<X>) -> R<(NodeId, Vec<X>)> {
        let mut out = Vec::new();
        for e in exprs {
            out.push(self.hoist_in(&mut n, &e)?);
        }
        Ok((n, out))
    }

    fn hoist_in(&mut self, n: &mut NodeId, e: &X) -> R<X> {
        let b = |s: &mut Self, n: &mut NodeId, x: &X| -> R<Box<X>> { Ok(Box::new(s.hoist_in(n, x)?)) };
        Ok(match e {
            Expr::Scalar(rel) => {
                let c = self.scalar(*n, rel)?;
                *n = c;
                col(self.width(c) - 1)
            }
            // EXISTS below OR/NOT/CASE or in a select list: `(SELECT count(*) FROM body) > 0`,
            // the count maintained per correlation key like any scalar aggregate (0 for a row
            // with no correlated rows). EXISTS is never NULL, as the comparison of a count.
            Expr::Exists { rel, negated } => {
                let prepped = prep_exists(rel);
                let ex = Expr::Exists { rel: Box::new(prepped.clone()), negated: *negated };
                if let Some(x) = split_exists(&ex) {
                    return self.hoist_in(n, &x);
                }
                let mut body = &prepped;
                loop {
                    match body {
                        Rel::Project { input, exprs, .. } => {
                            if exprs.iter().any(|e| expr_free(e, 1)) {
                                return refuse(&self.at, "correlated subquery projecting an outer column");
                            }
                            body = input;
                        }
                        Rel::Distinct { input, .. } => body = input,
                        _ => break,
                    }
                }
                let count = Rel::Aggregate { input: Box::new(body.clone()), keys: vec![], aggs: vec![AggCall { f: ir::AggFn::CountStar, arg: None, sep: None, distinct: false, total: ir::Total::Proven }] };
                let c = self.scalar(*n, &count)?;
                *n = c;
                let op = if *negated { CmpOp::Eq } else { CmpOp::Gt };
                Expr::Cmp(op, Box::new(col(self.width(c) - 1)), Box::new(Expr::Lit(ir::Lit::Int(0))), (Aff::None, Aff::None))
            }
            Expr::InRel { .. } => return refuse(&self.at, "IN (subquery) under OR, NOT or CASE, or in a select list"),
            Expr::Lit(_) | Expr::Col { .. } => e.clone(),
            Expr::Not(x) => Expr::Not(b(self, n, x)?),
            Expr::Neg(x) => Expr::Neg(b(self, n, x)?),
            Expr::Plus(x) => Expr::Plus(b(self, n, x)?),
            Expr::Cast(x, a) => Expr::Cast(b(self, n, x)?, *a),
            Expr::IsNull { e, negated } => Expr::IsNull { e: b(self, n, e)?, negated: *negated },
            Expr::Arith(o, x, y) => Expr::Arith(*o, b(self, n, x)?, b(self, n, y)?),
            Expr::Concat(x, y) => Expr::Concat(b(self, n, x)?, b(self, n, y)?),
            Expr::And(x, y) => Expr::And(b(self, n, x)?, b(self, n, y)?),
            Expr::Or(x, y) => Expr::Or(b(self, n, x)?, b(self, n, y)?),
            Expr::Cmp(o, x, y, a) => Expr::Cmp(*o, b(self, n, x)?, b(self, n, y)?, *a),
            Expr::Is { left, right, negated } => Expr::Is { left: b(self, n, left)?, right: b(self, n, right)?, negated: *negated },
            Expr::Like { text, pattern, negated } => Expr::Like { text: b(self, n, text)?, pattern: b(self, n, pattern)?, negated: *negated },
            Expr::Func(f, a) => Expr::Func(*f, a.iter().map(|x| self.hoist_in(n, x)).collect::<R<_>>()?),
            Expr::Case { operand, whens, else_ } => Expr::Case {
                operand: match operand {
                    Some(o) => Some(b(self, n, o)?),
                    None => None,
                },
                whens: whens.iter().map(|(w, t, a)| Ok((self.hoist_in(n, w)?, self.hoist_in(n, t)?, *a))).collect::<R<_>>()?,
                else_: match else_ {
                    Some(x) => Some(b(self, n, x)?),
                    None => None,
                },
            },
            Expr::InList { e, list, negated } => Expr::InList { e: b(self, n, e)?, list: list.iter().map(|(x, a)| Ok((self.hoist_in(n, x)?, *a))).collect::<R<_>>()?, negated: *negated },
        })
    }

    /// A scalar subquery as a column appended to `n`.
    fn scalar(&mut self, n: NodeId, rel: &Rel<RepoSource>) -> R<NodeId> {
        // R26: a correlated filter under a derived table inside the aggregate, pulled up to it.
        let pulled = pull_correlation(rel);
        let rel = pulled.as_ref().unwrap_or(rel);
        // Shapes: [Project] over (Aggregate | Filter(HAVING) over Aggregate | Sort LIMIT 1) over a
        // correlated Filter. Peel the projection of the first column and an optional HAVING.
        let mut proj: Option<X> = None;
        let mut having: Option<X> = None;
        let mut cur = rel;
        if let Rel::Project { input, exprs, .. } = cur {
            if exprs.iter().any(|e| expr_free(e, 1) || has_subquery(e)) {
                return refuse(&self.at, "scalar subquery projecting an outer column or a subquery");
            }
            proj = Some(exprs[0].clone());
            cur = input;
        }
        if let Rel::Filter { input, pred } = cur {
            if matches!(**input, Rel::Aggregate { .. }) {
                if expr_free(pred, 1) || has_subquery(pred) {
                    return refuse(&self.at, "correlated HAVING");
                }
                having = Some(pred.clone());
                cur = input;
            }
        }
        let wn = self.width(n);
        let pn = self.g.poison[n].clone();
        match cur {
            Rel::Aggregate { input, keys, aggs } if keys.is_empty() => {
                let (body, corr) = self.rows_of(input)?;
                // Ordered aggregates: the body's rows carry their sort keys (trailing columns),
                // read by the group as its order and dropped right after it.
                let (b, desc) = if aggs.iter().any(ordered_agg) { self.rel_keyed(&body)? } else { (self.rel(&body)?, Vec::new()) };
                let wk = self.width(b);
                let wb = wk - desc.len();
                // Group by the inner correlation expressions; the group row is
                // corr values ++ representative (poisoned) ++ aggs.
                let ck: Vec<X> = corr.iter().map(|(_, i, _)| i.clone()).collect();
                for k in &ck {
                    self.check_reads(b, k)?;
                }
                let keyed = self.add(Op::Map { input: b, exprs: ck.iter().cloned().chain((0..wk).map(col)).collect() }, ck.len() + wk, self.g.poison[b].iter().map(|c| c + ck.len()).collect());
                let gk: Key = corr.iter().enumerate().map(|(i, (_, _, (_, fi)))| (col(i), KeyMode::Eq(*fi))).collect();
                let aggs2: Vec<AggCall<RepoSource>> = aggs.iter().map(|a| AggCall { f: a.f, arg: a.arg.as_ref().map(|e| shift(e, ck.len() as isize)), sep: a.sep.as_ref().map(|e| shift(e, ck.len() as isize)), distinct: a.distinct, total: a.total }).collect();
                let order = desc.iter().enumerate().map(|(j, d)| (col(ck.len() + wb + j), *d)).collect();
                let g = self.group_eq(keyed, gk, aggs2, order)?;
                let g = if desc.is_empty() {
                    g
                } else {
                    let (gw, p) = (self.width(g), self.g.poison[g].iter().copied().filter(|c| *c < ck.len() + wb).collect());
                    self.add(Op::Map { input: g, exprs: (0..ck.len() + wb).chain(ck.len() + wk..gw).map(col).collect() }, ck.len() + wb + aggs.len(), p)
                };
                // The value per group: HAVING then the projection, both over (rep ++ aggs) of the
                // original aggregate, i.e. columns shifted by the correlation keys.
                let shiftk = |e: &X| shift(e, ck.len() as isize);
                let gw = self.width(g);
                let value_of = |pe: &Option<X>| pe.as_ref().map(shiftk).unwrap_or_else(|| col(ck.len() + wb));
                let mut vexpr = value_of(&proj);
                if let Some(h) = &having {
                    vexpr = Expr::Case { operand: None, whens: vec![(shiftk(h), vexpr, (Aff::None, Aff::None))], else_: None };
                }
                self.check_reads(g, &vexpr)?;
                let vals = self.add(Op::Map { input: g, exprs: (0..ck.len()).map(col).chain([vexpr]).collect() }, ck.len() + 1, Vec::new());
                let _ = gw;
                // The empty group's value (no correlated rows): the aggregate over no rows, as the
                // reference evaluator folds it (`count` 0, `total` 0.0, `json_group_array` '[]').
                let mut empty_row: Vec<V> = vec![V::Null; wb];
                for a in aggs {
                    empty_row.push(rerail_facts::reference::fold_aggregate(a, std::iter::empty()).map_err(|m| Refused { at: self.at.clone(), construct: format!("scalar subquery empty value: {m}") })?);
                }
                let mut ev = proj.clone().unwrap_or_else(|| col(wb));
                if let Some(h) = &having {
                    ev = Expr::Case { operand: None, whens: vec![(h.clone(), ev, (Aff::None, Aff::None))], else_: None };
                }
                let empty = super::scalar::eval(&ev, &empty_row, &[]).map_err(|m| Refused { at: self.at.clone(), construct: format!("scalar subquery empty value: {m}") })?;
                self.attach(n, wn, pn, vals, &corr, empty)
            }
            Rel::Sort { input, keys, limit: Some(l), offset: None, .. } if matches!(super::scalar::eval(l, &[], &[]), Ok(V::Int(1))) => {
                // (SELECT first FROM (SELECT sel…, keys… FROM corr-filtered) ORDER BY keys LIMIT 1):
                // per correlation key, the row first in the (checked total) order.
                let (sel, inner) = match &**input {
                    Rel::Project { input, exprs, .. } => (exprs.clone(), &**input),
                    other => ((0..rel_width(other, self)).map(col).collect(), other),
                };
                if sel.iter().any(|e| expr_free(e, 1) || has_subquery(e)) {
                    return refuse(&self.at, "scalar subquery projecting an outer column or a subquery");
                }
                let (body, corr) = self.rows_of(inner)?;
                let b = self.rel(&body)?;
                let k = corr.len();
                for (_, i, _) in &corr {
                    self.check_reads(b, i)?;
                }
                for e in &sel {
                    self.check_reads(b, e)?;
                }
                let keyed = self.add(Op::Map { input: b, exprs: corr.iter().map(|(_, i, _)| i.clone()).chain(sel.iter().cloned()).collect() }, k + sel.len(), Vec::new());
                let part: Key = corr.iter().enumerate().map(|(i, (_, _, (_, fi)))| (col(i), KeyMode::Eq(*fi))).collect();
                let order: Vec<(X, bool)> = keys.iter().map(|(c, d)| (col(k + *c as usize), *d)).collect();
                let wk = k + sel.len();
                let win = self.add(Op::Window { input: keyed, part: part.clone(), order, kind: WindowKind::RowNumber }, wk + 1, Vec::new());
                let one = Expr::Cmp(CmpOp::Eq, Box::new(col(wk)), Box::new(Expr::Lit(ir::Lit::Int(1))), (Aff::None, Aff::None));
                let first = self.add(Op::Filter { input: win, pred: one }, wk + 1, Vec::new());
                let value = proj.as_ref().map(|p| shift(p, k as isize)).unwrap_or_else(|| col(k));
                let vals = self.add(Op::Map { input: first, exprs: (0..k).map(col).chain([value]).collect() }, k + 1, Vec::new());
                self.attach(n, wn, pn, vals, &corr, V::Null)
            }
            _ => refuse(&self.at, "scalar subquery shape (not an aggregate over a correlated WHERE)"),
        }
    }

    /// Left-join `n` with `vals` (corr keys ++ value) on the correlation; a row with no group gets
    /// `empty`. Output: n's columns ++ value.
    fn attach(&mut self, n: NodeId, wn: usize, pn: Vec<usize>, vals: NodeId, corr: &[(X, X, (Aff, Aff))], empty: V) -> R<NodeId> {
        let k = corr.len();
        // The group key was computed from the inner expressions with grouping identity; the
        // correlation compares with `=` under each side's comparison affinity.
        let lk: Key = corr.iter().map(|(o, _, (fo, _))| (o.clone(), KeyMode::Eq(*fo))).collect();
        let rk: Key = corr.iter().enumerate().map(|(i, (_, _, (_, fi)))| (col(i), KeyMode::Eq(*fi))).collect();
        for (o, _, _) in corr {
            self.check_reads(n, o)?;
        }
        let j = self.add(Op::Join { l: n, r: vals, lk: lk.clone(), rk: rk.clone() }, wn + k + 1, pn.clone());
        let matched = self.add(Op::Map { input: j, exprs: (0..wn).map(col).chain([col(wn + k)]).collect() }, wn + 1, pn.clone());
        let none = self.add(Op::Semi { l: n, r: vals, lk, rk, anti: true }, wn, pn.clone());
        let lit = match &empty {
            V::Null => ir::Lit::Null,
            V::Int(i) => ir::Lit::Int(*i),
            V::Real(f) => ir::Lit::Real(*f),
            V::Text(s) => ir::Lit::Text(s.as_ref().into()),
        };
        let none = self.add(Op::Map { input: none, exprs: (0..wn).map(col).chain([Expr::Lit(lit)]).collect() }, wn + 1, pn.clone());
        Ok(self.add(Op::Concat { inputs: vec![matched, none] }, wn + 1, pn))
    }
}

/// Normalise one repository plan. Refusals name the construct and the stage.
pub fn plan(p: &ir::Plan<RepoSource>, exported: &[ir::ExportedRelation]) -> Result<Graph, Vec<Refused>> {
    let mut ctx = Ctx { g: Graph::default(), stages: Vec::new(), exported, locals: Vec::new(), at: String::new(), seen: FxHashMap::default() };
    let mut refused = Vec::new();
    for (si, st) in p.stages.iter().enumerate() {
        ctx.at = format!("{}/{}", p.clause, st.name);
        let r = match &st.def {
            StageDef::Query { rel } => ctx.rel(rel).map(|n| {
                // A stage stores its rows with its columns' affinities (CTAS).
                let affs: Vec<Aff> = st.schema.iter().map(|c| c.aff).collect();
                if affs.iter().all(|a| *a == Aff::None) {
                    return n;
                }
                let (w, p) = (ctx.width(n), ctx.g.poison[n].clone());
                ctx.add(Op::Affinity { input: n, affs }, w, p)
            }),
            StageDef::Reach { roots, edges, graph } => match graph {
                ir::Graph::Directed => (|| {
                    let rn = ctx.rel(roots)?;
                    let en = ctx.rel(edges.as_ref().ok_or_else(|| Refused { at: ctx.at.clone(), construct: "reachability without edges".into() })?)?;
                    // init: (path, id, id) for non-NULL ids; step: (path, root, node) ⋈ edges(path, src, dst).
                    let nn = |c: usize| Expr::IsNull { e: Box::new(col(c)), negated: true };
                    let rf = ctx.add(Op::Filter { input: rn, pred: Expr::And(Box::new(nn(0)), Box::new(nn(1))) }, 2, Vec::new());
                    let init = ctx.add(Op::Map { input: rf, exprs: vec![col(0), col(1), col(1)] }, 3, Vec::new());
                    let var = u32::MAX - si as u32;
                    let v = ctx.add(Op::Var(var), 3, Vec::new());
                    let k = |a: usize, b: usize| vec![(col(a), KeyMode::Eq(Aff::None)), (col(b), KeyMode::Eq(Aff::None))];
                    let j = ctx.add(Op::Join { l: v, r: en, lk: k(0, 2), rk: k(0, 1) }, 6, Vec::new());
                    let step = ctx.add(Op::Map { input: j, exprs: vec![col(0), col(1), col(5)] }, 3, Vec::new());
                    Ok(ctx.add(Op::Fix { var, init: vec![init], step: vec![step] }, 3, Vec::new()))
                })(),
                ir::Graph::Forest => refuse(&ctx.at, "forest reachability in a repository plan (its two-parent and cycle failures are whole-graph checks)"),
                ir::Graph::Syntax => refuse(&ctx.at, "syntax reachability in a repository plan"),
            },
        };
        match r {
            Ok(n) => ctx.stages.push(n),
            Err(e) => {
                refused.push(e);
                // The plan is refused; an empty stand-in of the stage's width lets the later
                // stages be normalised, so every refused construct is named, not only the first.
                let n = ctx.add(Op::Const(Vec::new()), st.schema.len(), Vec::new());
                ctx.stages.push(n);
            }
        }
    }
    ctx.at = format!("{}/<final>", p.clause);
    match &p.sink {
        Sink::Findings { rel, .. } => match ctx.rel(rel) {
            Ok(n) => {
                let names = match rel {
                    Rel::Project { schema, .. } => schema.iter().map(|c| c.name.clone()).collect(),
                    _ => Vec::new(),
                };
                ctx.g.outputs.push((p.clause.clone(), n, names));
                // `FACTS_STAGE_OUTPUTS=1` (diagnosis): every stage's relation is an output too
                // (`<clause>/stage/<name>`), so a parity run reports its rows.
                if std::env::var_os("FACTS_STAGE_OUTPUTS").is_some() {
                    for (st, n) in p.stages.iter().zip(ctx.stages.clone()) {
                        ctx.g.outputs.push((format!("{}/stage/{}", p.clause, st.name), n, Vec::new()));
                    }
                }
            }
            Err(e) => refused.push(e),
        },
        Sink::Budget { .. } => refused.push(Refused { at: ctx.at.clone(), construct: "budget sink in a repository plan (cross-file closure/group): not lowered yet".into() }),
        Sink::Witnesses { .. } | Sink::Export(_) => refused.push(Refused { at: ctx.at.clone(), construct: "per-file sink in a repository plan".into() }),
    }
    if refused.is_empty() { Ok(ctx.g) } else { Err(refused) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rerail_facts::ir::{Column, Plan, Violation};
    use rerail_facts::reference::{evaluate_repository, multiset};

    fn c(col: u16) -> X {
        Expr::Col { depth: 0, col }
    }
    fn lit(i: i64) -> X {
        Expr::Lit(ir::Lit::Int(i))
    }
    fn cmp(op: CmpOp, a: X, b: X) -> X {
        Expr::Cmp(op, Box::new(a), Box::new(b), (Aff::None, Aff::None))
    }
    fn and(a: X, b: X) -> X {
        Expr::And(Box::new(a), Box::new(b))
    }
    fn is_null(e: X) -> X {
        Expr::IsNull { e: Box::new(e), negated: false }
    }
    fn scan(id: u32, names: &[&str]) -> Rel<RepoSource> {
        Rel::Scan { src: RepoSource::Exported(id), schema: names.iter().map(|n| Column::new(n, Aff::None)).collect() }
    }
    fn plan(rel: Rel<RepoSource>) -> Plan<RepoSource> {
        Plan { clause: "t".into(), syntax: None, stages: Vec::new(), sink: Sink::Findings { rel, key: Vec::new(), code: None, locations: Vec::new(), violation: Violation { code: "t".into(), anchor: ir::Anchor::Span, each: None } } }
    }
    /// R15 applied until nothing moves, at the top and inside the join's inputs.
    fn pushed(r: &Rel<RepoSource>) -> Rel<RepoSource> {
        let mut cur = r.clone();
        while let Some(p) = push_filters(&cur) {
            cur = p;
        }
        cur
    }

    /// R15's differential test: for random tables (NULLs included) and filters over inner and
    /// LEFT joins mixing left-only, right-only and two-sided conjuncts in WHERE and ON, the
    /// reference evaluator gives the same multiset before and after the rewrite.
    #[test]
    fn rewrite_filter_pushdown_equals_unpushed() {
        let mut seed: u64 = 0x5eed_f15;
        let mut next = move |m: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % m
        };
        let a = scan(0, &["x", "y"]);
        let b = scan(1, &["x", "z"]);
        let on_eq = cmp(CmpOp::Eq, c(0), c(2));
        let preds = [
            cmp(CmpOp::Gt, c(1), lit(1)),
            is_null(c(3)),
            cmp(CmpOp::Gt, c(3), lit(0)),
            and(cmp(CmpOp::Lt, c(1), lit(3)), is_null(c(3))),
            and(cmp(CmpOp::Eq, c(1), c(3)), cmp(CmpOp::Ne, c(2), lit(1))),
        ];
        let mut fired = 0;
        for round in 0..200 {
            let mut table = |n: u64| -> Vec<Vec<V>> {
                (0..next(n))
                    .map(|_| (0..2).map(|_| if next(5) == 0 { V::Null } else { V::Int(next(4) as i64) }).collect())
                    .collect()
            };
            let (ta, tb) = (table(7), table(7));
            let exported = |id: ir::ExportId| -> Result<Vec<Vec<V>>, rerail_facts::reference::Error> { Ok(if id == 0 { ta.clone() } else { tb.clone() }) };
            let kind = if round % 2 == 0 { ir::JoinKind::Inner } else { ir::JoinKind::Left };
            for p in &preds {
                for on in [on_eq.clone(), and(on_eq.clone(), cmp(CmpOp::Gt, c(3), lit(1))), and(on_eq.clone(), cmp(CmpOp::Lt, c(1), lit(2)))] {
                    let j = Rel::Join { kind, left: Box::new(a.clone()), right: Box::new(b.clone()), on: Some(on) };
                    let orig = Rel::Filter { input: Box::new(j), pred: p.clone() };
                    let new = pushed(&orig);
                    if format!("{new:?}") != format!("{orig:?}") {
                        fired += 1;
                    }
                    let want = multiset(&evaluate_repository(&plan(orig.clone()), &exported).unwrap());
                    let got = multiset(&evaluate_repository(&plan(new.clone()), &exported).unwrap());
                    assert_eq!(want, got, "round {round}: {orig:?}\n→ {new:?}\nA {ta:?}\nB {tb:?}");
                }
            }
        }
        assert!(fired > 1000, "the rewrite fired {fired} times");
    }

    /// Where R15 must not move a conjunct: right-only WHERE and left-only ON under a LEFT join,
    /// a correlated conjunct, a subquery.
    #[test]
    fn rewrite_filter_pushdown_side_conditions() {
        let a = scan(0, &["x", "y"]);
        let b = scan(1, &["x", "z"]);
        let lj = |on: X| Rel::Join { kind: ir::JoinKind::Left, left: Box::new(a.clone()), right: Box::new(b.clone()), on: Some(on) };
        let on_eq = cmp(CmpOp::Eq, c(0), c(2));
        assert!(push_filters(&Rel::Filter { input: Box::new(lj(on_eq.clone())), pred: is_null(c(3)) }).is_none());
        assert!(push_filters(&lj(and(on_eq.clone(), cmp(CmpOp::Gt, c(1), lit(0))))).is_none());
        let corr = cmp(CmpOp::Eq, c(1), Expr::Col { depth: 1, col: 0 });
        let ij = Rel::Join { kind: ir::JoinKind::Inner, left: Box::new(a.clone()), right: Box::new(b.clone()), on: Some(on_eq) };
        assert!(push_filters(&Rel::Filter { input: Box::new(ij.clone()), pred: corr }).is_none());
        let sub = Expr::Exists { rel: Box::new(scan(2, &["w"])), negated: false };
        assert!(push_filters(&Rel::Filter { input: Box::new(ij), pred: sub }).is_none());
    }

    // ----------------------------------------------------------------------------------------
    // R20–R32 and ordered aggregates: differential tests. Each rewrite is applied where the
    // normaliser applies it; the reference evaluator must give the same result before and after
    // (the same multiset, or the same `tie:` refusal) on random tables with NULLs; and the
    // generated dataflow must give the reference's result on the unrewritten query (the
    // normaliser rewrites it itself), refusing exactly where the reference raises a tie.

    type Tables = Vec<Vec<Vec<V>>>;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self, m: u64) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) % m
        }
        fn tables(&mut self) -> Tables {
            (0..3).map(|_| (0..self.next(7)).map(|_| (0..2).map(|_| if self.next(5) == 0 { V::Null } else { V::Int(self.next(4) as i64) }).collect()).collect()).collect()
        }
    }
    fn o(depth: u16, col: u16) -> X {
        Expr::Col { depth, col }
    }
    fn eq(a: X, b: X) -> X {
        cmp(CmpOp::Eq, a, b)
    }
    fn or(a: X, b: X) -> X {
        Expr::Or(Box::new(a), Box::new(b))
    }
    fn exists(r: Rl, negated: bool) -> X {
        Expr::Exists { rel: Box::new(r), negated }
    }
    fn sub(r: Rl) -> X {
        Expr::Scalar(Box::new(r))
    }
    fn filt(r: Rl, p: X) -> Rl {
        Rel::Filter { input: Box::new(r), pred: p }
    }
    fn join(kind: ir::JoinKind, l: Rl, r: Rl, on: Option<X>) -> Rl {
        Rel::Join { kind, left: Box::new(l), right: Box::new(r), on }
    }
    fn proj(r: Rl, exprs: Vec<X>) -> Rl {
        let schema = (0..exprs.len()).map(|i| Column::new(&format!("c{i}"), Aff::None)).collect();
        Rel::Project { input: Box::new(r), exprs, schema }
    }
    fn call(f: ir::AggFn, arg: Option<X>, total: ir::Total) -> AggCall<RepoSource> {
        AggCall { f, arg, sep: None, distinct: false, total }
    }
    fn agg(r: Rl, aggs: Vec<AggCall<RepoSource>>) -> Rl {
        Rel::Aggregate { input: Box::new(r), keys: vec![], aggs }
    }
    fn sort(r: Rl, keys: Vec<(u16, bool)>, limit: Option<i64>) -> Rl {
        Rel::Sort { input: Box::new(r), keys, limit: limit.map(lit), offset: None, total: ir::Total::Checked }
    }
    fn tabs() -> (Rl, Rl, Rl) {
        (scan(0, &["x", "y"]), scan(1, &["x", "z"]), scan(2, &["u", "v"]))
    }
    const INNER: ir::JoinKind = ir::JoinKind::Inner;
    const LEFT: ir::JoinKind = ir::JoinKind::Left;

    fn reference(r: &Rl, t: &Tables) -> Result<BTreeMapRows, String> {
        let exported = |id: ir::ExportId| -> Result<Vec<Vec<V>>, rerail_facts::reference::Error> { Ok(t[id as usize].clone()) };
        evaluate_repository(&plan(r.clone()), &exported).map(|rows| {
            let mut m = BTreeMapRows::new();
            for r in rows {
                *m.entry(r.iter().map(val::from_v).collect::<Row>()).or_default() += 1;
            }
            m
        })
    }
    type BTreeMapRows = std::collections::BTreeMap<Row, i32>;

    /// Before vs after under the reference: equal multisets, or both a `tie:` refusal.
    fn agree(before: &Rl, after: &Rl, t: &Tables) {
        match (reference(before, t), reference(after, t)) {
            (Ok(a), Ok(b)) => assert_eq!(a, b, "{before:?}\n→ {after:?}\ntables {t:?}"),
            (Err(a), Err(b)) if a.starts_with("tie") && b.starts_with("tie") => {}
            (a, b) => panic!("before {a:?}, after {b:?}\n{before:?}\n→ {after:?}\ntables {t:?}"),
        }
    }

    fn install() {
        val::install(std::rc::Rc::new(std::cell::RefCell::new(crate::derive::World::new(crate::store::Store::default()))));
    }

    /// The generated dataflow's rows for `r` over `t` (one content, base tag), or `Err` with its
    /// live runtime refusals.
    fn generated(r: &Rl, t: &Tables) -> Result<BTreeMapRows, String> {
        let g = std::rc::Rc::new(super::plan(&plan(r.clone()), &[]).map_err(|e| format!("normalise: {e:?}"))?);
        let rows: Vec<std::rc::Rc<[Row]>> = t.iter().map(|x| x.iter().map(|r| r.iter().map(val::from_v).collect::<Row>()).collect::<Vec<_>>().into()).collect();
        // A worker on this thread (as `timely::execute_directly`, whose closure must be Send).
        let alloc = timely::communication::Allocator::Thread(timely::communication::allocator::thread::Thread::default());
        let mut worker = timely::worker::Worker::new(timely::WorkerConfig::default(), alloc, Some(std::time::Instant::now()));
        let probe = timely::dataflow::operators::probe::Handle::new();
        let src: super::super::lower::Source = std::rc::Rc::new(move |e, _, _| rows[e as usize].clone());
        let mut built = super::super::lower::build(&mut worker, vec![("t".into(), g)], src, &probe, false);
        built.files.update((super::super::tags::BASE, (0, 0)), 1);
        built.files.advance_to(1);
        built.files.flush();
        worker.step_while(|| probe.less_than(&1));
        let refused = super::super::lower::full(&built.refusals[0].1, super::super::tags::BASE)?;
        if !refused.is_empty() {
            return Err(format!("refused: {:?}", refused.keys().map(|r| format!("{:?}", val::to_v(r[0]))).collect::<Vec<_>>()));
        }
        super::super::lower::full(&built.outputs[0].1, super::super::tags::BASE)
    }

    /// The dataflow against the reference on the unrewritten query: equal rows, or a runtime
    /// refusal exactly where the reference raises a tie.
    fn agree_generated(r: &Rl, t: &Tables) {
        match (reference(r, t), generated(r, t)) {
            (Ok(a), Ok(b)) => assert_eq!(a, b, "generated ≠ reference for {r:?}\ntables {t:?}"),
            (Err(a), Err(b)) if a.starts_with("tie") && b.starts_with("refused") => {}
            (a, b) => panic!("reference {a:?}, generated {b:?}\n{r:?}\ntables {t:?}"),
        }
    }

    /// Runs `cases` (query builders over the rewrite under test, given `true` for the rewritten
    /// form) on `rounds` random table sets: reference before ≡ after, and the dataflow ≡ the
    /// reference on the unrewritten form. Returns how many cases the rewrite changed.
    fn differential(seed: u64, rounds: usize, cases: &[&dyn Fn(bool) -> Rl]) -> usize {
        install();
        let mut rng = Rng(seed);
        let fired = cases.iter().filter(|c| format!("{:?}", c(false)) != format!("{:?}", c(true))).count();
        for round in 0..rounds {
            let t = rng.tables();
            for c in cases {
                agree(&c(false), &c(true), &t);
                if round % 4 == 0 {
                    agree_generated(&c(false), &t);
                }
            }
        }
        fired
    }

    /// R20: scalar and EXISTS subqueries of an `ON` reading the left row only.
    #[test]
    fn rewrite_on_subqueries_to_left_columns() {
        let t = tabs();
        let (a, b, cc) = (&t.0, &t.1, &t.2);
        let hoist = |r: Rl, on: bool| if on { hoist_on_subqueries(&r).expect("R20 fires") } else { r };
        // min(C.v) over C.u = A.x; first C.v by v over C.u = A.y; EXISTS C.u = A.y AND C.v > 1.
        let min_v = || sub(proj(agg(filt(cc.clone(), eq(c(0), o(1, 0))), vec![call(ir::AggFn::Min, Some(c(1)), ir::Total::Proven)]), vec![c(2)]));
        let first_v = || sub(proj(sort(filt(cc.clone(), eq(c(0), o(1, 1))), vec![(1, false), (0, false)], Some(1)), vec![c(1)]));
        let ex = || exists(filt(cc.clone(), and(eq(c(0), o(1, 1)), cmp(CmpOp::Gt, c(1), lit(1)))), false);
        let k1 = |kind| move |on: bool| hoist(join(kind, a.clone(), b.clone(), Some(eq(c(2), min_v()))), on);
        let k2 = |kind| move |on: bool| hoist(join(kind, a.clone(), b.clone(), Some(and(eq(c(2), c(0)), cmp(CmpOp::Ge, c(3), Expr::Func(ir::Func::Coalesce, vec![first_v(), lit(1)]))))), on);
        let k3 = |kind| move |on: bool| hoist(join(kind, a.clone(), b.clone(), Some(or(eq(c(3), c(1)), ex()))), on);
        let (i1, l1, i2, l2, i3, l3) = (k1(INNER), k1(LEFT), k2(INNER), k2(LEFT), k3(INNER), k3(LEFT));
        assert_eq!(differential(0x5eed_20, 150, &[&i1, &l1, &i2, &l2, &i3, &l3]), 6);
    }

    /// R20's side conditions: a subquery reading a right column, or a row outside the join, stays.
    #[test]
    fn rewrite_on_subqueries_side_conditions() {
        let t = tabs();
        let (a, b, cc) = (&t.0, &t.1, &t.2);
        let right = sub(proj(agg(filt(cc.clone(), eq(c(0), o(1, 3))), vec![call(ir::AggFn::Max, Some(c(1)), ir::Total::Proven)]), vec![c(2)]));
        assert!(hoist_on_subqueries(&join(INNER, a.clone(), b.clone(), Some(eq(c(2), right)))).is_none());
        let outer = exists(filt(cc.clone(), and(eq(c(0), o(1, 0)), eq(c(1), o(2, 0)))), false);
        assert!(hoist_on_subqueries(&join(INNER, a.clone(), b.clone(), Some(outer))).is_none());
    }

    /// R28: an uncorrelated EXISTS / NOT EXISTS under OR in an `ON`.
    #[test]
    fn rewrite_on_exists_to_count() {
        let t = tabs();
        let (a, b, cc) = (&t.0, &t.1, &t.2);
        let r = |r: Rl, on: bool| if on { count_on_exists(&r).expect("R28 fires") } else { r };
        let ex = |neg| exists(proj(filt(cc.clone(), cmp(CmpOp::Gt, c(1), lit(1))), vec![lit(1)]), neg);
        let k = |kind, neg| move |on: bool| r(join(kind, a.clone(), b.clone(), Some(and(or(eq(c(2), c(0)), ex(neg)), cmp(CmpOp::Ne, c(3), c(1))))), on);
        let (q1, q2, q3, q4) = (k(INNER, false), k(LEFT, false), k(INNER, true), k(LEFT, true));
        assert_eq!(differential(0x5eed_28, 150, &[&q1, &q2, &q3, &q4]), 4);
    }

    /// R22: correlations inside inner-join `ON`s and nested filters of an EXISTS or aggregate body.
    #[test]
    fn rewrite_lift_correlations() {
        let t = tabs();
        let (a, b, cc) = (&t.0, &t.1, &t.2);
        // body: B ⋈ (C where C.u = A.x) on C.u = B.z ∧ C.v = A.y (correlated), WHERE B.x = A.x.
        let body = |kind| filt(join(kind, b.clone(), filt(cc.clone(), eq(c(0), o(1, 0))), Some(and(eq(c(2), c(1)), eq(c(3), o(1, 1))))), eq(c(0), o(1, 0)));
        let lift = |r: Rl, on: bool| if on { lift_correlations(&r).unwrap_or(r) } else { r };
        let q_exists = |kind, neg| move |on: bool| filt(a.clone(), exists(lift(proj(body(kind), vec![lit(1)]), on), neg));
        let q_count = move |on: bool| proj(a.clone(), vec![c(0), sub(proj(agg(lift(body(INNER), on), vec![call(ir::AggFn::CountStar, None, ir::Total::Proven)]), vec![c(4)]))]);
        let (q1, q2, q3) = (q_exists(INNER, false), q_exists(INNER, true), q_exists(LEFT, false));
        // The LEFT join's correlation stays where it is (q3 is unchanged and normalises refused,
        // so it is checked under the reference only).
        install();
        let mut rng = Rng(0x5eed_22);
        for _ in 0..150 {
            let t = rng.tables();
            agree(&q3(false), &q3(true), &t);
        }
        assert!(lift_correlations(&proj(body(LEFT), vec![lit(1)])).is_none(), "R22 crossed a LEFT join");
        assert_eq!(differential(0x5eed_22, 150, &[&q1, &q2, &q_count]), 3);
    }

    /// R24: a disjunctive correlation in EXISTS / NOT EXISTS, in WHERE and in a select list.
    #[test]
    fn rewrite_split_disjunctive_exists() {
        let t = tabs();
        let (a, b) = (&t.0, &t.1);
        let body = || filt(b.clone(), and(or(or(eq(c(0), o(1, 0)), eq(c(1), o(1, 1))), and(eq(c(0), o(1, 1)), is_null(c(1)))), cmp(CmpOp::Gt, c(1), lit(0))));
        let split = |neg: bool, on: bool| if on { split_exists(&exists(body(), neg)).expect("R24 fires") } else { exists(body(), neg) };
        let w = |neg| move |on: bool| filt(a.clone(), split(neg, on));
        let s = move |on: bool| proj(a.clone(), vec![c(0), split(false, on), split(true, on)]);
        let (w1, w2) = (w(false), w(true));
        assert_eq!(differential(0x5eed_24, 150, &[&w1, &w2, &s]), 3);
    }

    /// R32: a nested EXISTS reading both the EXISTS body's row and the row outside it.
    #[test]
    fn rewrite_flatten_nested_exists() {
        let t = tabs();
        let (a, b, cc) = (&t.0, &t.1, &t.2);
        let inner = |neg| exists(filt(cc.clone(), and(eq(c(0), o(2, 1)), eq(c(1), o(1, 1)))), neg);
        let body = |neg| proj(filt(b.clone(), and(eq(c(0), o(1, 0)), inner(neg))), vec![lit(1)]);
        let flat = |neg: bool, on: bool| if on { flatten_exists(&body(neg)).unwrap_or_else(|| body(neg)) } else { body(neg) };
        let q = |neg| move |on: bool| filt(a.clone(), exists(flat(neg, on), false));
        let (q1, q2) = (q(false), q(true));
        install();
        let mut rng = Rng(0x5eed_32);
        for _ in 0..150 {
            let t = rng.tables();
            agree(&q2(false), &q2(true), &t);
        }
        assert_eq!(differential(0x5eed_32, 150, &[&q1]), 1);
    }

    /// R26: a correlated filter under a derived table (Project / Sort) inside an aggregate,
    /// ordered (`json_group_array`, `group_concat`) and not (`count`).
    #[test]
    fn rewrite_pull_correlation_into_aggregate() {
        let t = tabs();
        let (a, cc) = (&t.0, &t.2);
        let pull = |r: Rl, on: bool| if on { pull_correlation(&r).expect("R26 fires") } else { r };
        let derived = || proj(sort(proj(filt(cc.clone(), and(eq(c(0), o(1, 0)), cmp(CmpOp::Ne, c(1), lit(3)))), vec![c(1), c(0)]), vec![(1, true), (0, false)], None), vec![c(0)]);
        let json = move |on: bool| proj(a.clone(), vec![c(0), sub(pull(proj(agg(derived(), vec![call(ir::AggFn::JsonGroupArray, Some(c(0)), ir::Total::Checked)]), vec![c(1)]), on))]);
        let a2 = &t.0;
        let derived2 = || proj(sort(proj(filt(tabs().2, and(eq(c(0), o(1, 0)), cmp(CmpOp::Ne, c(1), lit(3)))), vec![c(1), c(0)]), vec![(1, true), (0, false)], None), vec![c(0)]);
        let concat = move |on: bool| proj(a2.clone(), vec![sub(pull(proj(agg(derived2(), vec![call(ir::AggFn::GroupConcat, Some(c(0)), ir::Total::Checked)]), vec![c(1)]), on))]);
        let a3 = &t.0;
        let count = move |on: bool| filt(a3.clone(), cmp(CmpOp::Gt, sub(pull(proj(agg(proj(filt(tabs().2, eq(c(0), o(1, 1))), vec![c(1)]), vec![call(ir::AggFn::CountStar, None, ir::Total::Proven)]), vec![c(1)]), on)), lit(1)));
        assert_eq!(differential(0x5eed_26, 150, &[&json, &concat, &count]), 3);
        // Side condition: never through a Sort with LIMIT.
        let limited = proj(agg(sort(proj(filt(tabs().2, eq(c(0), o(1, 0))), vec![c(1)]), vec![(0, false)], Some(1)), vec![call(ir::AggFn::Max, Some(c(0)), ir::Total::Proven)]), vec![c(1)]);
        assert!(pull_correlation(&limited).is_none());
    }

    /// R30: a correlated count in a window's ORDER BY and a scalar as a window argument.
    #[test]
    fn rewrite_window_subqueries() {
        let t = tabs();
        let (a, cc) = (&t.0, &t.2);
        let cnt = || sub(proj(agg(filt(cc.clone(), eq(c(0), o(1, 0))), vec![call(ir::AggFn::CountStar, None, ir::Total::Proven)]), vec![c(2)]));
        let mx = || sub(proj(agg(filt(cc.clone(), eq(c(0), o(1, 1))), vec![call(ir::AggFn::Max, Some(c(1)), ir::Total::Proven)]), vec![c(2)]));
        let win = |kind, order: Vec<(X, bool)>, on: bool| {
            let r = Rel::Window { input: Box::new(a.clone()), funcs: vec![ir::WindowFn { kind, partition: vec![c(1)], order, total: ir::Total::Checked }] };
            if on { hoist_window_subqueries(&r).expect("R30 fires") } else { r }
        };
        let rn = |on: bool| win(WindowKind::RowNumber, vec![(cnt(), false), (c(0), true), (c(1), false)], on);
        let m = |on: bool| win(WindowKind::Max(mx()), vec![], on);
        assert_eq!(differential(0x5eed_30, 150, &[&rn, &m]), 2);
    }

    /// Ordered aggregates on the dataflow: `group_concat`/`json_group_array` over a sorted derived
    /// table, grouped and keyless, fold in the Sort's order; a tie between distinct rows is a
    /// runtime refusal exactly where the reference raises one.
    #[test]
    fn generated_ordered_aggregates() {
        let t = tabs();
        let b = &t.1;
        // Ordered by z descending only: within a group of equal x the order is total; across
        // rows of different x with equal z (the keyless array) it ties.
        let sorted = || proj(sort(proj(b.clone(), vec![c(0), c(1), c(1)]), vec![(2, true)], None), vec![c(0), c(1)]);
        let grouped = Rel::Aggregate { input: Box::new(sorted()), keys: vec![c(0)], aggs: vec![call(ir::AggFn::GroupConcat, Some(c(1)), ir::Total::Checked), call(ir::AggFn::CountStar, None, ir::Total::Proven)] };
        let grouped = proj(grouped, vec![c(0), c(2), c(3)]);
        let keyless = proj(agg(sorted(), vec![call(ir::AggFn::JsonGroupArray, Some(c(1)), ir::Total::Checked)]), vec![c(2)]);
        install();
        let mut rng = Rng(0x5eed_42);
        let (mut rows, mut ties) = (0, 0);
        for _ in 0..300 {
            let t = rng.tables();
            for q in [&grouped, &keyless] {
                agree_generated(q, &t);
                match reference(q, &t) {
                    Ok(m) => rows += m.len(),
                    Err(_) => ties += 1,
                }
            }
        }
        assert!(rows > 300 && ties > 10, "rows {rows}, ties {ties}");
    }

    /// R34: `e IN (list)` across an inner join without an equality: a union of keyed joins, each
    /// pair once (members repeated or equal on a pair included).
    #[test]
    fn rewrite_in_list_join_to_union() {
        let t = tabs();
        let (a, b) = (&t.0, &t.1);
        let split = |r: Rl, on: bool| if on { split_in_list_join(&r).expect("R34 fires") } else { r };
        let aff = (Aff::None, Aff::None);
        let inl = |e: X, xs: Vec<X>| Expr::InList { e: Box::new(e), list: xs.into_iter().map(|x| (x, aff)).collect(), negated: false };
        let q1 = move |on: bool| split(join(INNER, a.clone(), b.clone(), Some(and(inl(c(0), vec![c(2), c(3), Expr::Arith(ir::ArithOp::Add, Box::new(c(2)), Box::new(lit(1)))]), cmp(CmpOp::Ne, c(1), c(3))))), on);
        let q2 = move |on: bool| split(join(INNER, a.clone(), b.clone(), Some(inl(c(3), vec![c(0), c(0), c(1)]))), on);
        assert_eq!(differential(0x5eed_34, 150, &[&q1, &q2]), 2);
        // Not with an equality between the sides already (a keyed join), nor under a LEFT join.
        assert!(split_in_list_join(&join(INNER, a.clone(), b.clone(), Some(and(eq(c(0), c(2)), inl(c(1), vec![c(3)]))))).is_none());
        assert!(split_in_list_join(&join(LEFT, a.clone(), b.clone(), Some(inl(c(1), vec![c(3)])))).is_none());
    }

    /// LEFT JOIN on equalities only: unmatched left rows by an antijoin on the join's keys (so a
    /// LEFT JOIN of the recursive relation inside a recursion is stratified), against the
    /// reference, inside and outside a recursion, with NULL keys.
    #[test]
    fn generated_left_join_equi_antijoin() {
        let t = tabs();
        let (a, b) = (&t.0, &t.1);
        let plain = join(LEFT, a.clone(), b.clone(), Some(eq(c(0), c(2))));
        let local = Rel::Local { id: 7, schema: vec![Column::new("x", Aff::None)] };
        let step = proj(join(LEFT, local.clone(), b.clone(), Some(eq(c(0), c(1)))), vec![Expr::Func(ir::Func::Coalesce, vec![c(2), c(0)])]);
        let rec = Rel::With { bindings: vec![(7, Binding::Fix { init: vec![proj(a.clone(), vec![c(0)])], step: vec![step], set: true, schema: vec![Column::new("x", Aff::None)] })], body: Box::new(local) };
        install();
        let mut rng = Rng(0x5eed_46);
        for _ in 0..150 {
            let t = rng.tables();
            agree_generated(&plain, &t);
            agree_generated(&rec, &t);
        }
    }
}
