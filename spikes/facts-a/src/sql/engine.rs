//! Planner and executor for the SQLite SELECT subset. Plans are fixed by the SQL text: joins run
//! in FROM order as index nested loops; equality conjuncts between a FROM item's column and
//! earlier items (or outer rows, or constants) probe a hash index built once per table; every
//! other conjunct is applied at the earliest level where its columns are bound. Correlated
//! subqueries probe the same indexes; uncorrelated ones run once per statement execution.
use super::parse::{self, Body, Expr, JoinKind, Query, SelItem, SetOp, Source};
use super::value::{self as sv, Aff, DKey, Key, V};
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::rc::Rc;

#[derive(Clone, Debug, Default)]
pub struct Schema {
    pub cols: Vec<String>,
    pub affs: Vec<Aff>,
}
impl Schema {
    fn find(&self, name: &str) -> Option<usize> {
        self.cols.iter().position(|c| c.eq_ignore_ascii_case(name))
    }
}

#[derive(Debug, Default)]
pub struct Table {
    pub schema: Schema,
    pub rows: Vec<Box<[V]>>,
}

// ---------------------------------------------------------------------------------------------
// Plan

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Func {
    Coalesce,
    Ifnull,
    Length,
    Lower,
    Upper,
    Trim,
    Ltrim,
    Rtrim,
    Substr,
    Instr,
    Replace,
    Char,
    JsonQuote,
    JsonExtract,
    RxMatches,
    RxExtract,
    RxReplace,
    SourceText,
    Abs,
    Typeof,
    Max,
    Min,
    Iif,
    Nullif,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AggFn {
    Count,
    CountStar,
    Min,
    Max,
    GroupConcat,
    Sum,
    Total,
}

#[derive(Clone, Debug)]
pub enum PE {
    Lit(V),
    Col { depth: u16, item: u16, col: u16 },
    Agg(u16),
    Win(u16),
    Not(Box<PE>),
    Neg(Box<PE>),
    Cmp(Cmp, Box<PE>, Box<PE>, (Aff, Aff)),
    Arith(char, Box<PE>, Box<PE>),
    Concat(Box<PE>, Box<PE>),
    And(Box<PE>, Box<PE>),
    Or(Box<PE>, Box<PE>),
    Func(Func, Vec<PE>),
    Case { operand: Option<Box<PE>>, aff: (Aff, Aff), whens: Vec<(PE, PE)>, else_: Option<Box<PE>> },
    Cast(Box<PE>, Aff),
    InList { e: Box<PE>, list: Vec<(PE, (Aff, Aff))>, not: bool },
    InSub { e: Box<PE>, sub: usize, not: bool, aff: (Aff, Aff) },
    Exists { sub: usize, not: bool },
    Scalar(usize),
    IsNull(Box<PE>, bool),
    Is(Box<PE>, Box<PE>, bool),
    Like(Box<PE>, Box<PE>, bool),
}

#[derive(Clone, Debug)]
enum Src {
    Global(String),
    Cte(usize),
    Derived(usize),
    JsonEach(PE),
}

#[derive(Clone, Debug)]
struct PItem {
    src: Src,
    width: usize,
    left: bool,
    /// A table function whose argument reads earlier items: evaluated per binding.
    lateral: bool,
}

#[derive(Clone, Debug)]
struct Probe {
    cols: Vec<u16>,
    col_affs: Vec<Aff>,
    exprs: Vec<PE>,
    expr_affs: Vec<Aff>,
}

#[derive(Clone, Debug)]
struct AggSpec {
    f: AggFn,
    arg: Option<PE>,
    sep: Option<PE>,
    distinct: bool,
}

#[derive(Clone, Debug)]
struct WinSpec {
    partition: Vec<PE>,
    order: Vec<(PE, bool)>,
}

#[derive(Clone, Debug)]
struct PSelect {
    items: Vec<PItem>,
    probes: Vec<Option<Probe>>,
    /// Per item, the branch probes of an OR conjunct (candidates = union of the lookups).
    or_probes: Vec<Option<Vec<Probe>>>,
    /// Conjuncts tested once all of items[..=i] are bound (inner items; ON of left items).
    level: Vec<Vec<PE>>,
    /// WHERE conjuncts whose last item is a left-joined item i (after null extension).
    post: Vec<Vec<PE>>,
    /// Conjuncts with no item reference.
    konst: Vec<PE>,
    grouped: bool,
    group_by: Vec<PE>,
    aggs: Vec<AggSpec>,
    having: Option<PE>,
    wins: Vec<WinSpec>,
    proj: Vec<PE>,
    /// ORDER BY keys of a simple query, evaluated at projection time.
    order_keys: Vec<PE>,
    distinct: bool,
    out: Schema,
}

#[derive(Clone, Debug)]
enum PBody {
    Select(Box<PSelect>),
    Compound(Box<PBody>, SetOp, Box<PBody>, Schema),
}
impl PBody {
    fn schema(&self) -> &Schema {
        match self {
            PBody::Select(s) => &s.out,
            PBody::Compound(_, _, _, s) => s,
        }
    }
}

#[derive(Clone, Debug)]
enum PCte {
    Plain { slot: usize, q: PQuery },
    Recursive { slot: usize, batch: usize, init: Vec<PBody>, rec: Vec<PBody>, distinct: bool, schema: Schema },
}

#[derive(Clone, Debug)]
pub struct PQuery {
    ctes: Vec<PCte>,
    body: PBody,
    /// Sort keys: for a simple select, indices into the extra keys its projection appends;
    /// for a compound, output column indices.
    order: Vec<(usize, bool)>,
    order_on_output: bool,
    limit: Option<PE>,
    offset: Option<PE>,
    pub out: Schema,
}

#[derive(Clone, Debug)]
struct SubPlan {
    q: PQuery,
    cacheable: bool,
}

#[derive(Clone, Debug)]
pub struct Statement {
    root: PQuery,
    subs: Vec<SubPlan>,
    n_slots: usize,
    pub reads: Vec<String>,
}
impl Statement {
    pub fn schema(&self) -> &Schema {
        &self.root.out
    }
}

pub type Catalog = HashMap<String, Schema>;

struct ScopeItem {
    name: String,
    schema: Schema,
}
struct Scope {
    items: Vec<ScopeItem>,
    /// Result aliases of the select (for alias references in WHERE/GROUP BY/HAVING/ORDER BY).
    aliases: Vec<(String, Expr)>,
    /// Aggregates and windows collected while planning this select's projection.
    aggs: Vec<AggSpec>,
    wins: Vec<WinSpec>,
    allow_agg: bool,
}

struct SubBound {
    scope_depth: usize,
    correlated: bool,
    recursive_ref: bool,
}

pub struct Planner<'c> {
    catalog: &'c Catalog,
    scopes: Vec<Scope>,
    ctes: Vec<(String, usize, Schema, bool)>,
    subs: Vec<SubPlan>,
    n_slots: usize,
    bounds: Vec<SubBound>,
    reads: Vec<String>,
}

fn lc(s: &str) -> String {
    s.to_ascii_lowercase()
}

fn conjuncts(e: Expr, out: &mut Vec<Expr>) {
    match e {
        Expr::Bin("and", l, r) => {
            conjuncts(*l, out);
            conjuncts(*r, out);
        }
        o => out.push(o),
    }
}

fn disjuncts<'e>(e: &'e PE, out: &mut Vec<&'e PE>) {
    match e {
        PE::Or(l, r) => {
            disjuncts(l, out);
            disjuncts(r, out);
        }
        o => out.push(o),
    }
}

fn pe_conjuncts<'e>(e: &'e PE, out: &mut Vec<&'e PE>) {
    match e {
        PE::And(l, r) => {
            pe_conjuncts(l, out);
            pe_conjuncts(r, out);
        }
        o => out.push(o),
    }
}

/// Adds `c` to `probe` when it is an equality between a column of item `i` and an expression
/// over earlier items, outer rows or constants.
fn probe_eq(c: &PE, i: usize, probe: &mut Probe) -> bool {
    let PE::Cmp(Cmp::Eq, l, r, (al, ar)) = c else { return false };
    let pick = |a: &PE, b: &PE| -> Option<u16> {
        if let PE::Col { depth: 0, item, col } = a {
            if *item as usize == i && max_item(b).is_none_or(|m| m < i) && !refs_item(b, i) && !has_agg(b) {
                return Some(*col);
            }
        }
        None
    };
    if let Some(col) = pick(l, r) {
        probe.cols.push(col);
        probe.col_affs.push(*al);
        probe.exprs.push((**r).clone());
        probe.expr_affs.push(*ar);
        return true;
    }
    if let Some(col) = pick(r, l) {
        probe.cols.push(col);
        probe.col_affs.push(*ar);
        probe.exprs.push((**l).clone());
        probe.expr_affs.push(*al);
        return true;
    }
    false
}

pub fn plan(sql: &str, catalog: &Catalog) -> Result<Statement, String> {
    let q = parse::parse_query(sql)?;
    let mut p = Planner { catalog, scopes: Vec::new(), ctes: Vec::new(), subs: Vec::new(), n_slots: 0, bounds: Vec::new(), reads: Vec::new() };
    let root = p.query(&q)?;
    let mut seen = std::collections::BTreeSet::new();
    let reads = p.reads.into_iter().filter(|r| seen.insert(r.clone())).collect();
    Ok(Statement { root, subs: p.subs, n_slots: p.n_slots, reads })
}

/// Plans a scalar expression over one table's columns (a `deferred` column).
pub fn plan_scalar(sql: &str, schema: &Schema) -> Result<(PE, Statement), String> {
    let e = parse::parse_expr(sql)?;
    let catalog = Catalog::new();
    let mut p = Planner { catalog: &catalog, scopes: Vec::new(), ctes: Vec::new(), subs: Vec::new(), n_slots: 0, bounds: Vec::new(), reads: Vec::new() };
    p.scopes.push(Scope { items: vec![ScopeItem { name: "__row".into(), schema: schema.clone() }], aliases: Vec::new(), aggs: Vec::new(), wins: Vec::new(), allow_agg: false });
    let pe = p.expr(&e)?;
    let stmt = Statement { root: PQuery { ctes: vec![], body: PBody::Compound(Box::new(PBody::Select(Box::new(empty_select()))), SetOp::UnionAll, Box::new(PBody::Select(Box::new(empty_select()))), Schema::default()), order: vec![], order_on_output: false, limit: None, offset: None, out: Schema::default() }, subs: p.subs, n_slots: p.n_slots, reads: p.reads };
    Ok((pe, stmt))
}

fn empty_select() -> PSelect {
    PSelect { items: vec![], probes: vec![], or_probes: vec![], level: vec![], post: vec![], konst: vec![], grouped: false, group_by: vec![], aggs: vec![], having: None, wins: vec![], proj: vec![], order_keys: vec![], distinct: false, out: Schema::default() }
}

impl<'c> Planner<'c> {
    fn new_slot(&mut self) -> usize {
        self.n_slots += 1;
        self.n_slots - 1
    }

    fn query(&mut self, q: &Query) -> Result<PQuery, String> {
        let cte_mark = self.ctes.len();
        let mut ctes = Vec::new();
        for cte in &q.ctes {
            let self_ref = q.recursive && body_refs(&cte.q.body, &cte.name);
            if self_ref {
                ctes.push(self.recursive_cte(cte)?);
            } else {
                let pq = self.query(&cte.q)?;
                let mut schema = pq.out.clone();
                if let Some(cols) = &cte.cols {
                    if cols.len() != schema.cols.len() {
                        return Err(format!("CTE {} column count", cte.name));
                    }
                    schema.cols = cols.clone();
                }
                let slot = self.new_slot();
                self.ctes.push((lc(&cte.name), slot, schema, false));
                ctes.push(PCte::Plain { slot, q: pq });
            }
        }
        let body = self.body(&q.body, Some(q))?;
        let out = body.schema().clone();
        // ORDER BY: a simple select's keys were appended by `body`; a compound's resolve against
        // its output columns.
        let (order, order_on_output) = match &body {
            PBody::Select(s) => ((0..q.order.len()).map(|i| (s.out.cols.len() + i, q.order[i].1)).collect(), false),
            PBody::Compound(..) => {
                let mut v = Vec::new();
                for (e, desc) in &q.order {
                    let idx = match e {
                        Expr::Col { table: None, name } => out.find(name).ok_or_else(|| format!("ORDER BY {name}"))?,
                        Expr::Lit(V::Int(i)) => (*i as usize).checked_sub(1).ok_or("ORDER BY 0")?,
                        _ => return Err("compound ORDER BY expression unsupported".into()),
                    };
                    v.push((idx, *desc));
                }
                (v, true)
            }
        };
        let limit = q.limit.as_ref().map(|e| self.const_expr(e)).transpose()?;
        let offset = q.offset.as_ref().map(|e| self.const_expr(e)).transpose()?;
        self.ctes.truncate(cte_mark);
        Ok(PQuery { ctes, body, order, order_on_output, limit, offset, out })
    }

    fn const_expr(&mut self, e: &Expr) -> Result<PE, String> {
        self.scopes.push(Scope { items: vec![], aliases: vec![], aggs: vec![], wins: vec![], allow_agg: false });
        let r = self.expr(e);
        self.scopes.pop();
        r
    }

    fn recursive_cte(&mut self, cte: &parse::Cte) -> Result<PCte, String> {
        if !cte.q.order.is_empty() || cte.q.limit.is_some() || !cte.q.ctes.is_empty() {
            return Err("recursive CTE with ORDER BY/LIMIT/WITH unsupported".into());
        }
        let mut arms = Vec::new();
        let mut ops = Vec::new();
        flatten(&cte.q.body, &mut arms, &mut ops);
        let first_rec = arms.iter().position(|a| body_refs(a, &cte.name)).ok_or("recursive CTE without recursive arm")?;
        if first_rec == 0 {
            return Err("recursive CTE must start with a non-recursive arm".into());
        }
        let distinct = match ops[first_rec - 1] {
            SetOp::Union => true,
            SetOp::UnionAll => false,
        };
        if ops.iter().any(|o| *o != ops[first_rec - 1]) {
            return Err("recursive CTE mixing UNION and UNION ALL unsupported".into());
        }
        let mut init = Vec::new();
        for a in &arms[..first_rec] {
            init.push(self.body(a, None)?);
        }
        let mut schema = init[0].schema().clone();
        if let Some(cols) = &cte.cols {
            if cols.len() != schema.cols.len() {
                return Err("recursive CTE column count".into());
            }
            schema.cols = cols.clone();
        }
        let slot = self.new_slot();
        let batch = self.new_slot();
        self.ctes.push((lc(&cte.name), batch, schema.clone(), true));
        let mut rec = Vec::new();
        for a in &arms[first_rec..] {
            if !body_refs(a, &cte.name) {
                return Err("non-recursive arm after a recursive arm".into());
            }
            rec.push(self.body(a, None)?);
        }
        self.ctes.pop();
        self.ctes.push((lc(&cte.name), slot, schema.clone(), false));
        Ok(PCte::Recursive { slot, batch, init, rec, distinct, schema })
    }

    fn body(&mut self, b: &Body, order_of: Option<&Query>) -> Result<PBody, String> {
        match b {
            Body::Select(s) => {
                let order: &[(Expr, bool)] = order_of.map_or(&[], |q| &q.order);
                Ok(PBody::Select(Box::new(self.select(s, order)?)))
            }
            Body::Compound(l, op, r) => {
                let pl = self.body(l, None)?;
                let pr = self.body(r, None)?;
                if pl.schema().cols.len() != pr.schema().cols.len() {
                    return Err("compound arms differ in width".into());
                }
                // SQLite: a compound's column names come from its left-most arm; its affinity is
                // the left-most arm's.
                let schema = pl.schema().clone();
                Ok(PBody::Compound(Box::new(pl), *op, Box::new(pr), schema))
            }
        }
    }

    fn source_schema(&mut self, src: &Source) -> Result<(Src, Schema, String), String> {
        match src {
            Source::Table(name) => {
                let l = lc(name);
                if let Some((_, slot, schema, rec)) = self.ctes.iter().rev().find(|c| c.0 == l) {
                    if *rec {
                        if let Some(b) = self.bounds.last_mut() {
                            b.recursive_ref = true;
                        }
                        for b in self.bounds.iter_mut() {
                            b.recursive_ref = true;
                        }
                    }
                    return Ok((Src::Cte(*slot), schema.clone(), name.clone()));
                }
                let schema = self.catalog.get(&l).ok_or_else(|| format!("no such table: {name}"))?.clone();
                self.reads.push(l);
                Ok((Src::Global(name.clone()), schema, name.clone()))
            }
            Source::Func(f, args) => {
                if !f.eq_ignore_ascii_case("json_each") || args.len() != 1 {
                    return Err(format!("table function {f} unsupported"));
                }
                let arg = self.expr(&args[0])?;
                let schema = Schema {
                    cols: ["key", "value", "type", "atom", "id", "parent", "fullkey", "path"].iter().map(|s| s.to_string()).collect(),
                    affs: vec![Aff::None; 8],
                };
                Ok((Src::JsonEach(arg), schema, f.clone()))
            }
            Source::Sub(q) => {
                let id = self.subquery(q)?;
                let schema = self.subs[id].q.out.clone();
                Ok((Src::Derived(id), schema, String::new()))
            }
        }
    }

    fn subquery(&mut self, q: &Query) -> Result<usize, String> {
        self.bounds.push(SubBound { scope_depth: self.scopes.len(), correlated: false, recursive_ref: false });
        let pq = self.query(q);
        let b = self.bounds.pop().unwrap();
        let pq = pq?;
        if b.recursive_ref {
            for o in self.bounds.iter_mut() {
                o.recursive_ref = true;
            }
        }
        self.subs.push(SubPlan { q: pq, cacheable: !b.correlated && !b.recursive_ref });
        Ok(self.subs.len() - 1)
    }

    fn select(&mut self, s: &parse::Select, order: &[(Expr, bool)]) -> Result<PSelect, String> {
        // FROM items are resolved in the enclosing scope first (derived tables cannot see this
        // select's own items), then pushed one by one.
        self.scopes.push(Scope { items: Vec::new(), aliases: Vec::new(), aggs: Vec::new(), wins: Vec::new(), allow_agg: false });
        let mut items = Vec::new();
        for fi in &s.from {
            let (src, schema, name) = self.source_schema(&fi.source)?;
            let alias = fi.alias.clone().unwrap_or(name);
            let lateral = matches!(&src, Src::JsonEach(arg) if max_item(arg).is_some());
            items.push(PItem { src, width: schema.cols.len(), left: fi.kind == JoinKind::Left, lateral });
            self.scopes.last_mut().unwrap().items.push(ScopeItem { name: lc(&alias), schema });
        }
        // Result aliases.
        let mut aliases = Vec::new();
        for it in &s.items {
            if let SelItem::Expr { e, alias: Some(a), .. } = it {
                aliases.push((lc(a), e.clone()));
            }
        }
        self.scopes.last_mut().unwrap().aliases = aliases;
        let n = items.len();
        let mut level: Vec<Vec<PE>> = vec![Vec::new(); n];
        let mut post: Vec<Vec<PE>> = vec![Vec::new(); n];
        let mut konst = Vec::new();
        let mut probes: Vec<Option<Probe>> = vec![None; n];
        let mut or_probes: Vec<Option<Vec<Probe>>> = vec![None; n];
        // Conjunct placement.
        let mut placed: Vec<(PE, Option<usize>, bool)> = Vec::new(); // (expr, max item, from ON of a left item)
        for (i, fi) in s.from.iter().enumerate() {
            if let Some(on) = &fi.on {
                let mut cs = Vec::new();
                conjuncts(on.clone(), &mut cs);
                for c in cs {
                    let pe = self.expr(&c)?;
                    if fi.kind == JoinKind::Left {
                        placed.push((pe, Some(i), true));
                    } else {
                        let m = max_item(&pe);
                        placed.push((pe, m, false));
                    }
                }
            }
        }
        if let Some(w) = &s.where_ {
            let mut cs = Vec::new();
            conjuncts(w.clone(), &mut cs);
            for c in cs {
                let pe = self.expr(&c)?;
                let m = max_item(&pe);
                placed.push((pe, m, false));
            }
        }
        for (pe, m, left_on) in placed {
            // A conjunct holding a subquery may read any item: it runs at the last one.
            let m = m.map(|i| i.min(n.saturating_sub(1))).filter(|_| n > 0);
            match m {
                None => konst.push(pe),
                Some(i) if left_on => level[i].push(pe),
                Some(i) if items[i].left => post[i].push(pe),
                Some(i) => level[i].push(pe),
            }
        }
        // Probes: equality between a column of item i and an expression over earlier items,
        // outer rows or constants.
        for i in 0..n {
            if items[i].lateral {
                continue;
            }
            let mut probe = Probe { cols: vec![], col_affs: vec![], exprs: vec![], expr_affs: vec![] };
            let mut rest = Vec::new();
            for c in std::mem::take(&mut level[i]) {
                if !probe_eq(&c, i, &mut probe) {
                    rest.push(c);
                }
            }
            // OR probes (SQLite's OR optimisation): a conjunct `b1 OR b2 OR ...` whose every
            // branch holds such an equality. Candidates are the union of the branches' lookups,
            // each with the plain equalities above too, in table order; the OR conjunct stays in
            // `level[i]` and is re-checked, so the rows are exactly the filtered scan's.
            or_probes[i] = rest.iter().find_map(|c| {
                let mut branches = Vec::new();
                disjuncts(c, &mut branches);
                if branches.len() < 2 {
                    return None;
                }
                branches
                    .into_iter()
                    .map(|b| {
                        let mut cs = Vec::new();
                        pe_conjuncts(b, &mut cs);
                        let mut bp = probe.clone();
                        let found = cs.into_iter().fold(false, |found, c| probe_eq(c, i, &mut bp) || found);
                        found.then_some(bp)
                    })
                    .collect::<Option<Vec<Probe>>>()
            });
            level[i] = rest;
            if !probe.cols.is_empty() {
                probes[i] = Some(probe);
            }
        }
        // Projection.
        self.scopes.last_mut().unwrap().allow_agg = true;
        let mut proj = Vec::new();
        let mut out = Schema::default();
        let n_items = self.scopes.last().unwrap().items.len();
        for it in &s.items {
            match it {
                SelItem::Star => {
                    for ii in 0..n_items {
                        let sc = self.scopes.last().unwrap().items[ii].schema.clone();
                        for (ci, c) in sc.cols.iter().enumerate() {
                            proj.push(PE::Col { depth: 0, item: ii as u16, col: ci as u16 });
                            out.cols.push(c.clone());
                            out.affs.push(sc.affs[ci]);
                        }
                    }
                }
                SelItem::TableStar(t) => {
                    let tl = lc(t);
                    let ii = self.scopes.last().unwrap().items.iter().position(|x| x.name == tl).ok_or_else(|| format!("no such table: {t}"))?;
                    let sc = self.scopes.last().unwrap().items[ii].schema.clone();
                    for (ci, c) in sc.cols.iter().enumerate() {
                        proj.push(PE::Col { depth: 0, item: ii as u16, col: ci as u16 });
                        out.cols.push(c.clone());
                        out.affs.push(sc.affs[ci]);
                    }
                }
                SelItem::Expr { e, alias, text } => {
                    let pe = self.expr(e)?;
                    let name = match (alias, e) {
                        (Some(a), _) => a.clone(),
                        (None, Expr::Col { name, .. }) => {
                            // The source column's declared name.
                            match &pe {
                                PE::Col { depth, item, col } => self.col_name(*depth, *item, *col).unwrap_or(name.clone()),
                                _ => name.clone(),
                            }
                        }
                        (None, _) => text.clone(),
                    };
                    out.affs.push(self.affinity(&pe));
                    out.cols.push(name);
                    proj.push(pe);
                }
            }
        }
        let group_by = s.group_by.iter().map(|g| self.expr(g)).collect::<Result<Vec<_>, _>>()?;
        let having = s.having.as_ref().map(|h| self.expr(h)).transpose()?;
        let mut order_keys = Vec::new();
        for (e, _) in order {
            // A bare result alias or ordinal sorts by that output column.
            let pe = match e {
                Expr::Lit(V::Int(i)) => proj.get((*i as usize).wrapping_sub(1)).cloned().ok_or("ORDER BY ordinal")?,
                Expr::Col { table: None, name } if out.find(name).is_some() && self.resolve(None, name).is_err() => proj[out.find(name).unwrap()].clone(),
                _ => self.expr(e)?,
            };
            order_keys.push(pe);
        }
        let scope = self.scopes.pop().unwrap();
        let grouped = !group_by.is_empty() || !scope.aggs.is_empty();
        Ok(PSelect { items, probes, or_probes, level, post, konst, grouped, group_by, aggs: scope.aggs, having, wins: scope.wins, proj, order_keys, distinct: s.distinct, out })
    }

    fn col_name(&self, depth: u16, item: u16, col: u16) -> Option<String> {
        let s = self.scopes.len().checked_sub(1 + depth as usize)?;
        Some(self.scopes[s].items[item as usize].schema.cols[col as usize].clone())
    }

    fn affinity(&self, pe: &PE) -> Aff {
        match pe {
            PE::Col { depth, item, col } => {
                let s = self.scopes.len() - 1 - *depth as usize;
                self.scopes[s].items[*item as usize].schema.affs[*col as usize]
            }
            PE::Cast(_, a) => *a,
            PE::Scalar(id) => self.subs[*id].q.out.affs.first().copied().unwrap_or(Aff::None),
            _ => Aff::None,
        }
    }

    fn resolve(&mut self, table: Option<&str>, name: &str) -> Result<PE, String> {
        let n = self.scopes.len();
        for d in 0..n {
            let s = n - 1 - d;
            let found = match table {
                Some(t) => {
                    let tl = lc(t);
                    self.scopes[s].items.iter().enumerate().find(|(_, it)| it.name == tl).map(|(i, it)| (i, it.schema.find(name)))
                }
                None => self.scopes[s].items.iter().enumerate().find_map(|(i, it)| it.schema.find(name).map(|c| (i, Some(c)))),
            };
            if let Some((item, col)) = found {
                let col = col.ok_or_else(|| format!("no such column: {}.{name}", table.unwrap_or("")))?;
                for b in self.bounds.iter_mut() {
                    if b.scope_depth > s {
                        b.correlated = true;
                    }
                }
                return Ok(PE::Col { depth: d as u16, item: item as u16, col: col as u16 });
            }
            if table.is_some() && self.scopes[s].items.iter().any(|it| it.name == lc(table.unwrap())) {
                break;
            }
        }
        Err(format!("no such column: {}{name}", table.map(|t| format!("{t}.")).unwrap_or_default()))
    }

    fn cmp_affs(&self, l: &PE, r: &PE) -> (Aff, Aff) {
        sv::comparison_affinity(self.affinity(l), self.affinity(r))
    }

    fn expr(&mut self, e: &Expr) -> Result<PE, String> {
        Ok(match e {
            Expr::Lit(v) => PE::Lit(v.clone()),
            Expr::Col { table, name } => match self.resolve(table.as_deref(), name) {
                Ok(pe) => pe,
                Err(err) => {
                    // A result alias of the current select.
                    if table.is_none() {
                        let alias = self.scopes.last().and_then(|s| s.aliases.iter().find(|(a, _)| *a == lc(name)).map(|(_, e)| e.clone()));
                        if let Some(ae) = alias {
                            return self.expr(&ae);
                        }
                    }
                    return Err(err);
                }
            },
            Expr::Unary(op, x) => {
                let px = self.expr(x)?;
                match *op {
                    "not" => PE::Not(Box::new(px)),
                    "-" => PE::Neg(Box::new(px)),
                    "+" => px,
                    _ => unreachable!(),
                }
            }
            Expr::Bin(op, l, r) => {
                let pl = self.expr(l)?;
                let pr = self.expr(r)?;
                match *op {
                    "and" => PE::And(Box::new(pl), Box::new(pr)),
                    "or" => PE::Or(Box::new(pl), Box::new(pr)),
                    "||" => PE::Concat(Box::new(pl), Box::new(pr)),
                    "+" | "-" | "*" | "/" | "%" => PE::Arith(op.chars().next().unwrap(), Box::new(pl), Box::new(pr)),
                    _ => {
                        let c = match *op {
                            "=" => Cmp::Eq,
                            "!=" => Cmp::Ne,
                            "<" => Cmp::Lt,
                            "<=" => Cmp::Le,
                            ">" => Cmp::Gt,
                            ">=" => Cmp::Ge,
                            o => return Err(format!("operator {o}")),
                        };
                        let a = self.cmp_affs(&pl, &pr);
                        PE::Cmp(c, Box::new(pl), Box::new(pr), a)
                    }
                }
            }
            Expr::Func { name, args, star, distinct, over } => {
                if let Some(w) = over {
                    if name != "row_number" || !args.is_empty() {
                        return Err(format!("window function {name} unsupported"));
                    }
                    let partition = w.partition.iter().map(|p| self.expr(p)).collect::<Result<Vec<_>, _>>()?;
                    let order = w.order.iter().map(|(o, d)| Ok((self.expr(o)?, *d))).collect::<Result<Vec<_>, String>>()?;
                    let sc = self.scopes.last_mut().unwrap();
                    sc.wins.push(WinSpec { partition, order });
                    return Ok(PE::Win((sc.wins.len() - 1) as u16));
                }
                let agg = match (name.as_str(), args.len(), *star) {
                    ("count", 0, true) => Some(AggFn::CountStar),
                    ("count", 1, false) => Some(AggFn::Count),
                    ("min", 1, false) => Some(AggFn::Min),
                    ("max", 1, false) => Some(AggFn::Max),
                    ("group_concat", 1 | 2, false) => Some(AggFn::GroupConcat),
                    ("sum", 1, false) => Some(AggFn::Sum),
                    ("total", 1, false) => Some(AggFn::Total),
                    _ => None,
                };
                if let Some(f) = agg {
                    if !self.scopes.last().is_some_and(|s| s.allow_agg) {
                        return Err(format!("misuse of aggregate {name}"));
                    }
                    // Aggregate arguments are evaluated per input row in this select's scope.
                    let save = self.scopes.last_mut().unwrap().allow_agg;
                    self.scopes.last_mut().unwrap().allow_agg = false;
                    let arg = args.first().map(|a| self.expr(a)).transpose();
                    let sep = args.get(1).map(|a| self.expr(a)).transpose();
                    self.scopes.last_mut().unwrap().allow_agg = save;
                    let sc = self.scopes.last_mut().unwrap();
                    sc.aggs.push(AggSpec { f, arg: arg?, sep: sep?, distinct: *distinct });
                    return Ok(PE::Agg((sc.aggs.len() - 1) as u16));
                }
                if *distinct || *star {
                    return Err(format!("{name}(DISTINCT/*) unsupported"));
                }
                let f = match name.as_str() {
                    "coalesce" => Func::Coalesce,
                    "ifnull" => Func::Ifnull,
                    "length" => Func::Length,
                    "lower" => Func::Lower,
                    "upper" => Func::Upper,
                    "trim" => Func::Trim,
                    "ltrim" => Func::Ltrim,
                    "rtrim" => Func::Rtrim,
                    "substr" | "substring" => Func::Substr,
                    "instr" => Func::Instr,
                    "replace" => Func::Replace,
                    "char" => Func::Char,
                    "json_quote" => Func::JsonQuote,
                    "json_extract" => Func::JsonExtract,
                    "rx_matches" => Func::RxMatches,
                    "rx_extract" => Func::RxExtract,
                    "rx_replace" => Func::RxReplace,
                    "source_text" => Func::SourceText,
                    "abs" => Func::Abs,
                    "typeof" => Func::Typeof,
                    "max" => Func::Max,
                    "min" => Func::Min,
                    "iif" => Func::Iif,
                    "nullif" => Func::Nullif,
                    o => return Err(format!("function {o} unsupported")),
                };
                let pargs = args.iter().map(|a| self.expr(a)).collect::<Result<Vec<_>, _>>()?;
                PE::Func(f, pargs)
            }
            Expr::Case { operand, whens, else_ } => {
                let po = operand.as_ref().map(|o| self.expr(o)).transpose()?;
                let mut pw = Vec::new();
                let mut aff = (Aff::None, Aff::None);
                for (w, t) in whens {
                    let pwv = self.expr(w)?;
                    if let Some(o) = &po {
                        aff = self.cmp_affs(o, &pwv);
                    }
                    pw.push((pwv, self.expr(t)?));
                }
                let pe = else_.as_ref().map(|x| self.expr(x)).transpose()?;
                PE::Case { operand: po.map(Box::new), aff, whens: pw, else_: pe.map(Box::new) }
            }
            Expr::Cast(x, ty) => match (&**x, Aff::of_type(ty)) {
                (Expr::Blob(text), Aff::Text) => PE::Cast(Box::new(PE::Lit(V::text(text))), Aff::Text),
                _ => PE::Cast(Box::new(self.expr(x)?), Aff::of_type(ty)),
            },
            Expr::InList { e, list, not } => {
                let pe = self.expr(e)?;
                let mut pl = Vec::new();
                for x in list {
                    let px = self.expr(x)?;
                    // `x IN (list)`: the left operand's affinity governs the comparisons.
                    let la = self.affinity(&pe);
                    let a = if la == Aff::None { (Aff::None, Aff::None) } else { (Aff::None, la) };
                    pl.push((px, a));
                }
                PE::InList { e: Box::new(pe), list: pl, not: *not }
            }
            Expr::InSub { e, q, not } => {
                let pe = self.expr(e)?;
                let id = self.subquery(q)?;
                if self.subs[id].q.out.cols.len() != 1 {
                    return Err("IN subquery must return one column".into());
                }
                let sub_aff = self.subs[id].q.out.affs[0];
                let aff = sv::comparison_affinity(self.affinity(&pe), sub_aff);
                PE::InSub { e: Box::new(pe), sub: id, not: *not, aff }
            }
            Expr::Exists { q, not } => PE::Exists { sub: self.subquery(q)?, not: *not },
            Expr::Sub(q) => PE::Scalar(self.subquery(q)?),
            Expr::IsNull(x, not) => PE::IsNull(Box::new(self.expr(x)?), *not),
            Expr::Is(l, r, not) => PE::Is(Box::new(self.expr(l)?), Box::new(self.expr(r)?), *not),
            Expr::Like(l, r, not) => PE::Like(Box::new(self.expr(l)?), Box::new(self.expr(r)?), *not),
            Expr::Between(..) => return Err("BETWEEN unsupported".into()),
            Expr::Blob(_) => return Err("blob literal outside CAST(.. AS TEXT) unsupported".into()),
        })
    }
}

fn flatten<'a>(b: &'a Body, arms: &mut Vec<&'a Body>, ops: &mut Vec<SetOp>) {
    match b {
        Body::Compound(l, op, r) => {
            flatten(l, arms, ops);
            ops.push(*op);
            flatten(r, arms, ops);
        }
        s => arms.push(s),
    }
}

fn body_refs(b: &Body, name: &str) -> bool {
    match b {
        Body::Select(s) => select_refs(s, name),
        Body::Compound(l, _, r) => body_refs(l, name) || body_refs(r, name),
    }
}
fn query_refs(q: &Query, name: &str) -> bool {
    q.ctes.iter().any(|c| query_refs(&c.q, name)) || body_refs(&q.body, name)
}
fn select_refs(s: &parse::Select, name: &str) -> bool {
    s.from.iter().any(|f| match &f.source {
        Source::Table(t) => t.eq_ignore_ascii_case(name),
        Source::Sub(q) => query_refs(q, name),
        Source::Func(_, a) => a.iter().any(|e| expr_refs(e, name)),
    }) || s.from.iter().any(|f| f.on.as_ref().is_some_and(|e| expr_refs(e, name)))
        || s.where_.as_ref().is_some_and(|e| expr_refs(e, name))
        || s.items.iter().any(|i| matches!(i, SelItem::Expr { e, .. } if expr_refs(e, name)))
}
fn expr_refs(e: &Expr, name: &str) -> bool {
    match e {
        Expr::InSub { e, q, .. } => expr_refs(e, name) || query_refs(q, name),
        Expr::Exists { q, .. } | Expr::Sub(q) => query_refs(q, name),
        Expr::Unary(_, x) | Expr::Cast(x, _) | Expr::IsNull(x, _) => expr_refs(x, name),
        Expr::Bin(_, l, r) | Expr::Is(l, r, _) | Expr::Like(l, r, _) => expr_refs(l, name) || expr_refs(r, name),
        Expr::Func { args, .. } => args.iter().any(|a| expr_refs(a, name)),
        Expr::Case { operand, whens, else_ } => {
            operand.as_ref().is_some_and(|o| expr_refs(o, name))
                || whens.iter().any(|(w, t)| expr_refs(w, name) || expr_refs(t, name))
                || else_.as_ref().is_some_and(|x| expr_refs(x, name))
        }
        Expr::InList { e, list, .. } => expr_refs(e, name) || list.iter().any(|x| expr_refs(x, name)),
        _ => false,
    }
}

fn visit_cols(pe: &PE, f: &mut dyn FnMut(u16, u16)) {
    match pe {
        PE::Col { depth, item, .. } => f(*depth, *item),
        PE::Not(x) | PE::Neg(x) | PE::Cast(x, _) | PE::IsNull(x, _) => visit_cols(x, f),
        PE::Cmp(_, l, r, _) | PE::Arith(_, l, r) | PE::Concat(l, r) | PE::And(l, r) | PE::Or(l, r) | PE::Is(l, r, _) | PE::Like(l, r, _) => {
            visit_cols(l, f);
            visit_cols(r, f);
        }
        PE::Func(_, args) => args.iter().for_each(|a| visit_cols(a, f)),
        PE::Case { operand, whens, else_, .. } => {
            if let Some(o) = operand {
                visit_cols(o, f);
            }
            for (w, t) in whens {
                visit_cols(w, f);
                visit_cols(t, f);
            }
            if let Some(x) = else_ {
                visit_cols(x, f);
            }
        }
        PE::InList { e, list, .. } => {
            visit_cols(e, f);
            list.iter().for_each(|(x, _)| visit_cols(x, f));
        }
        PE::InSub { e, .. } => visit_cols(e, f),
        // Subqueries' own outer references are tracked by their correlation; their reference to
        // this select's items is invisible here, so a correlated subquery is placed at the last
        // item (conservative: see `max_item`).
        _ => {}
    }
}

fn has_sub(pe: &PE) -> bool {
    let mut found = false;
    fn walk(pe: &PE, found: &mut bool) {
        match pe {
            PE::Exists { .. } | PE::Scalar(_) | PE::InSub { .. } => *found = true,
            PE::Not(x) | PE::Neg(x) | PE::Cast(x, _) | PE::IsNull(x, _) => walk(x, found),
            PE::Cmp(_, l, r, _) | PE::Arith(_, l, r) | PE::Concat(l, r) | PE::And(l, r) | PE::Or(l, r) | PE::Is(l, r, _) | PE::Like(l, r, _) => {
                walk(l, found);
                walk(r, found);
            }
            PE::Func(_, a) => a.iter().for_each(|x| walk(x, found)),
            PE::Case { operand, whens, else_, .. } => {
                if let Some(o) = operand {
                    walk(o, found);
                }
                for (w, t) in whens {
                    walk(w, found);
                    walk(t, found);
                }
                if let Some(x) = else_ {
                    walk(x, found);
                }
            }
            PE::InList { e, list, .. } => {
                walk(e, found);
                list.iter().for_each(|(x, _)| walk(x, found));
            }
            _ => {}
        }
    }
    walk(pe, &mut found);
    found
}

/// The last FROM item (depth 0) an expression reads; a subquery inside counts as reading every
/// item (it may be correlated to any of them).
fn max_item(pe: &PE) -> Option<usize> {
    let mut m: Option<usize> = None;
    visit_cols(pe, &mut |d, i| {
        if d == 0 {
            m = Some(m.map_or(i as usize, |x| x.max(i as usize)));
        }
    });
    if has_sub(pe) {
        return Some(usize::MAX);
    }
    m
}
fn refs_item(pe: &PE, i: usize) -> bool {
    let mut r = false;
    visit_cols(pe, &mut |d, it| {
        if d == 0 && it as usize == i {
            r = true;
        }
    });
    r
}
fn has_agg(pe: &PE) -> bool {
    match pe {
        PE::Agg(_) | PE::Win(_) => true,
        PE::Not(x) | PE::Neg(x) | PE::Cast(x, _) | PE::IsNull(x, _) => has_agg(x),
        PE::Cmp(_, l, r, _) | PE::Arith(_, l, r) | PE::Concat(l, r) | PE::And(l, r) | PE::Or(l, r) | PE::Is(l, r, _) | PE::Like(l, r, _) => has_agg(l) || has_agg(r),
        PE::Func(_, a) => a.iter().any(has_agg),
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Execution

pub struct Db<'a> {
    pub tables: FxHashMap<String, Rc<Table>>,
    pub source: &'a str,
    pub path: &'a str,
}

type Index = FxHashMap<Vec<Key>, Vec<u32>>;

/// A join's hash indexes per item: the plain equality probe's, and each OR branch's.
struct Indexes {
    plain: Vec<Option<Rc<Index>>>,
    or: Vec<Option<Vec<Rc<Index>>>>,
}

pub struct Exec<'d, 'a> {
    db: &'d Db<'a>,
    stmt: &'d Statement,
    slots: RefCell<Vec<Option<Rc<Table>>>>,
    indexes: RefCell<FxHashMap<(usize, Vec<u16>, Vec<Aff>), Rc<Index>>>,
    sub_cache: RefCell<FxHashMap<usize, Rc<SubResult>>>,
    derived: RefCell<FxHashMap<usize, Rc<Table>>>,
    keep: RefCell<Vec<Rc<Table>>>,
    pub error: RefCell<Option<String>>,
}

enum SubResult {
    Exists(bool),
    Scalar(V),
    Set(rustc_hash::FxHashSet<Key>, bool, bool),
}

#[derive(Clone, Copy)]
struct Frame<'r> {
    rows: &'r [&'r [V]],
    parent: Option<&'r Frame<'r>>,
}

struct EvalCtx<'r> {
    frame: Frame<'r>,
    aggs: Option<&'r [V]>,
    wins: Option<&'r [V]>,
}

fn empty_row<'a>() -> &'a [V] {
    &[]
}

pub fn execute(db: &Db<'_>, stmt: &Statement) -> Result<Table, String> {
    let ex = Exec {
        db,
        stmt,
        slots: RefCell::new(vec![None; stmt.n_slots]),
        indexes: Default::default(),
        sub_cache: Default::default(),
        derived: Default::default(),
        keep: Default::default(),
        error: RefCell::new(None),
    };
    let t = ex.query(&stmt.root, None);
    if let Some(e) = ex.error.borrow_mut().take() {
        return Err(e);
    }
    Ok(t)
}

/// Evaluates a planned scalar expression (deferred column) over one row.
pub fn eval_scalar(db: &Db<'_>, stmt: &Statement, pe: &PE, row: &[V]) -> Result<V, String> {
    let ex = Exec {
        db,
        stmt,
        slots: RefCell::new(vec![None; stmt.n_slots]),
        indexes: Default::default(),
        sub_cache: Default::default(),
        derived: Default::default(),
        keep: Default::default(),
        error: RefCell::new(None),
    };
    let rows = [row];
    let ctx = EvalCtx { frame: Frame { rows: &rows, parent: None }, aggs: None, wins: None };
    let v = ex.eval(pe, &ctx);
    if let Some(e) = ex.error.borrow_mut().take() {
        return Err(e);
    }
    Ok(v)
}

impl<'d, 'a> Exec<'d, 'a> {
    fn fail(&self, msg: String) {
        let mut e = self.error.borrow_mut();
        if e.is_none() {
            *e = Some(msg);
        }
    }
    fn failed(&self) -> bool {
        self.error.borrow().is_some()
    }

    fn query(&self, q: &PQuery, outer: Option<&Frame<'_>>) -> Table {
        for cte in &q.ctes {
            match cte {
                PCte::Plain { slot, q } => {
                    let t = self.query(q, outer);
                    self.slots.borrow_mut()[*slot] = Some(Rc::new(t));
                }
                PCte::Recursive { slot, batch, init, rec, distinct, schema } => {
                    let t = self.recursive(*batch, init, rec, *distinct, schema, outer);
                    self.slots.borrow_mut()[*slot] = Some(Rc::new(t));
                }
            }
        }
        let mut rows = self.body(&q.body, outer);
        if !q.order.is_empty() {
            let keys = &q.order;
            rows.sort_by(|a, b| {
                for (i, desc) in keys {
                    let o = sv::cmp(&a[*i], &b[*i]);
                    if o != Ordering::Equal {
                        return if *desc { o.reverse() } else { o };
                    }
                }
                Ordering::Equal
            });
        }
        let width = q.out.cols.len();
        let limit = q.limit.as_ref().map(|e| self.const_int(e, outer));
        let offset = q.offset.as_ref().map(|e| self.const_int(e, outer)).unwrap_or(0).max(0) as usize;
        let mut out: Vec<Box<[V]>> = Vec::new();
        for (i, r) in rows.into_iter().enumerate() {
            if i < offset {
                continue;
            }
            if let Some(l) = limit {
                if l >= 0 && out.len() as i64 >= l {
                    break;
                }
            }
            if r.len() == width {
                out.push(r);
            } else {
                out.push(r[..width].to_vec().into_boxed_slice());
            }
        }
        Table { schema: q.out.clone(), rows: out }
    }

    fn const_int(&self, e: &PE, outer: Option<&Frame<'_>>) -> i64 {
        let empty: [&[V]; 0] = [];
        let ctx = EvalCtx { frame: Frame { rows: &empty, parent: outer }, aggs: None, wins: None };
        match sv::cast(&self.eval(e, &ctx), Aff::Integer) {
            V::Int(i) => i,
            _ => -1,
        }
    }

    fn body(&self, b: &PBody, outer: Option<&Frame<'_>>) -> Vec<Box<[V]>> {
        match b {
            PBody::Select(s) => self.select_rows(s, outer),
            PBody::Compound(l, op, r, _) => {
                let mut left = self.body(l, outer);
                let right = self.body(r, outer);
                match op {
                    SetOp::UnionAll => {
                        left.extend(right);
                        left
                    }
                    SetOp::Union => {
                        let mut seen = rustc_hash::FxHashSet::default();
                        let mut out = Vec::new();
                        for row in left.into_iter().chain(right) {
                            let k: Vec<DKey> = row.iter().map(sv::dkey).collect();
                            if seen.insert(k) {
                                out.push(row);
                            }
                        }
                        out
                    }
                }
            }
        }
    }

    fn recursive(&self, batch: usize, init: &[PBody], rec: &[PBody], distinct: bool, schema: &Schema, outer: Option<&Frame<'_>>) -> Table {
        let mut seen = rustc_hash::FxHashSet::default();
        let mut result: Vec<Box<[V]>> = Vec::new();
        let mut queue: Vec<Box<[V]>> = Vec::new();
        let width = schema.cols.len();
        let mut admit = |row: Box<[V]>, queue: &mut Vec<Box<[V]>>| {
            let row: Box<[V]> = if row.len() == width { row } else { row[..width].to_vec().into_boxed_slice() };
            if distinct {
                let k: Vec<DKey> = row.iter().map(sv::dkey).collect();
                if !seen.insert(k) {
                    return;
                }
            }
            queue.push(row);
        };
        for arm in init {
            for row in self.body(arm, outer) {
                admit(row, &mut queue);
            }
        }
        let mut steps = 0usize;
        while !queue.is_empty() {
            if self.failed() {
                break;
            }
            steps += queue.len();
            if steps > 50_000_000 {
                self.fail("recursive CTE exceeded 50M rows".into());
                break;
            }
            let batch_rows = std::mem::take(&mut queue);
            let t = Rc::new(Table { schema: schema.clone(), rows: batch_rows });
            self.slots.borrow_mut()[batch] = Some(t.clone());
            // Indexes over a batch table are per batch.
            self.indexes.borrow_mut().retain(|k, _| k.0 != Rc::as_ptr(&t) as usize);
            for arm in rec {
                for row in self.body(arm, outer) {
                    admit(row, &mut queue);
                }
            }
            self.indexes.borrow_mut().retain(|k, _| k.0 != Rc::as_ptr(&t) as usize);
            self.slots.borrow_mut()[batch] = None;
            let t = Rc::try_unwrap(t).unwrap_or_else(|rc| Table { schema: rc.schema.clone(), rows: rc.rows.clone() });
            result.extend(t.rows);
        }
        Table { schema: schema.clone(), rows: result }
    }

    fn table_of(&self, src: &Src, outer: Option<&Frame<'_>>) -> Rc<Table> {
        match src {
            Src::Global(name) => match self.db.tables.get(&lc(name)) {
                Some(t) => t.clone(),
                None => {
                    self.fail(format!("table {name} missing at run time"));
                    Rc::new(Table::default())
                }
            },
            Src::Cte(slot) => match &self.slots.borrow()[*slot] {
                Some(t) => t.clone(),
                None => {
                    self.fail("CTE not materialised".into());
                    Rc::new(Table::default())
                }
            },
            Src::Derived(id) => {
                let sub = &self.stmt.subs[*id];
                if sub.cacheable {
                    if let Some(t) = self.derived.borrow().get(id) {
                        return t.clone();
                    }
                }
                let t = Rc::new(self.query(&sub.q, outer));
                if sub.cacheable {
                    self.derived.borrow_mut().insert(*id, t.clone());
                }
                t
            }
            Src::JsonEach(arg) => {
                let empty: [&[V]; 0] = [];
                let ctx = EvalCtx { frame: Frame { rows: &empty, parent: outer }, aggs: None, wins: None };
                let v = self.eval(arg, &ctx);
                Rc::new(self.json_each_table(v))
            }
        }
    }

    fn json_each_table(&self, v: V) -> Table {
        let schema = Schema {
            cols: ["key", "value", "type", "atom", "id", "parent", "fullkey", "path"].iter().map(|s| s.to_string()).collect(),
            affs: vec![Aff::None; 8],
        };
        let mut rows = Vec::new();
        if let V::Text(s) = v {
            match serde_json::from_str::<serde_json::Value>(&s) {
                Ok(serde_json::Value::Array(items)) => {
                    for (i, it) in items.iter().enumerate() {
                        rows.push(json_each_row(V::Int(i as i64), it, &format!("$[{i}]")));
                    }
                }
                Ok(serde_json::Value::Object(map)) => {
                    for (k, it) in map.iter() {
                        rows.push(json_each_row(V::text(k), it, &format!("$.{k}")));
                    }
                }
                Ok(other) => rows.push(json_each_row(V::Null, &other, "$")),
                Err(e) => self.fail(format!("malformed JSON: {e}")),
            }
        }
        Table { schema, rows }
    }

    fn index(&self, t: &Rc<Table>, cols: &[u16], affs: &[Aff]) -> Rc<Index> {
        let k = (Rc::as_ptr(t) as usize, cols.to_vec(), affs.to_vec());
        if let Some(ix) = self.indexes.borrow().get(&k) {
            return ix.clone();
        }
        let mut ix: Index = FxHashMap::default();
        'rows: for (ri, row) in t.rows.iter().enumerate() {
            let mut key = Vec::with_capacity(cols.len());
            for (c, a) in cols.iter().zip(affs) {
                match sv::key(&sv::compare_affinity(&row[*c as usize], *a)) {
                    Some(kk) => key.push(kk),
                    None => continue 'rows,
                }
            }
            ix.entry(key).or_default().push(ri as u32);
        }
        let ix = Rc::new(ix);
        self.keep.borrow_mut().push(t.clone());
        self.indexes.borrow_mut().insert(k, ix.clone());
        ix
    }

    /// Runs the join pipeline of `s`, calling `sink` with each binding that passes every
    /// conjunct.
    fn join<'o>(&self, s: &PSelect, outer: Option<&'o Frame<'o>>, sink: &mut dyn FnMut(&Frame<'_>) -> ControlFlow<()>) {
        let tables: Vec<Rc<Table>> = s.items.iter().map(|it| self.table_of(&it.src, outer)).collect();
        if self.failed() {
            return;
        }
        let nulls: Vec<Box<[V]>> = s.items.iter().map(|it| vec![V::Null; it.width].into_boxed_slice()).collect();
        let mut rows: Vec<&[V]> = vec![empty_row(); s.items.len()];
        // Constant conjuncts.
        {
            let ctx = EvalCtx { frame: Frame { rows: &rows, parent: outer }, aggs: None, wins: None };
            for c in &s.konst {
                if sv::truth(&self.eval(c, &ctx)) != Some(true) {
                    return;
                }
            }
        }
        let indexes = Indexes {
            plain: s.probes.iter().zip(&tables).map(|(p, t)| p.as_ref().map(|p| self.index(t, &p.cols, &p.col_affs))).collect(),
            or: s.or_probes.iter().zip(&tables).map(|(b, t)| b.as_ref().map(|b| b.iter().map(|p| self.index(t, &p.cols, &p.col_affs)).collect())).collect(),
        };
        let _ = self.level(s, 0, &tables, &nulls, &indexes, &mut rows, outer, sink);
    }

    /// The probe's key for the current binding; `None` when a key part is NULL (no row equals).
    fn probe_key(&self, p: &Probe, ctx: &EvalCtx<'_>) -> Option<Vec<Key>> {
        p.exprs.iter().zip(&p.expr_affs).map(|(e, a)| sv::key(&sv::compare_affinity(&self.eval(e, ctx), *a))).collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn level<'t>(
        &self,
        s: &PSelect,
        i: usize,
        tables: &'t [Rc<Table>],
        nulls: &'t [Box<[V]>],
        indexes: &Indexes,
        rows: &mut Vec<&'t [V]>,
        outer: Option<&Frame<'_>>,
        sink: &mut dyn FnMut(&Frame<'_>) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        if self.failed() {
            return ControlFlow::Break(());
        }
        if i == s.items.len() {
            let f = Frame { rows: rows.as_slice(), parent: outer };
            return sink(&f);
        }
        let item = &s.items[i];
        let lateral_table;
        let t: &'t Table = if item.lateral {
            let Src::JsonEach(arg) = &item.src else { unreachable!() };
            let v = {
                let ctx = EvalCtx { frame: Frame { rows: rows.as_slice(), parent: outer }, aggs: None, wins: None };
                self.eval(arg, &ctx)
            };
            lateral_table = Rc::new(self.json_each_table(v));
            let ptr = Rc::as_ptr(&lateral_table);
            self.keep.borrow_mut().push(lateral_table);
            // SAFETY: `keep` holds the table until this execution ends, past every borrow of it.
            unsafe { &*ptr }
        } else {
            &tables[i]
        };
        // Candidate rows, in table order.
        let cands: Option<Vec<u32>> = {
            let ctx = EvalCtx { frame: Frame { rows: rows.as_slice(), parent: outer }, aggs: None, wins: None };
            match (&s.or_probes[i], &indexes.or[i], &s.probes[i], &indexes.plain[i]) {
                (Some(branches), Some(ixs), _, _) => {
                    let mut c: Vec<u32> = Vec::new();
                    for (p, ix) in branches.iter().zip(ixs) {
                        if let Some(hits) = self.probe_key(p, &ctx).and_then(|k| ix.get(&k)) {
                            c.extend_from_slice(hits);
                        }
                    }
                    c.sort_unstable();
                    c.dedup();
                    Some(c)
                }
                (_, _, Some(p), Some(ix)) => Some(self.probe_key(p, &ctx).and_then(|k| ix.get(&k).cloned()).unwrap_or_default()),
                _ => None,
            }
        };
        let n = cands.as_ref().map_or(t.rows.len(), |c| c.len());
        let mut matched = false;
        for k in 0..n {
            let r = cands.as_ref().map_or(k, |c| c[k] as usize);
            rows[i] = &t.rows[r];
            let ok = {
                let ctx = EvalCtx { frame: Frame { rows: rows.as_slice(), parent: outer }, aggs: None, wins: None };
                s.level[i].iter().all(|c| sv::truth(&self.eval(c, &ctx)) == Some(true))
            };
            if !ok {
                continue;
            }
            matched = true;
            let post_ok = {
                let ctx = EvalCtx { frame: Frame { rows: rows.as_slice(), parent: outer }, aggs: None, wins: None };
                s.post[i].iter().all(|c| sv::truth(&self.eval(c, &ctx)) == Some(true))
            };
            if post_ok {
                self.level(s, i + 1, tables, nulls, indexes, rows, outer, sink)?;
            }
        }
        if item.left && !matched {
            rows[i] = &nulls[i];
            let post_ok = {
                let ctx = EvalCtx { frame: Frame { rows: rows.as_slice(), parent: outer }, aggs: None, wins: None };
                s.post[i].iter().all(|c| sv::truth(&self.eval(c, &ctx)) == Some(true))
            };
            if post_ok {
                self.level(s, i + 1, tables, nulls, indexes, rows, outer, sink)?;
            }
        }
        rows[i] = empty_row();
        ControlFlow::Continue(())
    }

    /// Output rows of a select (projection, plus ORDER BY keys appended).
    fn select_rows(&self, s: &PSelect, outer: Option<&Frame<'_>>) -> Vec<Box<[V]>> {
        let width = s.proj.len() + s.order_keys.len();
        let mut out: Vec<Box<[V]>> = Vec::new();
        if s.grouped {
            // Groups in first-seen order: key -> (agg states, representative row values).
            let mut groups: FxHashMap<Vec<DKey>, usize> = FxHashMap::default();
            let mut states: Vec<(Vec<AggState>, Vec<Vec<V>>)> = Vec::new();
            self.join(s, outer, &mut |f| {
                let ctx = EvalCtx { frame: *f, aggs: None, wins: None };
                let key: Vec<DKey> = s.group_by.iter().map(|g| sv::dkey(&self.eval(g, &ctx))).collect();
                let gi = *groups.entry(key).or_insert_with(|| {
                    states.push((s.aggs.iter().map(|_| AggState::default()).collect(), Vec::new()));
                    states.len() - 1
                });
                let (st, rep) = &mut states[gi];
                let mut extremum_updated = false;
                for (a, spec) in st.iter_mut().zip(&s.aggs) {
                    let v = spec.arg.as_ref().map(|e| self.eval(e, &ctx));
                    let sep = spec.sep.as_ref().map(|e| self.eval(e, &ctx));
                    if a.update(spec, v, sep) {
                        extremum_updated = true;
                    }
                }
                // Bare columns read the row a lone min()/max() chose, else the last row.
                let lone_extremum = s.aggs.len() == 1 && matches!(s.aggs[0].f, AggFn::Min | AggFn::Max);
                if rep.is_empty() || !lone_extremum || extremum_updated {
                    *rep = f.rows.iter().map(|r| r.to_vec()).collect();
                }
                ControlFlow::Continue(())
            });
            if s.group_by.is_empty() && states.is_empty() {
                states.push((s.aggs.iter().map(|_| AggState::default()).collect(), Vec::new()));
            }
            for (st, rep) in states {
                let aggs: Vec<V> = st.into_iter().zip(&s.aggs).map(|(a, spec)| a.finish(spec)).collect();
                let rep_rows: Vec<Box<[V]>> = if rep.is_empty() {
                    s.items.iter().map(|it| vec![V::Null; it.width].into_boxed_slice()).collect()
                } else {
                    rep.into_iter().map(|r| r.into_boxed_slice()).collect()
                };
                let refs: Vec<&[V]> = rep_rows.iter().map(|r| &r[..]).collect();
                let ctx = EvalCtx { frame: Frame { rows: &refs, parent: outer }, aggs: Some(&aggs), wins: None };
                if let Some(h) = &s.having {
                    if sv::truth(&self.eval(h, &ctx)) != Some(true) {
                        continue;
                    }
                }
                let mut row = Vec::with_capacity(width);
                for p in s.proj.iter().chain(&s.order_keys) {
                    row.push(self.eval(p, &ctx));
                }
                out.push(row.into_boxed_slice());
            }
        } else if !s.wins.is_empty() {
            // Window functions: keep each binding's values, number them, then project.
            let mut bound: Vec<Vec<Vec<V>>> = Vec::new();
            self.join(s, outer, &mut |f| {
                bound.push(f.rows.iter().map(|r| r.to_vec()).collect());
                ControlFlow::Continue(())
            });
            let mut wins = vec![vec![V::Null; s.wins.len()]; bound.len()];
            for (wi, w) in s.wins.iter().enumerate() {
                let mut keyed: Vec<(Vec<V>, Vec<V>, usize)> = bound
                    .iter()
                    .enumerate()
                    .map(|(bi, b)| {
                        let refs: Vec<&[V]> = b.iter().map(|r| &r[..]).collect();
                        let ctx = EvalCtx { frame: Frame { rows: &refs, parent: outer }, aggs: None, wins: None };
                        (
                            w.partition.iter().map(|e| self.eval(e, &ctx)).collect(),
                            w.order.iter().map(|(e, _)| self.eval(e, &ctx)).collect(),
                            bi,
                        )
                    })
                    .collect();
                keyed.sort_by(|a, b| {
                    for (x, y) in a.0.iter().zip(&b.0) {
                        let o = sv::cmp(x, y);
                        if o != Ordering::Equal {
                            return o;
                        }
                    }
                    for (k, (x, y)) in a.1.iter().zip(&b.1).enumerate() {
                        let o = sv::cmp(x, y);
                        if o != Ordering::Equal {
                            return if w.order[k].1 { o.reverse() } else { o };
                        }
                    }
                    a.2.cmp(&b.2)
                });
                let mut n = 0i64;
                let mut prev: Option<&Vec<V>> = None;
                for (p, _, bi) in &keyed {
                    let same = prev.is_some_and(|q| q.iter().zip(p).all(|(x, y)| sv::cmp(x, y) == Ordering::Equal));
                    n = if same { n + 1 } else { 1 };
                    prev = Some(p);
                    wins[*bi][wi] = V::Int(n);
                }
            }
            for (b, w) in bound.iter().zip(&wins) {
                let refs: Vec<&[V]> = b.iter().map(|r| &r[..]).collect();
                let ctx = EvalCtx { frame: Frame { rows: &refs, parent: outer }, aggs: None, wins: Some(w) };
                let mut row = Vec::with_capacity(width);
                for p in s.proj.iter().chain(&s.order_keys) {
                    row.push(self.eval(p, &ctx));
                }
                out.push(row.into_boxed_slice());
            }
        } else {
            self.join(s, outer, &mut |f| {
                let ctx = EvalCtx { frame: *f, aggs: None, wins: None };
                let mut row = Vec::with_capacity(width);
                for p in s.proj.iter().chain(&s.order_keys) {
                    row.push(self.eval(p, &ctx));
                }
                out.push(row.into_boxed_slice());
                ControlFlow::Continue(())
            });
        }
        if s.distinct {
            let mut seen = rustc_hash::FxHashSet::default();
            out.retain(|r| seen.insert(r[..s.proj.len()].iter().map(sv::dkey).collect::<Vec<_>>()));
        }
        out
    }

    fn sub(&self, id: usize, ctx: &EvalCtx<'_>, want: SubWant) -> Rc<SubResult> {
        let sub = &self.stmt.subs[id];
        if sub.cacheable {
            if let Some(r) = self.sub_cache.borrow().get(&id) {
                return r.clone();
            }
        }
        let frame = ctx.frame;
        let r = match want {
            SubWant::Exists => {
                let mut found = false;
                match &sub.q.body {
                    PBody::Select(s) if !s.grouped && s.wins.is_empty() && sub.q.ctes.is_empty() && sub.q.limit.is_none() => {
                        self.join(s, Some(&frame), &mut |_| {
                            found = true;
                            ControlFlow::Break(())
                        });
                    }
                    _ => {
                        found = !self.query(&sub.q, Some(&frame)).rows.is_empty();
                    }
                }
                SubResult::Exists(found)
            }
            SubWant::Scalar => {
                let t = self.query(&sub.q, Some(&frame));
                SubResult::Scalar(t.rows.first().map_or(V::Null, |r| r[0].clone()))
            }
            SubWant::Set(aff) => {
                let t = self.query(&sub.q, Some(&frame));
                let mut set = rustc_hash::FxHashSet::default();
                let mut has_null = false;
                for r in &t.rows {
                    match sv::key(&sv::compare_affinity(&r[0], aff)) {
                        Some(k) => {
                            set.insert(k);
                        }
                        None => has_null = true,
                    }
                }
                SubResult::Set(set, has_null, !t.rows.is_empty())
            }
        };
        let r = Rc::new(r);
        if sub.cacheable {
            self.sub_cache.borrow_mut().insert(id, r.clone());
        }
        r
    }

    fn col(&self, ctx: &EvalCtx<'_>, depth: u16, item: u16, col: u16) -> V {
        let mut f = &ctx.frame;
        for _ in 0..depth {
            match f.parent {
                Some(p) => f = p,
                None => return V::Null,
            }
        }
        f.rows.get(item as usize).and_then(|r| r.get(col as usize)).cloned().unwrap_or(V::Null)
    }

    fn eval(&self, pe: &PE, ctx: &EvalCtx<'_>) -> V {
        match pe {
            PE::Lit(v) => v.clone(),
            PE::Col { depth, item, col } => self.col(ctx, *depth, *item, *col),
            PE::Agg(i) => ctx.aggs.and_then(|a| a.get(*i as usize)).cloned().unwrap_or(V::Null),
            PE::Win(i) => ctx.wins.and_then(|a| a.get(*i as usize)).cloned().unwrap_or(V::Null),
            PE::Not(x) => match sv::truth(&self.eval(x, ctx)) {
                None => V::Null,
                Some(b) => V::bool(!b),
            },
            PE::Neg(x) => match sv::to_numeric(&self.eval(x, ctx)) {
                None => V::Null,
                Some(V::Int(i)) => i.checked_neg().map_or(V::Real(-(i as f64)), V::Int),
                Some(V::Real(f)) => V::Real(-f),
                Some(_) => V::Null,
            },
            PE::Cmp(op, l, r, aff) => {
                let lv = self.eval(l, ctx);
                let rv = self.eval(r, ctx);
                match sv::compare(&lv, &rv, *aff) {
                    None => V::Null,
                    Some(o) => V::bool(match op {
                        Cmp::Eq => o == Ordering::Equal,
                        Cmp::Ne => o != Ordering::Equal,
                        Cmp::Lt => o == Ordering::Less,
                        Cmp::Le => o != Ordering::Greater,
                        Cmp::Gt => o == Ordering::Greater,
                        Cmp::Ge => o != Ordering::Less,
                    }),
                }
            }
            PE::Arith(op, l, r) => sv::arith(*op, &self.eval(l, ctx), &self.eval(r, ctx)),
            PE::Concat(l, r) => {
                let (a, b) = (self.eval(l, ctx), self.eval(r, ctx));
                match (sv::to_text(&a), sv::to_text(&b)) {
                    (Some(x), Some(y)) => {
                        let mut s = String::with_capacity(x.len() + y.len());
                        s.push_str(&x);
                        s.push_str(&y);
                        V::Text(Rc::from(s))
                    }
                    _ => V::Null,
                }
            }
            PE::And(l, r) => {
                let a = sv::truth(&self.eval(l, ctx));
                if a == Some(false) {
                    return V::Int(0);
                }
                let b = sv::truth(&self.eval(r, ctx));
                match (a, b) {
                    (_, Some(false)) => V::Int(0),
                    (Some(true), Some(true)) => V::Int(1),
                    _ => V::Null,
                }
            }
            PE::Or(l, r) => {
                let a = sv::truth(&self.eval(l, ctx));
                if a == Some(true) {
                    return V::Int(1);
                }
                let b = sv::truth(&self.eval(r, ctx));
                match (a, b) {
                    (_, Some(true)) => V::Int(1),
                    (Some(false), Some(false)) => V::Int(0),
                    _ => V::Null,
                }
            }
            PE::Func(f, args) => self.func(*f, args, ctx),
            PE::Case { operand, aff, whens, else_ } => {
                let ov = operand.as_ref().map(|o| self.eval(o, ctx));
                for (w, t) in whens {
                    let hit = match &ov {
                        Some(o) => sv::compare(o, &self.eval(w, ctx), *aff) == Some(Ordering::Equal),
                        None => sv::truth(&self.eval(w, ctx)) == Some(true),
                    };
                    if hit {
                        return self.eval(t, ctx);
                    }
                }
                else_.as_ref().map_or(V::Null, |e| self.eval(e, ctx))
            }
            PE::Cast(x, a) => sv::cast(&self.eval(x, ctx), *a),
            PE::InList { e, list, not } => {
                let v = self.eval(e, ctx);
                if v.is_null() {
                    return if list.is_empty() { V::bool(*not) } else { V::Null };
                }
                let mut null = false;
                for (x, aff) in list {
                    let xv = self.eval(x, ctx);
                    match sv::compare(&v, &xv, *aff) {
                        Some(Ordering::Equal) => return V::bool(!*not),
                        None => null = true,
                        _ => {}
                    }
                }
                if null { V::Null } else { V::bool(*not) }
            }
            PE::InSub { e, sub, not, aff } => {
                let v = self.eval(e, ctx);
                let r = self.sub(*sub, ctx, SubWant::Set(aff.1));
                let SubResult::Set(set, has_null, nonempty) = &*r else { unreachable!() };
                if !nonempty {
                    return V::bool(*not);
                }
                let Some(k) = sv::key(&sv::compare_affinity(&v, aff.0)) else { return V::Null };
                if set.contains(&k) {
                    V::bool(!*not)
                } else if *has_null {
                    V::Null
                } else {
                    V::bool(*not)
                }
            }
            PE::Exists { sub, not } => {
                let r = self.sub(*sub, ctx, SubWant::Exists);
                let SubResult::Exists(b) = &*r else { unreachable!() };
                V::bool(*b != *not)
            }
            PE::Scalar(sub) => {
                let r = self.sub(*sub, ctx, SubWant::Scalar);
                let SubResult::Scalar(v) = &*r else { unreachable!() };
                v.clone()
            }
            PE::IsNull(x, not) => V::bool(self.eval(x, ctx).is_null() != *not),
            PE::Is(l, r, not) => {
                let (a, b) = (self.eval(l, ctx), self.eval(r, ctx));
                let eq = match (&a, &b) {
                    (V::Null, V::Null) => true,
                    (V::Null, _) | (_, V::Null) => false,
                    _ => sv::cmp(&a, &b) == Ordering::Equal,
                };
                V::bool(eq != *not)
            }
            PE::Like(l, r, not) => {
                let (a, b) = (self.eval(l, ctx), self.eval(r, ctx));
                match (sv::to_text(&a), sv::to_text(&b)) {
                    (Some(t), Some(p)) => V::bool(sv::like(&p, &t) != *not),
                    _ => V::Null,
                }
            }
        }
    }

    fn regex(&self, pattern: &str) -> Option<Rc<regex::Regex>> {
        match crate::rx::local(pattern) {
            Ok(r) => Some(r),
            Err(e) => {
                self.fail(format!("regex: {e}"));
                None
            }
        }
    }

    fn rx_text(&self, v: &V) -> Option<Rc<str>> {
        match v {
            V::Null => Some(Rc::from("")),
            V::Text(s) => Some(s.clone()),
            _ => {
                self.fail("rx_* text argument must be text".into());
                None
            }
        }
    }

    fn func(&self, f: Func, args: &[PE], ctx: &EvalCtx<'_>) -> V {
        let a = |i: usize| self.eval(&args[i], ctx);
        match f {
            Func::Coalesce | Func::Ifnull => {
                for x in args {
                    let v = self.eval(x, ctx);
                    if !v.is_null() {
                        return v;
                    }
                }
                V::Null
            }
            Func::Length => match a(0) {
                V::Null => V::Null,
                V::Text(s) => V::Int(s.chars().count() as i64),
                v => V::Int(sv::to_text(&v).map_or(0, |s| s.chars().count() as i64)),
            },
            Func::Lower | Func::Upper => match sv::to_text(&a(0)) {
                None => V::Null,
                Some(s) => V::text(&if f == Func::Lower { s.to_ascii_lowercase() } else { s.to_ascii_uppercase() }),
            },
            Func::Trim | Func::Ltrim | Func::Rtrim => {
                let Some(s) = sv::to_text(&a(0)) else { return V::Null };
                let chars = if args.len() > 1 {
                    match sv::to_text(&a(1)) {
                        Some(c) => Some(c),
                        None => return V::Null,
                    }
                } else {
                    None
                };
                V::text(&sv::trim(&s, chars.as_deref(), f != Func::Rtrim, f != Func::Ltrim))
            }
            Func::Substr => {
                let Some(s) = sv::to_text(&a(0)) else { return V::Null };
                let st = match sv::cast(&a(1), Aff::Integer) {
                    V::Int(i) => i,
                    _ => return V::Null,
                };
                let len = if args.len() > 2 {
                    match sv::cast(&a(2), Aff::Integer) {
                        V::Int(i) => Some(i),
                        _ => return V::Null,
                    }
                } else {
                    None
                };
                V::text(&sv::substr(&s, st, len))
            }
            Func::Instr => {
                let (h, n) = (a(0), a(1));
                match (sv::to_text(&h), sv::to_text(&n)) {
                    (Some(h), Some(n)) => match h.find(&*n) {
                        Some(b) => V::Int(h[..b].chars().count() as i64 + 1),
                        None => V::Int(0),
                    },
                    _ => V::Null,
                }
            }
            Func::Replace => {
                let (s, from, to) = (a(0), a(1), a(2));
                match (sv::to_text(&s), sv::to_text(&from), sv::to_text(&to)) {
                    (Some(s), Some(from), Some(to)) => {
                        if from.is_empty() {
                            V::Text(s)
                        } else {
                            V::text(&s.replace(&*from, &to))
                        }
                    }
                    _ => V::Null,
                }
            }
            Func::Char => {
                let mut s = String::new();
                for i in 0..args.len() {
                    if let V::Int(c) = sv::cast(&a(i), Aff::Integer) {
                        if let Some(ch) = char::from_u32(c as u32) {
                            s.push(ch);
                        }
                    }
                }
                V::text(&s)
            }
            Func::JsonQuote => sv::json_quote(&a(0)),
            Func::JsonExtract => {
                let (j, p) = (a(0), a(1));
                match (sv::to_text(&j), sv::to_text(&p)) {
                    (Some(j), Some(p)) => match sv::json_extract(&j, &p) {
                        Ok(v) => v,
                        Err(e) => {
                            self.fail(e);
                            V::Null
                        }
                    },
                    _ => V::Null,
                }
            }
            Func::RxMatches => {
                let Some(t) = self.rx_text(&a(0)) else { return V::Null };
                let V::Text(p) = a(1) else {
                    self.fail("rx pattern must be text".into());
                    return V::Null;
                };
                match self.regex(&p) {
                    Some(r) => V::bool(r.is_match(&t)),
                    None => V::Null,
                }
            }
            Func::RxExtract => {
                let Some(t) = self.rx_text(&a(0)) else { return V::Null };
                let V::Text(p) = a(1) else {
                    self.fail("rx pattern must be text".into());
                    return V::Null;
                };
                let g = match a(2) {
                    V::Int(g) if g >= 0 => g as usize,
                    _ => {
                        self.fail("rx_extract group must be a non-negative integer".into());
                        return V::Null;
                    }
                };
                let Some(r) = self.regex(&p) else { return V::Null };
                if g >= r.captures_len() {
                    self.fail("unknown regex group".into());
                    return V::Null;
                }
                match r.captures(&t).and_then(|c| c.get(g)) {
                    Some(m) => V::text(m.as_str()),
                    None => V::Null,
                }
            }
            Func::RxReplace => {
                let Some(t) = self.rx_text(&a(0)) else { return V::Null };
                let (V::Text(p), V::Text(by)) = (a(1), a(2)) else {
                    self.fail("rx_replace pattern and replacement must be text".into());
                    return V::Null;
                };
                let Some(r) = self.regex(&p) else { return V::Null };
                V::text(&r.replace_all(&t, &*by))
            }
            Func::SourceText => {
                let (p, s, e) = (a(0), a(1), a(2));
                let V::Text(p) = p else {
                    if !p.is_null() {
                        self.fail("source_text path must be text".into());
                    }
                    return V::Null;
                };
                if &*p != self.db.path {
                    return V::Null;
                }
                match (s, e) {
                    (V::Int(s), V::Int(e)) if s >= 0 && e >= s => match self.db.source.get(s as usize..e as usize) {
                        Some(t) => V::text(t),
                        None => V::Null,
                    },
                    _ => V::Null,
                }
            }
            Func::Abs => match sv::to_numeric(&a(0)) {
                None => V::Null,
                Some(V::Int(i)) => V::Int(i.abs()),
                Some(V::Real(f)) => V::Real(f.abs()),
                _ => V::Null,
            },
            Func::Typeof => V::text(match a(0) {
                V::Null => "null",
                V::Int(_) => "integer",
                V::Real(_) => "real",
                V::Text(_) => "text",
            }),
            Func::Max | Func::Min => {
                let mut best: Option<V> = None;
                for i in 0..args.len() {
                    let v = a(i);
                    if v.is_null() {
                        return V::Null;
                    }
                    best = Some(match best {
                        None => v,
                        Some(b) => {
                            let o = sv::cmp(&v, &b);
                            if (f == Func::Max && o == Ordering::Greater) || (f == Func::Min && o == Ordering::Less) { v } else { b }
                        }
                    });
                }
                best.unwrap_or(V::Null)
            }
            Func::Iif => {
                if sv::truth(&a(0)) == Some(true) { a(1) } else { a(2) }
            }
            Func::Nullif => {
                let (x, y) = (a(0), a(1));
                if sv::compare(&x, &y, (Aff::None, Aff::None)) == Some(Ordering::Equal) { V::Null } else { x }
            }
        }
    }
}

fn json_each_row(key: V, v: &serde_json::Value, path: &str) -> Box<[V]> {
    let ty = match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(true) => "true",
        serde_json::Value::Bool(false) => "false",
        serde_json::Value::Number(n) => {
            if n.is_i64() || n.is_u64() { "integer" } else { "real" }
        }
        serde_json::Value::String(_) => "text",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    };
    let value = sv::json_value_to_sql(v);
    let atom = match v {
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => V::Null,
        _ => value.clone(),
    };
    vec![key, value, V::text(ty), atom, V::Null, V::Null, V::text(path), V::text("$")].into_boxed_slice()
}

#[derive(Clone, Copy)]
enum SubWant {
    Exists,
    Scalar,
    Set(Aff),
}

#[derive(Default)]
struct AggState {
    count: i64,
    best: Option<V>,
    concat: Option<String>,
    sum_i: Option<i64>,
    sum_f: Option<f64>,
    seen: Option<rustc_hash::FxHashSet<DKey>>,
}

impl AggState {
    /// Folds one value; true when a min/max took this row.
    fn update(&mut self, spec: &AggSpec, v: Option<V>, sep: Option<V>) -> bool {
        if spec.f == AggFn::CountStar {
            self.count += 1;
            return false;
        }
        let v = v.unwrap_or(V::Null);
        if v.is_null() {
            return false;
        }
        if spec.distinct {
            let set = self.seen.get_or_insert_default();
            if !set.insert(sv::dkey(&v)) {
                return false;
            }
        }
        match spec.f {
            AggFn::Count => self.count += 1,
            AggFn::Min | AggFn::Max => {
                let better = match &self.best {
                    None => true,
                    Some(b) => {
                        let o = sv::cmp(&v, b);
                        (spec.f == AggFn::Max && o == Ordering::Greater) || (spec.f == AggFn::Min && o == Ordering::Less)
                    }
                };
                if better {
                    self.best = Some(v);
                    return true;
                }
            }
            AggFn::GroupConcat => {
                let t = sv::to_text(&v).unwrap();
                match &mut self.concat {
                    None => self.concat = Some(t.to_string()),
                    Some(c) => {
                        let s = match &sep {
                            Some(V::Null) => String::new(),
                            Some(x) => sv::to_text(x).map(|t| t.to_string()).unwrap_or_default(),
                            None => ",".into(),
                        };
                        c.push_str(&s);
                        c.push_str(&t);
                    }
                }
            }
            AggFn::Sum | AggFn::Total => match sv::to_numeric(&v) {
                Some(V::Int(i)) if self.sum_f.is_none() => match self.sum_i.unwrap_or(0).checked_add(i) {
                    Some(s) => self.sum_i = Some(s),
                    None => self.sum_f = Some(self.sum_i.unwrap_or(0) as f64 + i as f64),
                },
                Some(V::Int(i)) => self.sum_f = Some(self.sum_f.unwrap() + i as f64),
                Some(V::Real(f)) => {
                    self.sum_f = Some(self.sum_f.unwrap_or(self.sum_i.unwrap_or(0) as f64) + f);
                }
                _ => {}
            },
            AggFn::CountStar => {}
        }
        false
    }
    fn finish(self, spec: &AggSpec) -> V {
        match spec.f {
            AggFn::Count | AggFn::CountStar => V::Int(self.count),
            AggFn::Min | AggFn::Max => self.best.unwrap_or(V::Null),
            AggFn::GroupConcat => self.concat.map_or(V::Null, |c| V::text(&c)),
            AggFn::Sum => match (self.sum_i, self.sum_f) {
                (_, Some(f)) => V::Real(f),
                (Some(i), None) => V::Int(i),
                (None, None) => V::Null,
            },
            AggFn::Total => V::Real(self.sum_f.unwrap_or(self.sum_i.unwrap_or(0) as f64)),
        }
    }
}
