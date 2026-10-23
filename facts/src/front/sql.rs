//! SQL front end: a parsed statement (`sql::parse`) lowered to the algebra (`ir::Rel`), names
//! resolved, affinities fixed. Nothing downstream holds SQL text (INVARIANTS I3).
//!
//! Lowering of one `SELECT` (IR.md § 4): FROM items fold left in text order (`Join`, or
//! `JsonEach` over the items before it); `WHERE` is one `Filter` above them; aggregates make an
//! `Aggregate` (representative row ++ aggregates), windows a `Window` (input ++ functions);
//! the select list (with `ORDER BY` keys behind it) is a `Project`; `DISTINCT` a `Distinct`
//! over the select-list prefix; `ORDER BY`/`LIMIT` a `Sort` and a `Project` dropping the keys.
//!
//! Scopes: a subquery expression opens a new level; a derived table, a CTE body or a compound
//! arm stays on its level. Only the top-most scope of each level is visible (a derived table
//! cannot see its FROM siblings: no lateral subqueries). A column found `d` levels out is
//! `Col { depth: d }`.
//!
//! Typing (SQLite 3.53.2, the native engine's): the lowerer tracks each expression's SQLite
//! affinity as [`SAff`], which tells BLOB (a stored column without a declared type, `CAST(… AS
//! BLOB)`, a subquery column that is such an expression) apart from an expression's no affinity;
//! the IR keeps the [`Aff`] and the comparison affinities that follow from it. A compound used as
//! a FROM subquery, a CTE or a statement (a stage's CTAS) types its columns as
//! `sqlite3SubqueryColumnTypes` does ([`column_aff`]); reading a REAL column of such a compound
//! converts integers to reals (`OP_RealAffinity`), lowered as a `Project` ([`real_read`]).
use crate::ir::{AggCall, AggFn, AnySource, ArithOp, Binding, CmpOp, Column, Expr, Func, JoinKind, Lit, LocalId, Rel, WindowFn, WindowKind, Total};
use crate::sql::parse::{self, Body, Expr as P, Query, SelItem, SetOp, Source};
use crate::sql::value::{self as sv, Aff, V};
use std::collections::HashMap;

type E = Expr<AnySource>;
type R = Rel<AnySource>;
/// A lowered query body: its relation, columns, `ORDER BY` keys riding behind them, and per arm
/// its columns' typing.
type Lowered = (R, Vec<Column>, usize, Vec<Vec<ArmCol>>);

/// Relations a statement may name: lower-case name → source and schema.
#[derive(Default, Clone)]
pub struct Catalog {
    tables: HashMap<String, (AnySource, Vec<Column>)>,
}

impl Catalog {
    pub fn insert(&mut self, name: &str, src: AnySource, schema: Vec<Column>) {
        self.tables.insert(name.to_ascii_lowercase(), (src, schema));
    }
    pub fn get(&self, name: &str) -> Option<&(AnySource, Vec<Column>)> {
        self.tables.get(&name.to_ascii_lowercase())
    }
}

pub fn lit(v: &V) -> Lit {
    match v {
        V::Null => Lit::Null,
        V::Int(i) => Lit::Int(*i),
        V::Real(r) => Lit::Real(*r),
        V::Text(t) => Lit::Text(Box::from(&**t)),
    }
}

/// The json_each table function's columns.
pub fn json_each_schema() -> Vec<Column> {
    ["key", "value", "type", "atom", "id", "parent", "fullkey", "path"].iter().map(|c| Column::new(c, Aff::None)).collect()
}

/// An expression's SQLite affinity, in SQLite's order (`SQLITE_AFF_NONE` … `SQLITE_AFF_FLEXNUM`).
/// `None` is an expression's no affinity; `Blob` a column's BLOB affinity (no declared type),
/// which ranks above it in a comparison; `Flex` (FLEXNUM) is numeric in a comparison and converts
/// nothing on read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SAff {
    None,
    Blob,
    Text,
    Numeric,
    Integer,
    Real,
    Flex,
}

impl SAff {
    /// An expression's affinity as the IR's (BLOB and none are both `Aff::None`; FLEXNUM is
    /// numeric).
    fn aff(self) -> Aff {
        match self {
            SAff::None | SAff::Blob => Aff::None,
            SAff::Text => Aff::Text,
            SAff::Numeric | SAff::Flex => Aff::Numeric,
            SAff::Integer => Aff::Integer,
            SAff::Real => Aff::Real,
        }
    }
    /// The affinity of a stored column (an input, a stage, a catalog relation) of IR affinity
    /// `a`: none declared is BLOB.
    fn stored(a: Aff) -> SAff {
        match a {
            Aff::None => SAff::Blob,
            a => SAff::of(a),
        }
    }
    /// An expression's affinity of IR affinity `a` (`CAST` targets: `BLOB` is `Blob`).
    fn of(a: Aff) -> SAff {
        match a {
            Aff::None => SAff::None,
            Aff::Text => SAff::Text,
            Aff::Numeric => SAff::Numeric,
            Aff::Integer => SAff::Integer,
            Aff::Real => SAff::Real,
        }
    }
    /// `sqlite3ExprDataType` of a column, subquery or `CAST` of this affinity: the storage
    /// classes it may hold (0x01 numeric, 0x02 text, 0x04 blob).
    fn data_type(self) -> u8 {
        match self {
            SAff::Numeric | SAff::Integer | SAff::Real | SAff::Flex => 0x05,
            SAff::Text => 0x06,
            SAff::None | SAff::Blob => 0x07,
        }
    }
}

fn stored_types(schema: &[Column]) -> Vec<SAff> {
    schema.iter().map(|c| SAff::stored(c.aff)).collect()
}

/// The comparison affinities of two operands (`sqlite3CompareAffinity`): `comparison_affinity`,
/// except that a TEXT operand compared with a BLOB one converts nothing (both have an affinity,
/// neither numeric), where with an expression of no affinity it converts that expression.
fn cmp_aff(a: SAff, b: SAff) -> (Aff, Aff) {
    if (a == SAff::Text && b == SAff::Blob) || (b == SAff::Text && a == SAff::Blob) {
        return (Aff::None, Aff::None);
    }
    sv::comparison_affinity(a.aff(), b.aff())
}

/// Whether a subquery's relation is a compound (under its `ORDER BY`/`LIMIT` and `WITH`): its
/// values come from every arm, while its affinity is the right-most arm's.
fn compound(r: &R) -> bool {
    match r {
        Rel::Union { .. } => true,
        Rel::Sort { input, .. } => compound(input),
        Rel::With { body, .. } => compound(body),
        _ => false,
    }
}

/// Whether an operand is a scalar subquery over a compound: its value need not be normalised for
/// its affinity (`(SELECT k FROM untyped UNION ALL SELECT v FROM real_col)` may be the text
/// `'1'` with REAL affinity).
fn loose(e: &E) -> bool {
    matches!(e, Expr::Scalar(r) if compound(r))
}

/// The comparison affinities when an operand's values are not normalised for its affinity (a
/// compound subquery): SQLite applies the comparison's one affinity to both operands, which for
/// normalised values is what [`cmp_aff`] converts, and for others is not.
fn cmp_aff_both(a: SAff, b: SAff) -> (Aff, Aff) {
    let x = if a.aff().numeric_class() || b.aff().numeric_class() {
        Aff::Numeric
    } else {
        match cmp_aff(a, b) {
            (Aff::None, y) => y,
            (x, _) => x,
        }
    };
    (x, x)
}

/// The comparison affinities of operands `l` and `r` of affinities `a` and `b`.
fn operands_aff(l: &E, a: SAff, r: &E, b: SAff) -> (Aff, Aff) {
    if loose(l) || loose(r) { cmp_aff_both(a, b) } else { cmp_aff(a, b) }
}

/// One output column of one compound arm (a simple select): its expression's affinity, its
/// `sqlite3ExprDataType` and whether it is a `CAST`.
#[derive(Clone, Copy, Debug)]
struct ArmCol {
    ty: SAff,
    dt: u8,
    cast: bool,
}

/// `sqlite3SubqueryColumnTypes`' first phase over one column of a compound's arms (in text order):
/// the first arm's affinity; while it is none and another arm follows, the data types of the arms
/// passed over accumulate and the next arm's affinity is taken. The affinity, the index of the arm
/// it came from, and the accumulated data types.
fn walk_arms(arms: &[ArmCol]) -> (SAff, usize, u8) {
    let (mut aff, mut j, mut m) = (arms[0].ty, 0, 0u8);
    while aff <= SAff::None && j + 1 < arms.len() {
        m |= arms[j].dt;
        j += 1;
        aff = arms[j].ty;
    }
    (aff, j, m)
}

/// The second phase: a TEXT or numeric affinity of a compound (`post`: the arms, typed as they
/// are after the first phase) becomes BLOB when an arm after `j` may hold a number (TEXT) or a
/// text (numeric); a numeric one whose left-most arm is a `CAST` becomes FLEXNUM. A none affinity
/// stays none (a subquery's default; a CTAS's BLOB is the stored column's none).
fn settle(aff: SAff, j: usize, mut m: u8, post: &[ArmCol]) -> SAff {
    if aff < SAff::Text || post.len() < 2 {
        return aff;
    }
    for a in &post[j + 1..] {
        m |= a.dt;
    }
    let mut aff = aff;
    if (aff == SAff::Text && m & 0x01 != 0) || (aff >= SAff::Numeric && m & 0x02 != 0) {
        aff = SAff::Blob;
    }
    if aff >= SAff::Numeric && post[0].cast {
        aff = SAff::Flex;
    }
    aff
}

/// A compound column's affinity over its arms (`sqlite3SubqueryColumnTypes`).
fn column_aff(arms: &[ArmCol]) -> SAff {
    let (aff, j, m) = walk_arms(arms);
    settle(aff, j, m, arms)
}

/// A query's typing: its columns' affinities as a FROM subquery, CTE or statement sees them;
/// the affinity of the right-most arm's first expression (what a scalar subquery or `IN
/// (SELECT …)` compares with: SQLite reads the compound's last select); whether it is a compound.
struct QTy {
    cols: Vec<SAff>,
    first: SAff,
    compound: bool,
}

fn qty(arms: &[Vec<ArmCol>]) -> QTy {
    let w = arms[0].len();
    let cols = (0..w).map(|i| column_aff(&arms.iter().map(|a| a[i]).collect::<Vec<_>>())).collect();
    QTy { cols, first: arms.last().and_then(|a| a.first()).map_or(SAff::None, |c| c.ty), compound: arms.len() > 1 }
}

/// `rel`'s rows as read through `schema`'s REAL columns (`ty`): an integer becomes a real
/// (`OP_RealAffinity`; texts and reals unchanged), `CASE typeof(c) WHEN 'integer' THEN CAST(c AS
/// REAL) ELSE c END`. `rel` itself when no column is REAL.
fn real_wrap(rel: R, schema: &[Column], ty: &[SAff]) -> R {
    if !ty.contains(&SAff::Real) {
        return rel;
    }
    let exprs = ty
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let c = || Expr::Col { depth: 0, col: i as u16 };
            if *t != SAff::Real {
                return c();
            }
            Expr::Case {
                operand: Some(Box::new(Expr::Func(Func::Typeof, vec![c()]))),
                whens: vec![(Expr::Lit(Lit::Text("integer".into())), Expr::Cast(Box::new(c()), Aff::Real), (Aff::None, Aff::None))],
                else_: Some(Box::new(c())),
            }
        })
        .collect();
    Rel::Project { input: Box::new(rel), exprs, schema: schema.to_vec() }
}

/// A query read as a FROM subquery or CTE: a compound's REAL columns convert on read.
fn real_read(rel: R, schema: &[Column], qt: &QTy) -> R {
    if qt.compound { real_wrap(rel, schema, &qt.cols) } else { rel }
}

struct Item {
    name: String,
    schema: Vec<Column>,
    ty: Vec<SAff>,
    offset: usize,
}

struct Scope {
    level: usize,
    items: Vec<Item>,
    width: usize,
    aliases: Vec<(String, P)>,
    allow_agg: bool,
    aggs: Vec<AggCall<AnySource>>,
    wins: Vec<WindowFn<AnySource>>,
}

struct Cte {
    name: String,
    id: LocalId,
    schema: Vec<Column>,
    ty: Vec<SAff>,
}

pub struct Lowerer<'c> {
    catalog: &'c Catalog,
    scopes: Vec<Scope>,
    ctes: Vec<Cte>,
    level: usize,
    next_local: &'c mut LocalId,
}

fn lc(s: &str) -> String {
    s.to_ascii_lowercase()
}

fn find(schema: &[Column], name: &str) -> Option<usize> {
    schema.iter().position(|c| c.name.eq_ignore_ascii_case(name))
}

/// Lowers a statement (final query, reachability roots/edges) to a relation and its schema.
/// `next_local` numbers `WITH` bindings across the plan.
pub fn lower_query(q: &Query, catalog: &Catalog, next_local: &mut LocalId) -> Result<(R, Vec<Column>), String> {
    let mut l = Lowerer { catalog, scopes: Vec::new(), ctes: Vec::new(), level: 0, next_local };
    let (rel, schema, _) = l.query(q)?;
    Ok((rel, schema))
}

/// [`lower_query`] for a stage, whose rows are stored as SQLite's `CREATE TABLE … AS` stores
/// them. A column of FLEXNUM affinity is refused: CTAS stores it converting texts to numbers and
/// keeping integral reals, which no IR affinity expresses (NUMERIC would make `4.0` an integer).
pub fn lower_stage(q: &Query, catalog: &Catalog, next_local: &mut LocalId) -> Result<(R, Vec<Column>), String> {
    let mut l = Lowerer { catalog, scopes: Vec::new(), ctes: Vec::new(), level: 0, next_local };
    let (rel, schema, qt) = l.query(q)?;
    if let Some(i) = qt.cols.iter().position(|t| *t == SAff::Flex) {
        return Err(format!("stage column {}: FLEXNUM affinity (a compound whose left-most arm CASTs to a numeric type, or a column of one) stored by CREATE TABLE … AS", schema[i].name));
    }
    Ok((rel, schema))
}

/// Lowers a scalar expression over one row of `schema` (a `deferred` column): no subqueries.
pub fn lower_scalar(e: &P, schema: &[Column], next_local: &mut LocalId) -> Result<(E, Aff), String> {
    let catalog = Catalog::default();
    let mut l = Lowerer { catalog: &catalog, scopes: Vec::new(), ctes: Vec::new(), level: 0, next_local };
    l.scopes.push(Scope { level: 0, items: vec![Item { name: "__row".into(), schema: schema.to_vec(), ty: stored_types(schema), offset: 0 }], width: schema.len(), aliases: vec![], allow_agg: false, aggs: vec![], wins: vec![] });
    let (e, ty, _) = l.expr(e)?;
    if has_subquery(&e) {
        return Err("subquery in a deferred column".into());
    }
    Ok((e, ty.aff()))
}

fn has_subquery<S>(e: &Expr<S>) -> bool {
    let mut found = false;
    visit_expr(e, &mut |x| {
        if matches!(x, Expr::Exists { .. } | Expr::InRel { .. } | Expr::Scalar(_)) {
            found = true;
        }
    });
    found
}

/// Pre-order visit of an expression's nodes (not descending into subquery relations).
pub fn visit_expr<S>(e: &Expr<S>, f: &mut dyn FnMut(&Expr<S>)) {
    f(e);
    match e {
        Expr::Lit(_) | Expr::Col { .. } | Expr::Exists { .. } | Expr::Scalar(_) => {}
        Expr::Not(x) | Expr::Neg(x) | Expr::Plus(x) | Expr::Cast(x, _) => visit_expr(x, f),
        Expr::IsNull { e, .. } => visit_expr(e, f),
        Expr::Arith(_, a, b) | Expr::Concat(a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Cmp(_, a, b, _) => {
            visit_expr(a, f);
            visit_expr(b, f);
        }
        Expr::Is { left, right, .. } => {
            visit_expr(left, f);
            visit_expr(right, f);
        }
        Expr::Like { text, pattern, .. } => {
            visit_expr(text, f);
            visit_expr(pattern, f);
        }
        Expr::Func(_, args) => args.iter().for_each(|a| visit_expr(a, f)),
        Expr::Case { operand, whens, else_ } => {
            if let Some(o) = operand {
                visit_expr(o, f);
            }
            for (w, t, _) in whens {
                visit_expr(w, f);
                visit_expr(t, f);
            }
            if let Some(x) = else_ {
                visit_expr(x, f);
            }
        }
        Expr::InList { e, list, .. } => {
            visit_expr(e, f);
            list.iter().for_each(|(x, _)| visit_expr(x, f));
        }
        Expr::InRel { e, .. } => visit_expr(e, f),
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
        || s.having.as_ref().is_some_and(|e| expr_refs(e, name))
        || s.group_by.iter().any(|e| expr_refs(e, name))
        || s.items.iter().any(|i| matches!(i, SelItem::Expr { e, .. } if expr_refs(e, name)))
}
fn expr_refs(e: &P, name: &str) -> bool {
    match e {
        P::InSub { e, q, .. } => expr_refs(e, name) || query_refs(q, name),
        P::Exists { q, .. } | P::Sub(q) => query_refs(q, name),
        P::Unary(_, x) | P::Cast(x, _) | P::IsNull(x, _) => expr_refs(x, name),
        P::Bin(_, l, r) | P::Is(l, r, _) | P::Like(l, r, _) => expr_refs(l, name) || expr_refs(r, name),
        P::Between(a, b, c, _) => expr_refs(a, name) || expr_refs(b, name) || expr_refs(c, name),
        P::Func { args, over, .. } => {
            args.iter().any(|a| expr_refs(a, name))
                || over.as_ref().is_some_and(|w| w.partition.iter().any(|p| expr_refs(p, name)) || w.order.iter().any(|(o, _)| expr_refs(o, name)))
        }
        P::Case { operand, whens, else_ } => {
            operand.as_ref().is_some_and(|o| expr_refs(o, name))
                || whens.iter().any(|(w, t)| expr_refs(w, name) || expr_refs(t, name))
                || else_.as_ref().is_some_and(|x| expr_refs(x, name))
        }
        P::InList { e, list, .. } => expr_refs(e, name) || list.iter().any(|x| expr_refs(x, name)),
        _ => false,
    }
}

fn scalar_func(name: &str) -> Option<Func> {
    Some(match name {
        "coalesce" => Func::Coalesce,
        "ifnull" => Func::Ifnull,
        "iif" => Func::Iif,
        "nullif" => Func::Nullif,
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
        "abs" => Func::Abs,
        "typeof" => Func::Typeof,
        "printf" => Func::Printf,
        "max" => Func::Max,
        "min" => Func::Min,
        "json_quote" => Func::JsonQuote,
        "json_extract" => Func::JsonExtract,
        "json_array" => Func::JsonArray,
        "rx_matches" => Func::RxMatches,
        "rx_extract" => Func::RxExtract,
        "rx_replace" => Func::RxReplace,
        "source_text" => Func::SourceText,
        "rerail_refuse" => Func::Refuse,
        _ => return None,
    })
}

impl Lowerer<'_> {
    fn new_local(&mut self) -> LocalId {
        *self.next_local += 1;
        *self.next_local - 1
    }

    fn query(&mut self, q: &Query) -> Result<(R, Vec<Column>, QTy), String> {
        let cte_mark = self.ctes.len();
        let mut bindings = Vec::new();
        for cte in &q.ctes {
            if q.recursive && body_refs(&cte.q.body, &cte.name) {
                let (id, b) = self.recursive_cte(cte)?;
                bindings.push((id, b));
            } else {
                let (r, mut schema, qt) = self.query(&cte.q)?;
                if let Some(cols) = &cte.cols {
                    if cols.len() != schema.len() {
                        return Err(format!("CTE {} column count", cte.name));
                    }
                    for (c, n) in schema.iter_mut().zip(cols) {
                        c.name = n.clone();
                    }
                }
                let r = real_read(r, &schema, &qt);
                let id = self.new_local();
                self.ctes.push(Cte { name: lc(&cte.name), id, schema, ty: qt.cols });
                bindings.push((id, Binding::Plain(r)));
            }
        }
        let simple = matches!(q.body, Body::Select(_));
        let (body, mut out, n_keys, arms) = self.body(&q.body, if simple { &q.order } else { &[] })?;
        let qt = qty(&arms);
        for (c, t) in out.iter_mut().zip(&qt.cols) {
            c.aff = t.aff();
        }
        let mut keys = Vec::new();
        if simple {
            for (i, (_, desc)) in q.order.iter().enumerate() {
                keys.push(((out.len() + i) as u16, *desc));
            }
        } else {
            for (e, desc) in &q.order {
                let idx = match e {
                    P::Col { table: None, name } => find(&out, name).ok_or_else(|| format!("ORDER BY {name}"))?,
                    P::Lit(V::Int(i)) => (*i as usize).checked_sub(1).filter(|i| *i < out.len()).ok_or("ORDER BY ordinal")?,
                    _ => return Err("compound ORDER BY expression".into()),
                };
                keys.push((idx as u16, *desc));
            }
        }
        let limit = q.limit.as_ref().map(|e| self.const_expr(e)).transpose()?;
        let offset = q.offset.as_ref().map(|e| self.const_expr(e)).transpose()?;
        let mut rel = body;
        if !keys.is_empty() || limit.is_some() || offset.is_some() {
            rel = Rel::Sort { input: Box::new(rel), keys, limit, offset, total: Total::InputOrder };
        }
        if n_keys > 0 {
            let exprs = (0..out.len()).map(|i| Expr::Col { depth: 0, col: i as u16 }).collect();
            rel = Rel::Project { input: Box::new(rel), exprs, schema: out.clone() };
        }
        self.ctes.truncate(cte_mark);
        if !bindings.is_empty() {
            rel = Rel::With { bindings, body: Box::new(rel) };
        }
        Ok((rel, out, qt))
    }

    fn const_expr(&mut self, e: &P) -> Result<E, String> {
        self.scopes.push(Scope { level: self.level, items: vec![], width: 0, aliases: vec![], allow_agg: false, aggs: vec![], wins: vec![] });
        let r = self.expr(e);
        self.scopes.pop();
        Ok(r?.0)
    }

    /// A recursive CTE's step arms lowered with the CTE's columns typed `ty` (the CTE in scope
    /// while they lower): their relations and output columns' typing.
    fn lower_steps(&mut self, name: &str, id: LocalId, schema: &[Column], ty: &[SAff], steps: &[&Body]) -> Result<(Vec<R>, Vec<Vec<ArmCol>>), String> {
        let schema: Vec<Column> = schema.iter().zip(ty).map(|(c, t)| Column { name: c.name.clone(), aff: t.aff() }).collect();
        self.ctes.push(Cte { name: lc(name), id, schema: schema.clone(), ty: ty.to_vec() });
        let r = (|| {
            let (mut rels, mut arms) = (Vec::new(), Vec::new());
            for a in steps {
                let (r, s, _, ac) = self.body(a, &[])?;
                if s.len() != schema.len() {
                    return Err("recursive arm width".to_string());
                }
                rels.push(r);
                arms.extend(ac);
            }
            Ok((rels, arms))
        })();
        self.ctes.pop();
        r
    }

    fn recursive_cte(&mut self, cte: &parse::Cte) -> Result<(LocalId, Binding<AnySource>), String> {
        if !cte.q.order.is_empty() || cte.q.limit.is_some() || !cte.q.ctes.is_empty() {
            return Err("recursive CTE with ORDER BY/LIMIT/WITH".into());
        }
        let mut arms = Vec::new();
        let mut ops = Vec::new();
        flatten(&cte.q.body, &mut arms, &mut ops);
        let first_rec = arms.iter().position(|a| body_refs(a, &cte.name)).ok_or("recursive CTE without a recursive arm")?;
        if first_rec == 0 {
            return Err("recursive CTE starting with a recursive arm".into());
        }
        let set = ops[first_rec - 1] == SetOp::Union;
        if ops.iter().any(|o| *o != ops[first_rec - 1]) {
            return Err("recursive CTE mixing UNION and UNION ALL".into());
        }
        let mut init = Vec::new();
        let mut init_arms: Vec<Vec<ArmCol>> = Vec::new();
        let mut schema = None;
        for a in &arms[..first_rec] {
            let (r, s, _, ac) = self.body(a, &[])?;
            schema.get_or_insert(s);
            init.push(r);
            init_arms.extend(ac);
        }
        let mut schema = schema.unwrap();
        if let Some(cols) = &cte.cols {
            if cols.len() != schema.len() {
                return Err(format!("CTE {} column count", cte.name));
            }
            for (c, n) in schema.iter_mut().zip(cols) {
                c.name = n.clone();
            }
        }
        let steps = &arms[first_rec..];
        if !steps.iter().all(|a| body_refs(a, &cte.name)) {
            return Err("non-recursive arm after a recursive arm".into());
        }
        let id = self.new_local();
        // Column types over every arm, as SQLite computes them column by column while the steps'
        // references to the CTE read its columns as typed so far: an earlier column's final
        // type, a later one's none; the column itself none while the arms are walked
        // (`walk_arms`), then the walk's affinity (`settle`).
        let w = schema.len();
        let mut ty = vec![SAff::None; w];
        for i in 0..w {
            let col = |arms: &[Vec<ArmCol>]| arms.iter().map(|a| a[i]).collect::<Vec<_>>();
            let now = |ty: &[SAff], own: SAff| (0..w).map(|j| if j < i { ty[j] } else if j == i { own } else { SAff::None }).collect::<Vec<_>>();
            let (_, walked) = self.lower_steps(&cte.name, id, &schema, &now(&ty, SAff::None), steps)?;
            let walked = col(&[init_arms.clone(), walked].concat());
            let (aff, j, m) = walk_arms(&walked);
            ty[i] = if aff >= SAff::Text {
                let (_, post) = self.lower_steps(&cte.name, id, &schema, &now(&ty, aff), steps)?;
                settle(aff, j, m, &col(&[init_arms.clone(), post].concat()))
            } else {
                aff
            };
        }
        let (step, _) = self.lower_steps(&cte.name, id, &schema, &ty, steps)?;
        for (c, t) in schema.iter_mut().zip(&ty) {
            c.aff = t.aff();
        }
        // Rows are read through the CTE's REAL columns (every arm's, so the step reads the
        // previous level converted too).
        let init = init.into_iter().map(|r| real_wrap(r, &schema, &ty)).collect();
        let step = step.into_iter().map(|r| real_wrap(r, &schema, &ty)).collect();
        // The binding stays in scope: outside the step, `Local(id)` is the fixpoint's result.
        self.ctes.push(Cte { name: lc(&cte.name), id, schema: schema.clone(), ty });
        Ok((id, Binding::Fix { init, step, set, schema }))
    }

    /// A query body; returns its relation, the visible schema, how many `ORDER BY` key columns
    /// ride behind it (simple selects only), and per arm (in text order) its columns' typing.
    fn body(&mut self, b: &Body, order: &[(P, bool)]) -> Result<Lowered, String> {
        match b {
            Body::Select(s) => {
                let (r, out, n, arm) = self.select(s, order)?;
                Ok((r, out, n, vec![arm]))
            }
            Body::Compound(l, op, r) => {
                let (lr, ls, _, mut la) = self.body(l, &[])?;
                let (rr, rs, _, ra) = self.body(r, &[])?;
                if ls.len() != rs.len() {
                    return Err("compound arms differ in width".into());
                }
                la.extend(ra);
                Ok((Rel::Union { all: *op == SetOp::UnionAll, left: Box::new(lr), right: Box::new(rr) }, ls, 0, la))
            }
        }
    }

    fn source(&mut self, src: &Source) -> Result<(R, Vec<Column>, Vec<SAff>, String), String> {
        match src {
            Source::Table(name) => {
                let l = lc(name);
                if let Some(c) = self.ctes.iter().rev().find(|c| c.name == l) {
                    return Ok((Rel::Local { id: c.id, schema: c.schema.clone() }, c.schema.clone(), c.ty.clone(), name.clone()));
                }
                let (src, schema) = self.catalog.get(&l).ok_or_else(|| format!("no such table: {name}"))?.clone();
                let ty = stored_types(&schema);
                Ok((Rel::Scan { src, schema: schema.clone() }, schema, ty, name.clone()))
            }
            Source::Sub(q) => {
                let (r, s, qt) = self.query(q)?;
                Ok((real_read(r, &s, &qt), s, qt.cols, String::new()))
            }
            Source::Func(f, _) => Err(format!("table function {f}")),
        }
    }

    fn top(&mut self) -> &mut Scope {
        self.scopes.last_mut().unwrap()
    }

    fn select(&mut self, s: &parse::Select, order: &[(P, bool)]) -> Result<(R, Vec<Column>, usize, Vec<ArmCol>), String> {
        self.scopes.push(Scope { level: self.level, items: vec![], width: 0, aliases: vec![], allow_agg: false, aggs: vec![], wins: vec![] });
        let r = self.select_in_scope(s, order);
        self.scopes.pop();
        r
    }

    fn push_item(&mut self, name: String, schema: Vec<Column>, ty: Vec<SAff>) {
        let sc = self.top();
        let offset = sc.width;
        sc.width += schema.len();
        sc.items.push(Item { name: lc(&name), schema, ty, offset });
    }

    /// A simple select: its relation, its columns, how many `ORDER BY` keys ride behind them, and
    /// its columns' typing.
    fn select_in_scope(&mut self, s: &parse::Select, order: &[(P, bool)]) -> Result<(R, Vec<Column>, usize, Vec<ArmCol>), String> {
        let mut acc: Option<R> = None;
        let mut first_on: Option<E> = None;
        for fi in &s.from {
            let left = fi.kind == parse::JoinKind::Left;
            if let Source::Func(f, args) = &fi.source {
                if !f.eq_ignore_ascii_case("json_each") || args.len() != 1 {
                    return Err(format!("table function {f}"));
                }
                let arg = self.expr(&args[0])?.0;
                self.push_item(fi.alias.clone().unwrap_or_else(|| f.clone()), json_each_schema(), vec![SAff::None; 8]);
                let on = fi.on.as_ref().map(|e| self.expr(e).map(|x| x.0)).transpose()?;
                acc = Some(Rel::JsonEach { input: Box::new(acc.take().unwrap_or(Rel::Unit)), arg, on, outer: left });
                continue;
            }
            let (rel, schema, ty, name) = self.source(&fi.source)?;
            self.push_item(fi.alias.clone().unwrap_or(name), schema, ty);
            let on = fi.on.as_ref().map(|e| self.expr(e).map(|x| x.0)).transpose()?;
            acc = Some(match acc.take() {
                None => {
                    first_on = on;
                    rel
                }
                Some(l) => Rel::Join { kind: if left { JoinKind::Left } else { JoinKind::Inner }, left: Box::new(l), right: Box::new(rel), on },
            });
        }
        let mut base = acc.unwrap_or(Rel::Unit);
        if let Some(on) = first_on {
            base = Rel::Filter { input: Box::new(base), pred: on };
        }
        let aliases = s.items.iter().filter_map(|it| match it {
            SelItem::Expr { e, alias: Some(a), .. } => Some((lc(a), e.clone())),
            _ => None,
        });
        let aliases: Vec<_> = aliases.collect();
        self.top().aliases = aliases;
        if let Some(w) = &s.where_ {
            let pred = self.expr(w)?.0;
            base = Rel::Filter { input: Box::new(base), pred };
        }
        let keys = s.group_by.iter().map(|g| self.expr(g).map(|x| x.0)).collect::<Result<Vec<_>, _>>()?;
        self.top().allow_agg = true;
        let mut proj = Vec::new();
        let mut out = Vec::new();
        let mut arm = Vec::new();
        for it in &s.items {
            match it {
                SelItem::Star | SelItem::TableStar(_) => {
                    let sc = self.scopes.last().unwrap();
                    let items: Vec<&Item> = match it {
                        SelItem::TableStar(t) => vec![sc.items.iter().find(|x| x.name == lc(t)).ok_or_else(|| format!("no such table: {t}"))?],
                        _ => sc.items.iter().collect(),
                    };
                    for item in items {
                        for (ci, c) in item.schema.iter().enumerate() {
                            proj.push(Expr::Col { depth: 0, col: (item.offset + ci) as u16 });
                            out.push(c.clone());
                            arm.push(ArmCol { ty: item.ty[ci], dt: item.ty[ci].data_type(), cast: false });
                        }
                    }
                }
                SelItem::Expr { e, alias, text } => {
                    let (pe, ty, dt) = self.expr(e)?;
                    let name = match (alias, e, &pe) {
                        (Some(a), _, _) => a.clone(),
                        (None, P::Col { .. }, Expr::Col { depth: 0, col }) => self.col_name(*col as usize),
                        (None, P::Col { name, .. }, _) => name.clone(),
                        (None, _, _) => text.clone(),
                    };
                    out.push(Column { name, aff: ty.aff() });
                    arm.push(ArmCol { ty, dt, cast: matches!(e, P::Cast(..)) });
                    proj.push(pe);
                }
            }
        }
        let having = s.having.as_ref().map(|h| self.expr(h).map(|x| x.0)).transpose()?;
        let mut order_keys = Vec::new();
        for (e, _) in order {
            let pe = match e {
                P::Lit(V::Int(i)) => proj.get((*i as usize).wrapping_sub(1)).cloned().ok_or("ORDER BY ordinal")?,
                P::Col { table: None, name } if find(&out, name).is_some() && self.resolve(None, name).is_err() => proj[find(&out, name).unwrap()].clone(),
                _ => self.expr(e)?.0,
            };
            order_keys.push(pe);
        }
        let sc = self.scopes.last_mut().unwrap();
        let aggs = std::mem::take(&mut sc.aggs);
        let wins = std::mem::take(&mut sc.wins);
        // HAVING without GROUP BY makes an aggregate select (one group), as in SQLite 3.39+.
        let grouped = !keys.is_empty() || !aggs.is_empty() || having.is_some();
        let mut rel = if grouped {
            if !wins.is_empty() {
                return Err("window function in an aggregate select".into());
            }
            let mut r = Rel::Aggregate { input: Box::new(base), keys, aggs };
            if let Some(h) = having {
                r = Rel::Filter { input: Box::new(r), pred: h };
            }
            r
        } else {
            if wins.is_empty() { base } else { Rel::Window { input: Box::new(base), funcs: wins } }
        };
        let n_keys = order_keys.len();
        let mut schema = out.clone();
        for (i, _) in order_keys.iter().enumerate() {
            schema.push(Column { name: format!("__order_{i}"), aff: Aff::None });
        }
        proj.extend(order_keys);
        rel = Rel::Project { input: Box::new(rel), exprs: proj, schema };
        if s.distinct {
            rel = Rel::Distinct { input: Box::new(rel), prefix: out.len() as u16 };
        }
        Ok((rel, out, n_keys, arm))
    }

    fn col_name(&self, col: usize) -> String {
        let sc = self.scopes.last().unwrap();
        for it in &sc.items {
            if col >= it.offset && col < it.offset + it.schema.len() {
                return it.schema[col - it.offset].name.clone();
            }
        }
        String::new()
    }

    /// Resolves a column: the top-most scope of each level is visible, innermost level first.
    fn resolve(&self, table: Option<&str>, name: &str) -> Result<(E, SAff), String> {
        let mut seen_level: Option<usize> = None;
        for sc in self.scopes.iter().rev() {
            if seen_level.is_some_and(|l| sc.level >= l) {
                // A lower scope on an already visited level: a FROM sibling (lateral).
                if let Some(t) = table {
                    if sc.items.iter().any(|it| it.name == lc(t)) {
                        return Err(format!("lateral reference {t}.{name}"));
                    }
                }
                continue;
            }
            seen_level = Some(sc.level);
            let found = match table {
                Some(t) => sc.items.iter().find(|it| it.name == lc(t)).map(|it| (it, find(&it.schema, name))),
                None => sc.items.iter().find_map(|it| find(&it.schema, name).map(|c| (it, Some(c)))),
            };
            if let Some((it, col)) = found {
                let col = col.ok_or_else(|| format!("no such column: {}.{name}", table.unwrap_or("")))?;
                let depth = (self.level - sc.level) as u16;
                return Ok((Expr::Col { depth, col: (it.offset + col) as u16 }, it.ty[col]));
            }
            if sc.level == 0 {
                break;
            }
        }
        Err(format!("no such column: {}{name}", table.map(|t| format!("{t}.")).unwrap_or_default()))
    }

    fn subquery(&mut self, q: &Query) -> Result<(R, Vec<Column>, QTy), String> {
        self.level += 1;
        let r = self.query(q);
        self.level -= 1;
        r
    }

    fn cmp(&mut self, op: CmpOp, l: &P, r: &P) -> Result<E, String> {
        let (pl, al, _) = self.expr(l)?;
        let (pr, ar, _) = self.expr(r)?;
        let aff = operands_aff(&pl, al, &pr, ar);
        Ok(Expr::Cmp(op, Box::new(pl), Box::new(pr), aff))
    }

    /// An expression, its affinity (`sqlite3ExprAffinity`) and the storage classes it may hold
    /// (`sqlite3ExprDataType`: 0x01 numeric, 0x02 text, 0x04 blob, 0 for NULL).
    fn expr(&mut self, e: &P) -> Result<(E, SAff, u8), String> {
        // An operator's result: no affinity, a number.
        let num = |x: E| Ok((x, SAff::None, 0x01));
        match e {
            P::Lit(v) => {
                let dt = match v {
                    V::Null => 0,
                    V::Text(_) => 0x02,
                    V::Int(_) | V::Real(_) => 0x01,
                };
                Ok((Expr::Lit(lit(v)), SAff::None, dt))
            }
            P::Col { table, name } => match self.resolve(table.as_deref(), name) {
                Ok((e, ty)) => Ok((e, ty, ty.data_type())),
                Err(err) => {
                    if table.is_none() {
                        let alias = self.scopes.last().and_then(|s| s.aliases.iter().find(|(a, _)| *a == lc(name)).map(|(_, e)| e.clone()));
                        if let Some(ae) = alias {
                            return self.expr(&ae);
                        }
                    }
                    Err(err)
                }
            },
            P::Unary(op, x) => {
                let (px, _, dt) = self.expr(x)?;
                match *op {
                    "not" => num(Expr::Not(Box::new(px))),
                    "-" => num(Expr::Neg(Box::new(px))),
                    // Unary `+` drops the affinity; the data type is its operand's.
                    _ => Ok((Expr::Plus(Box::new(px)), SAff::None, dt)),
                }
            }
            P::Bin(op, l, r) => {
                let cmp = match *op {
                    "=" => Some(CmpOp::Eq),
                    "!=" => Some(CmpOp::Ne),
                    "<" => Some(CmpOp::Lt),
                    "<=" => Some(CmpOp::Le),
                    ">" => Some(CmpOp::Gt),
                    ">=" => Some(CmpOp::Ge),
                    _ => None,
                };
                if let Some(c) = cmp {
                    return num(self.cmp(c, l, r)?);
                }
                let pl = self.expr(l)?.0;
                let pr = self.expr(r)?.0;
                let (pl, pr) = (Box::new(pl), Box::new(pr));
                if *op == "||" {
                    return Ok((Expr::Concat(pl, pr), SAff::None, 0x06));
                }
                num(match *op {
                    "and" => Expr::And(pl, pr),
                    "or" => Expr::Or(pl, pr),
                    "+" => Expr::Arith(ArithOp::Add, pl, pr),
                    "-" => Expr::Arith(ArithOp::Sub, pl, pr),
                    "*" => Expr::Arith(ArithOp::Mul, pl, pr),
                    "/" => Expr::Arith(ArithOp::Div, pl, pr),
                    "%" => Expr::Arith(ArithOp::Rem, pl, pr),
                    o => return Err(format!("operator {o}")),
                })
            }
            P::Func { name, args, star, distinct, over } => self.func(name, args, *star, *distinct, over.as_ref()),
            P::Case { operand, whens, else_ } => {
                let po = operand.as_ref().map(|o| self.expr(o)).transpose()?;
                let mut pw = Vec::new();
                let mut dt = 0;
                for (w, t) in whens {
                    let (pwv, wa, _) = self.expr(w)?;
                    let aff = po.as_ref().map_or((Aff::None, Aff::None), |(o, oa, _)| operands_aff(o, *oa, &pwv, wa));
                    let (pt, _, tdt) = self.expr(t)?;
                    dt |= tdt;
                    pw.push((pwv, pt, aff));
                }
                let pe = match else_ {
                    Some(x) => {
                        let (px, _, xdt) = self.expr(x)?;
                        dt |= xdt;
                        Some(Box::new(px))
                    }
                    None => None,
                };
                Ok((Expr::Case { operand: po.map(|o| Box::new(o.0)), whens: pw, else_: pe }, SAff::None, dt))
            }
            P::Cast(x, ty) => {
                let aff = Aff::of_type(ty);
                // `CAST(… AS BLOB)` has BLOB affinity.
                let sa = if aff == Aff::None { SAff::Blob } else { SAff::of(aff) };
                let pe = match (&**x, aff) {
                    (P::Blob(text), Aff::Text) => Expr::Cast(Box::new(Expr::Lit(Lit::Text(text.as_str().into()))), Aff::Text),
                    _ => Expr::Cast(Box::new(self.expr(x)?.0), aff),
                };
                Ok((pe, sa, sa.data_type()))
            }
            P::InList { e, list, not } => {
                let (pe, la, _) = self.expr(e)?;
                let la = la.aff();
                let mut pl = Vec::new();
                for x in list {
                    let px = self.expr(x)?.0;
                    // `x IN (list)`: the left operand's affinity governs the comparisons.
                    pl.push((px, if la == Aff::None { (Aff::None, Aff::None) } else { (Aff::None, la) }));
                }
                num(Expr::InList { e: Box::new(pe), list: pl, negated: *not })
            }
            P::InSub { e, q, not } => {
                let (pe, la, _) = self.expr(e)?;
                let (rel, schema, qt) = self.subquery(q)?;
                if schema.len() != 1 {
                    return Err("IN subquery must return one column".into());
                }
                let aff = if compound(&rel) || loose(&pe) { cmp_aff_both(la, qt.first) } else { cmp_aff(la, qt.first) };
                num(Expr::InRel { e: Box::new(pe), rel: Box::new(rel), negated: *not, aff })
            }
            P::Exists { q, not } => {
                let (rel, _, _) = self.subquery(q)?;
                num(Expr::Exists { rel: Box::new(rel), negated: *not })
            }
            P::Sub(q) => {
                let (rel, _, qt) = self.subquery(q)?;
                Ok((Expr::Scalar(Box::new(rel)), qt.first, qt.first.data_type()))
            }
            P::IsNull(x, not) => num(Expr::IsNull { e: Box::new(self.expr(x)?.0), negated: *not }),
            P::Is(l, r, not) => {
                let pl = self.expr(l)?.0;
                let pr = self.expr(r)?.0;
                num(Expr::Is { left: Box::new(pl), right: Box::new(pr), negated: *not })
            }
            P::Like(l, r, not) => {
                let pl = self.expr(l)?.0;
                let pr = self.expr(r)?.0;
                num(Expr::Like { text: Box::new(pl), pattern: Box::new(pr), negated: *not })
            }
            P::Between(x, lo, hi, not) => {
                // `x BETWEEN lo AND hi` ≡ `x >= lo AND x <= hi` (NOT: `x < lo OR x > hi`).
                let (a, b) = if *not { (CmpOp::Lt, CmpOp::Gt) } else { (CmpOp::Ge, CmpOp::Le) };
                let l = self.cmp(a, x, lo)?;
                let r = self.cmp(b, x, hi)?;
                num(if *not { Expr::Or(Box::new(l), Box::new(r)) } else { Expr::And(Box::new(l), Box::new(r)) })
            }
            P::Blob(_) => Err("blob literal outside CAST(X'..' AS TEXT)".into()),
        }
    }

    /// A function call: no affinity, any storage class (`sqlite3ExprDataType`'s 0x07).
    fn func(&mut self, name: &str, args: &[P], star: bool, distinct: bool, over: Option<&parse::Window>) -> Result<(E, SAff, u8), String> {
        if let Some(w) = over {
            let kind_arg = match (name, args.len()) {
                ("row_number", 0) => None,
                ("min" | "max", 1) => Some(&args[0]),
                _ => return Err(format!("window function {name}")),
            };
            if kind_arg.is_some() && !w.order.is_empty() {
                return Err(format!("window {name}() with ORDER BY (running frame)"));
            }
            let save = self.top().allow_agg;
            self.top().allow_agg = false;
            let r: Result<WindowFn<AnySource>, String> = (|| {
                let kind = match kind_arg {
                    None => WindowKind::RowNumber,
                    Some(a) if name == "min" => WindowKind::Min(self.expr(a)?.0),
                    Some(a) => WindowKind::Max(self.expr(a)?.0),
                };
                let partition = w.partition.iter().map(|p| self.expr(p).map(|x| x.0)).collect::<Result<Vec<_>, _>>()?;
                let order = w.order.iter().map(|(o, d)| Ok((self.expr(o)?.0, *d))).collect::<Result<Vec<_>, String>>()?;
                Ok(WindowFn { kind, partition, order, total: Total::InputOrder })
            })();
            self.top().allow_agg = save;
            let wf = r?;
            let sc = self.top();
            sc.wins.push(wf);
            let col = (sc.width + sc.wins.len() - 1) as u16;
            return Ok((Expr::Col { depth: 0, col }, SAff::None, 0x07));
        }
        let agg = match (name, args.len(), star) {
            ("count", 0, true) => Some(AggFn::CountStar),
            ("count", 1, false) => Some(AggFn::Count),
            ("min", 1, false) => Some(AggFn::Min),
            ("max", 1, false) => Some(AggFn::Max),
            ("group_concat", 1 | 2, false) => Some(AggFn::GroupConcat),
            ("json_group_array", 1, false) => Some(AggFn::JsonGroupArray),
            ("sum", 1, false) => Some(AggFn::Sum),
            ("total", 1, false) => Some(AggFn::Total),
            _ => None,
        };
        if let Some(f) = agg {
            if !self.scopes.last().is_some_and(|s| s.allow_agg) {
                return Err(format!("misuse of aggregate {name}"));
            }
            self.top().allow_agg = false;
            let arg = args.first().map(|a| self.expr(a).map(|x| x.0)).transpose();
            let sep = args.get(1).map(|a| self.expr(a).map(|x| x.0)).transpose();
            self.top().allow_agg = true;
            let sc = self.top();
            sc.aggs.push(AggCall { f, arg: arg?, sep: sep?, distinct, total: Total::InputOrder });
            let col = (sc.width + sc.aggs.len() - 1) as u16;
            return Ok((Expr::Col { depth: 0, col }, SAff::None, 0x07));
        }
        if distinct || star {
            return Err(format!("{name}(DISTINCT/*) outside an aggregate"));
        }
        let f = scalar_func(name).ok_or_else(|| format!("function {name}"))?;
        let pargs = args.iter().map(|a| self.expr(a).map(|x| x.0)).collect::<Result<Vec<_>, _>>()?;
        Ok((Expr::Func(f, pargs), SAff::None, 0x07))
    }
}
