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
use rerail_facts::ir::{self, AggCall, Aff, Binding, CmpOp, Expr, Rel, RepoSource, Sink, StageDef, WindowFn, WindowKind};
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
    /// read: checked) ++ aggregates. `keyless`: one group even over no rows.
    Group { input: NodeId, keys: Key, aggs: Vec<AggCall<RepoSource>>, keyless: bool, width: usize },
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
}

pub fn col(c: usize) -> X {
    Expr::Col { depth: 0, col: c as u16 }
}

// --------------------------------------------------------------------------------------------
// Expression analysis

/// Does `e` read a row `k` or more subquery levels out (relative to the expression's operator)?
fn expr_free(e: &X, k: u16) -> bool {
    match e {
        Expr::Lit(_) => false,
        Expr::Col { depth, .. } => *depth >= k,
        Expr::Not(x) | Expr::Neg(x) | Expr::Plus(x) | Expr::Cast(x, _) => expr_free(x, k),
        Expr::IsNull { e, .. } => expr_free(e, k),
        Expr::Arith(_, a, b) | Expr::Concat(a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Cmp(_, a, b, _) => expr_free(a, k) || expr_free(b, k),
        Expr::Is { left, right, .. } => expr_free(left, k) || expr_free(right, k),
        Expr::Like { text, pattern, .. } => expr_free(text, k) || expr_free(pattern, k),
        Expr::Func(_, a) => a.iter().any(|x| expr_free(x, k)),
        Expr::Case { operand, whens, else_ } => operand.as_ref().is_some_and(|o| expr_free(o, k)) || whens.iter().any(|(w, t, _)| expr_free(w, k) || expr_free(t, k)) || else_.as_ref().is_some_and(|x| expr_free(x, k)),
        Expr::InList { e, list, .. } => expr_free(e, k) || list.iter().any(|(x, _)| expr_free(x, k)),
        Expr::InRel { e, rel, .. } => expr_free(e, k) || rel_free(rel, k + 1),
        Expr::Exists { rel, .. } => rel_free(rel, k + 1),
        Expr::Scalar(rel) => rel_free(rel, k + 1),
    }
}

/// Does a relation's expressions read a row `k` or more levels out (k = 1: the row enclosing the
/// subquery the relation is)?
fn rel_free(r: &Rel<RepoSource>, k: u16) -> bool {
    match r {
        Rel::Scan { .. } | Rel::Local { .. } | Rel::Unit => false,
        Rel::JsonEach { input, arg, on, .. } => rel_free(input, k) || expr_free(arg, k) || on.as_ref().is_some_and(|o| expr_free(o, k)),
        Rel::Join { left, right, on, .. } => rel_free(left, k) || rel_free(right, k) || on.as_ref().is_some_and(|o| expr_free(o, k)),
        Rel::Filter { input, pred } => rel_free(input, k) || expr_free(pred, k),
        Rel::Project { input, exprs, .. } => rel_free(input, k) || exprs.iter().any(|e| expr_free(e, k)),
        Rel::Aggregate { input, keys, aggs } => rel_free(input, k) || keys.iter().any(|e| expr_free(e, k)) || aggs.iter().any(|a| a.arg.as_ref().is_some_and(|e| expr_free(e, k)) || a.sep.as_ref().is_some_and(|e| expr_free(e, k))),
        Rel::Window { input, funcs } => rel_free(input, k) || funcs.iter().any(|f| f.partition.iter().any(|e| expr_free(e, k)) || f.order.iter().any(|(e, _)| expr_free(e, k)) || matches!(&f.kind, WindowKind::Min(e) | WindowKind::Max(e) if expr_free(e, k))),
        Rel::Distinct { input, .. } => rel_free(input, k),
        Rel::Union { left, right, .. } => rel_free(left, k) || rel_free(right, k),
        Rel::Sort { input, limit, offset, .. } => rel_free(input, k) || limit.as_ref().is_some_and(|e| expr_free(e, k)) || offset.as_ref().is_some_and(|e| expr_free(e, k)),
        Rel::With { bindings, body } => {
            rel_free(body, k)
                || bindings.iter().any(|(_, b)| match b {
                    Binding::Plain(r) => rel_free(r, k),
                    Binding::Fix { init, step, .. } => init.iter().chain(step).any(|r| rel_free(r, k)),
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
    fn add(&mut self, op: Op, width: usize, poison: Vec<usize>) -> NodeId {
        self.g.nodes.push(op);
        self.g.widths.push(width);
        self.g.poison.push(poison);
        self.g.nodes.len() - 1
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
                let l = self.rel(left)?;
                let r = self.rel(right)?;
                self.join(l, r, on.as_ref(), *kind == ir::JoinKind::Left)
            }
            Rel::Aggregate { input, keys, aggs } => {
                let n = self.rel(input)?;
                self.group(n, keys.clone(), aggs.clone())
            }
            Rel::Window { input, funcs } => {
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
            Rel::Sort { input, keys, limit, offset } => {
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

    fn group(&mut self, n: NodeId, keys: Vec<X>, aggs: Vec<AggCall<RepoSource>>) -> R<NodeId> {
        let w = self.width(n);
        for e in keys.iter().chain(aggs.iter().filter_map(|a| a.arg.as_ref())).chain(aggs.iter().filter_map(|a| a.sep.as_ref())) {
            if has_subquery(e) {
                return refuse(&self.at, "subquery in GROUP BY or an aggregate argument");
            }
            self.check_reads(n, e)?;
        }
        for a in &aggs {
            if matches!(a.f, ir::AggFn::GroupConcat | ir::AggFn::JsonGroupArray) {
                // Input order is observed: the lowering requires the group's rows to be totally
                // ordered by an enclosing Sort it can see; it cannot here.
                return refuse(&self.at, format!("{:?} over a repository relation: aggregates in input order (no total order in the dataflow)", a.f));
            }
        }
        // Representative row: only the key columns are defined (a key that is a plain column).
        let key_cols: Vec<usize> = keys.iter().filter_map(|k| if let Expr::Col { depth: 0, col } = k { Some(*col as usize) } else { None }).collect();
        let poison: Vec<usize> = (0..w).filter(|c| !key_cols.contains(c)).collect();
        let keyless = keys.is_empty();
        let keys: Key = keys.into_iter().map(|e| (e, KeyMode::Group)).collect();
        Ok(self.add(Op::Group { input: n, keys, aggs: aggs.clone(), keyless, width: w }, w + aggs.len(), poison))
    }

    /// Group by correlation keys compared with `=` under their affinities (scalar subqueries).
    fn group_eq(&mut self, n: NodeId, keys: Key, aggs: Vec<AggCall<RepoSource>>) -> R<NodeId> {
        let w = self.width(n);
        let g = self.group(n, Vec::new(), aggs)?;
        let Op::Group { aggs, .. } = std::mem::replace(&mut self.g.nodes[g], Op::Const(Vec::new())) else { unreachable!() };
        let poison: Vec<usize> = (0..w).filter(|c| !keys.iter().any(|(k, _)| matches!(k, Expr::Col { depth: 0, col } if *col as usize == *c))).collect();
        self.g.nodes[g] = Op::Group { input: n, keyless: keys.is_empty(), keys, aggs, width: w };
        self.g.poison[g] = poison;
        Ok(g)
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
        // LEFT JOIN: matched rows ∪ left rows with no match, NULL-extended. A left row matched
        // once is matched for every copy (same row, same matches), so "no match" is an antijoin
        // of the left side against the matched left parts by row identity.
        if !self.g.poison[l].is_empty() {
            return refuse(&self.at, "LEFT JOIN over bare aggregate columns");
        }
        let lpart = self.add(Op::Map { input: inner, exprs: (0..wl).map(col).collect() }, wl, Vec::new());
        let ident: Key = (0..wl).map(|c| (col(c), KeyMode::Group)).collect();
        let unmatched = self.add(Op::Semi { l, r: lpart, lk: ident.clone(), rk: ident, anti: true }, wl, Vec::new());
        let ext = self.add(Op::Map { input: unmatched, exprs: (0..wl).map(col).chain((0..wr).map(|_| Expr::Lit(ir::Lit::Null))).collect() }, wl + wr, Vec::new());
        Ok(self.add(Op::Concat { inputs: vec![inner, ext] }, wl + wr, poison))
    }

    fn filter(&mut self, mut n: NodeId, pred: &X) -> R<NodeId> {
        let mut cs = Vec::new();
        conjuncts(pred, &mut cs);
        let mut plain = Vec::new();
        for c in cs {
            let (c, neg) = match c {
                Expr::Not(x) if matches!(*x, Expr::Exists { .. } | Expr::InRel { .. }) => (*x, true),
                x => (x, false),
            };
            match c {
                Expr::Exists { rel, negated } => n = self.semi(n, &rel, None, negated != neg)?,
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

    /// Split a subquery body into its uncorrelated part and the equality correlations with the
    /// enclosing row: `(body, [(outer expr at depth 0 of the enclosing row, inner expr, affs)],
    /// projection of the first output column)`.
    fn decorrelate(&mut self, rel: &Rel<RepoSource>) -> R<(Rel<RepoSource>, Vec<(X, X, (Aff, Aff))>, Option<X>)> {
        let mut first: Option<X> = None;
        let mut cur = rel.clone();
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
                // e NOT IN s is true iff e is not NULL, no s row equals e, and s holds no NULL.
                let notnull = self.add(Op::Filter { input: n, pred: Expr::IsNull { e: Box::new(e), negated: true } }, w, p.clone());
                let a = self.add(Op::Semi { l: notnull, r: b, lk, rk, anti: true }, w, p.clone());
                let nulls = self.add(Op::Filter { input: b, pred: Expr::IsNull { e: Box::new(f), negated: false } }, self.width(b), self.g.poison[b].clone());
                Ok(self.add(Op::Semi { l: a, r: nulls, lk: corr_l, rk: corr_r, anti: true }, w, p))
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
            Expr::InRel { .. } | Expr::Exists { .. } => return refuse(&self.at, "EXISTS/IN under OR, NOT or CASE, or in a select list"),
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
                let (body, corr, _) = self.decorrelate(input)?;
                let b = self.rel(&body)?;
                let wb = self.width(b);
                // Group by the inner correlation expressions; the group row is
                // corr values ++ representative (poisoned) ++ aggs.
                let ck: Vec<X> = corr.iter().map(|(_, i, _)| i.clone()).collect();
                for k in &ck {
                    self.check_reads(b, k)?;
                }
                let keyed = self.add(Op::Map { input: b, exprs: ck.iter().cloned().chain((0..wb).map(col)).collect() }, ck.len() + wb, self.g.poison[b].iter().map(|c| c + ck.len()).collect());
                let gk: Key = corr.iter().enumerate().map(|(i, (_, _, (_, fi)))| (col(i), KeyMode::Eq(*fi))).collect();
                let aggs2: Vec<AggCall<RepoSource>> = aggs.iter().map(|a| AggCall { f: a.f, arg: a.arg.as_ref().map(|e| shift(e, ck.len() as isize)), sep: a.sep.as_ref().map(|e| shift(e, ck.len() as isize)), distinct: a.distinct }).collect();
                let g = self.group_eq(keyed, gk, aggs2)?;
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
                // The empty group's value (no correlated rows): the aggregate over no rows.
                let empty_row: Vec<V> = std::iter::repeat_n(V::Null, wb).chain(aggs.iter().map(|a| match a.f {
                    ir::AggFn::Count | ir::AggFn::CountStar => V::Int(0),
                    ir::AggFn::Total => V::Real(0.0),
                    _ => V::Null,
                })).collect();
                let mut ev = proj.clone().unwrap_or_else(|| col(wb));
                if let Some(h) = &having {
                    ev = Expr::Case { operand: None, whens: vec![(h.clone(), ev, (Aff::None, Aff::None))], else_: None };
                }
                let empty = super::scalar::eval(&ev, &empty_row, &[]).map_err(|m| Refused { at: self.at.clone(), construct: format!("scalar subquery empty value: {m}") })?;
                self.attach(n, wn, pn, vals, &corr, empty)
            }
            Rel::Sort { input, keys, limit: Some(l), offset: None } if matches!(super::scalar::eval(l, &[], &[]), Ok(V::Int(1))) => {
                // (SELECT first FROM (SELECT sel…, keys… FROM corr-filtered) ORDER BY keys LIMIT 1):
                // per correlation key, the row first in the (checked total) order.
                let (sel, inner) = match &**input {
                    Rel::Project { input, exprs, .. } => (exprs.clone(), &**input),
                    other => ((0..rel_width(other, self)).map(col).collect(), other),
                };
                if sel.iter().any(|e| expr_free(e, 1) || has_subquery(e)) {
                    return refuse(&self.at, "scalar subquery projecting an outer column or a subquery");
                }
                let (body, corr, _) = self.decorrelate(inner)?;
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
    let mut ctx = Ctx { g: Graph::default(), stages: Vec::new(), exported, locals: Vec::new(), at: String::new() };
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
                ctx.stages.push(usize::MAX);
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
                ctx.g.outputs.push((p.clause.clone(), n, names))
            }
            Err(e) => refused.push(e),
        },
        Sink::Budget { .. } => refused.push(Refused { at: ctx.at.clone(), construct: "budget sink in a repository plan (cross-file closure/group): not lowered yet".into() }),
        Sink::Witnesses { .. } | Sink::Export(_) => refused.push(Refused { at: ctx.at.clone(), construct: "per-file sink in a repository plan".into() }),
    }
    if refused.is_empty() { Ok(ctx.g) } else { Err(refused) }
}
