//! Physical plans for matcher inputs lowered to the algebra (INVARIANTS I3, I5).
//!
//! `rerail_facts::front::pattern` lowers a matcher input to `Lowered`: rows =
//! `Project(LEFT JOIN … (Filter(file ⋈ node, keep)), exprs)` over one content's core relations,
//! the captures as key joins on `node.id`. This module plans that algebra once per rule onto
//! physical operators over the facts' columns, and refuses (the input then runs on the leaf
//! executor, `eval::Input::rows`) anything outside them:
//!
//! - **Selection, set at a time.** `keep` is two-valued (no `value` test: no NULL source in the
//!   predicate), so it is evaluated as bitsets over the file's nodes: kind tests from the kind
//!   postings, regexes over node text only on the nodes still undecided (`AND` narrows, `OR`
//!   skips decided nodes), relation searches (`EXISTS` over a declared tree axis, recognised by
//!   its predicate from `pattern::axis_pred`: children / parent key lookups, descendants and
//!   ancestors over the preorder interval property, the first child under a field, stop rules)
//!   as one pass over the tree evaluating the inner predicate once per candidate.
//! - **Memoised per-node predicates.** Predicates are hash-consed per rule (one policy set), and
//!   each one's truth is memoised per file over the nodes it was evaluated on, so a predicate
//!   shared by several inputs or clauses (a selection's gate, a utility) is evaluated once per
//!   node per file: the union of the clause's selections in one pass over `node`, without
//!   naming any input, stage or clause.
//! - **Captures** (`x.id = (first candidate on an axis where its rule holds)`): per selected
//!   root, read from the facts (parent, first field child, nearest ancestor, first descendant)
//!   with the inner predicate's bitset.
//!
//! Rows, their order (node preorder), values and anchors equal `Input::rows`' (checked by
//! `pattern-check`, and by the gate's differential against `FACTS_REWRITES=0`).
use crate::eval::Rows;
use crate::facts::{self, Facts, NONE};
use crate::sql::engine::{Column, Schema, Table, TextCell, U32_NULL};
use crate::sql::value::V;
use regex::Regex;
use rerail_facts::front::pattern::{self as fp, Axis};
use rerail_facts::ir::{ArithOp, CmpOp, CoreRel, Expr, FileSource, Func, JoinKind, Lit, Lowered, Rel};
use rustc_hash::FxHashMap;
use std::rc::Rc;

type E = Expr<FileSource>;
type R = Rel<FileSource>;

// The candidate row: `file ⋈ node`.
const PATH: u16 = 0;
const ID: u16 = 1;
const KIND: u16 = 2;
const START: u16 = 4;
const END: u16 = 5;
const NAMED: u16 = 9;
/// Columns of the candidate row, and of one capture's `node` join.
const ROW_W: u16 = 12;
const NODE_W: u16 = 11;

pub type Id = u32;

/// A physical predicate over one file's nodes, hash-consed per rule. Three-valued only through
/// `Value` (NULL where a literal's value is unknown); everything else is two-valued.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Pn {
    Const(bool),
    /// `node.kind = k` (named or not).
    Kind(u32),
    KindIn(Vec<u32>),
    Named,
    TextEq(Box<str>),
    /// Index into `Preds::regexes`.
    TextRx(u32),
    PathRx(u32),
    /// `[NOT] EXISTS syntax_error()`: holds iff the file's syntax-error flag differs from the bool.
    SyntaxError(bool),
    And(Vec<Id>),
    Or(Vec<Id>),
    Not(Id),
    /// TRUE if a candidate holds, else NULL if one is NULL, else FALSE.
    Exists(Ax, Id),
    /// A named string literal's `str` value: equal to `eq` and matching the regex (index into
    /// `Preds::regexes`); NULL when its value is unknown; FALSE for bytes and other nodes.
    Value(Option<Box<str>>, Option<u32>),
}

/// A relation search's axis (`pattern::Axis`), field codes resolved; `*Until` carry the stop
/// predicate.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Ax {
    Children,
    Descendants,
    DescendantsUntil(Id),
    Parent,
    Ancestors,
    AncestorsUntil(Id),
    FieldChild(u16),
    FieldSubtree(u16),
    InsideField(u16),
    InsideFieldEnd(u16),
}

/// One rule's physical predicates (shared by its inputs' plans).
#[derive(Default)]
pub struct Preds {
    nodes: Vec<Pn>,
    index: FxHashMap<Pn, Id>,
    regexes: Vec<Regex>,
    rx_index: FxHashMap<String, u32>,
}

/// A capture: the first candidate on `ax` where `inner` holds (search order).
struct Cap {
    ax: Ax,
    inner: Id,
}

#[derive(Clone, Copy)]
enum NodeCol {
    Id,
    Kind,
    Start,
    End,
}

/// `(SELECT value FROM string_value WHERE id = <root id>)`.
fn is_decoded(e: &E) -> bool {
    let Expr::Scalar(r) = e else { return false };
    let Rel::Project { input, exprs, .. } = &**r else { return false };
    let Rel::Filter { input: s, pred } = &**input else { return false };
    exprs.len() == 1
        && is_col(&exprs[0], 1)
        && matches!(&**s, Rel::Scan { src: FileSource::Core(CoreRel::StringValue), .. })
        && matches!(pred, Expr::Cmp(CmpOp::Eq, a, b, _) if is_col(a, 0) && matches!(**b, Expr::Col { depth: 1, col: ID }))
}

/// An output column. `u8`: 0 = the root, k = the k-th capture.
enum Out {
    Path,
    Int(i64),
    Node(u8, NodeCol),
    Text(u8),
    Line(u8),
    Coalesce(Box<Out>, Box<Out>),
    RxExtract(Box<Out>, Regex, usize),
    /// The root's decoded `str` value, `''` when it has none (`COALESCE((SELECT value FROM
    /// string_value WHERE id = root), '')`).
    Decoded,
}

/// One input's physical plan.
pub struct Plan {
    keep: Id,
    caps: Vec<Cap>,
    anchor: (Out, Out),
    outs: Vec<Out>,
    /// `refuse_if = [syntax_error()]`: the query has no `syntax` and the file has errors.
    refuse_syntax: bool,
    /// A `value` test's `\N{...}` refusal (`refuse_if = [nodes WHERE reach AND named AND escape]`).
    /// It needs a literal holding `\N{`; a file whose source has none satisfies it vacuously, so
    /// the plan runs only there (side condition checked per file; elsewhere the leaf executor).
    escape_refusal: bool,
}

fn dbg<T: std::fmt::Debug>(x: &T) -> String {
    format!("{x:?}")
}

fn is_rows(r: &R) -> bool {
    dbg(r) == dbg(&fp::nodes())
}

fn is_col(e: &E, c: u16) -> bool {
    matches!(e, Expr::Col { depth: 0, col } if *col == c)
}

fn lit_text(e: &E) -> Option<&str> {
    match e {
        Expr::Lit(Lit::Text(t)) => Some(t),
        _ => None,
    }
}

fn node_text_of(e: &E, start: u16, end: u16) -> bool {
    matches!(e, Expr::Func(Func::SourceText, a) if a.len() == 3 && is_col(&a[0], PATH) && is_col(&a[1], start) && is_col(&a[2], end))
}

/// Text literals in an expression (field-name candidates of a field axis).
fn texts(e: &E, out: &mut Vec<String>) {
    fn rel(r: &R, out: &mut Vec<String>) {
        match r {
            Rel::Filter { input, pred } => {
                rel(input, out);
                texts(pred, out);
            }
            Rel::Join { left, right, on, .. } => {
                rel(left, out);
                rel(right, out);
                if let Some(o) = on {
                    texts(o, out);
                }
            }
            _ => {}
        }
    }
    match e {
        Expr::Lit(Lit::Text(t)) => {
            if !out.iter().any(|x| **x == **t) {
                out.push(t.to_string());
            }
        }
        Expr::And(a, b) | Expr::Or(a, b) => {
            texts(a, out);
            texts(b, out);
        }
        Expr::Not(a) => texts(a, out),
        Expr::Cmp(_, a, b, _) => {
            texts(a, out);
            texts(b, out);
        }
        Expr::Exists { rel: r, .. } => rel(r, out),
        _ => {}
    }
}

fn field_code(name: &str) -> Option<u16> {
    facts::GRAMMAR.field_code(name)
}

impl Preds {
    fn intern(&mut self, p: Pn) -> Id {
        if let Some(&i) = self.index.get(&p) {
            return i;
        }
        let i = self.nodes.len() as Id;
        self.nodes.push(p.clone());
        self.index.insert(p, i);
        i
    }

    fn regex(&mut self, p: &str) -> Result<u32, String> {
        if let Some(&i) = self.rx_index.get(p) {
            return Ok(i);
        }
        let r = crate::rx::get(p).map_err(|e| e.to_string())?;
        let i = self.regexes.len() as u32;
        self.regexes.push(r);
        self.rx_index.insert(p.to_owned(), i);
        Ok(i)
    }

    /// A two-valued predicate over the candidate row (depth 0).
    fn pred(&mut self, e: &E) -> Result<Id, String> {
        let p = match e {
            Expr::Lit(Lit::Int(1)) => Pn::Const(true),
            Expr::Lit(Lit::Int(0)) => Pn::Const(false),
            Expr::And(..) | Expr::Or(..) => {
                let and = matches!(e, Expr::And(..));
                let mut items = Vec::new();
                let mut stack = vec![e];
                // Flattened in written (left-to-right) order.
                let mut flat = Vec::new();
                while let Some(x) = stack.pop() {
                    match (x, and) {
                        (Expr::And(a, b), true) | (Expr::Or(a, b), false) => {
                            stack.push(b);
                            stack.push(a);
                        }
                        _ => flat.push(x),
                    }
                }
                for x in flat {
                    items.push(self.pred(x)?);
                }
                if and { Pn::And(items) } else { Pn::Or(items) }
            }
            Expr::Not(a) => Pn::Not(self.pred(a)?),
            Expr::Cmp(CmpOp::Eq, a, b, _) if is_col(a, KIND) && lit_text(b).is_some() => Pn::Kind(facts::intern(lit_text(b).unwrap())),
            Expr::Cmp(CmpOp::Eq, a, b, _) if is_col(a, NAMED) && matches!(**b, Expr::Lit(Lit::Int(1))) => Pn::Named,
            Expr::Cmp(CmpOp::Eq, a, b, _) if node_text_of(a, START, END) && lit_text(b).is_some() => Pn::TextEq(lit_text(b).unwrap().into()),
            Expr::InList { e: x, list, negated: false } if is_col(x, KIND) && list.iter().all(|(l, _)| lit_text(l).is_some()) => {
                Pn::KindIn(list.iter().map(|(l, _)| facts::intern(lit_text(l).unwrap())).collect())
            }
            Expr::Func(Func::RxMatches, a) if a.len() == 2 && node_text_of(&a[0], START, END) && lit_text(&a[1]).is_some() => Pn::TextRx(self.regex(lit_text(&a[1]).unwrap())?),
            Expr::Func(Func::RxMatches, a) if a.len() == 2 && is_col(&a[0], PATH) && lit_text(&a[1]).is_some() => Pn::PathRx(self.regex(lit_text(&a[1]).unwrap())?),
            Expr::Exists { rel, negated } if matches!(&**rel, Rel::Scan { src: FileSource::Core(CoreRel::SyntaxError), .. }) => Pn::SyntaxError(*negated),
            Expr::Exists { rel, negated: false } => self.search(rel)?,
            Expr::Case { operand: None, whens, else_: Some(el) } if matches!(**el, Expr::Lit(Lit::Int(0))) => match whens.as_slice() {
                // A value test (`front::pattern::verdict`, `Rule::Value`).
                [(Expr::Not(nm), Expr::Lit(Lit::Int(0)), _), (Expr::Exists { rel: sv, negated: false }, Expr::Lit(Lit::Int(1)), _), (Expr::Exists { rel: su, negated: false }, Expr::Lit(Lit::Null), _)] => {
                    let bad = || "physical: value test form".to_string();
                    if !matches!(&**nm, Expr::Cmp(CmpOp::Eq, a, b, _) if is_col(a, NAMED) && matches!(**b, Expr::Lit(Lit::Int(1)))) {
                        return Err(bad());
                    }
                    let key = |e: &E| matches!(e, Expr::Cmp(CmpOp::Eq, a, b, _) if is_col(a, 0) && matches!(**b, Expr::Col { depth: 1, col: ID }));
                    let Rel::Filter { input: u, pred: up } = &**su else { return Err(bad()) };
                    if !matches!(&**u, Rel::Scan { src: FileSource::Core(CoreRel::StringValueUnknown), .. }) || !key(up) {
                        return Err(bad());
                    }
                    let Rel::Filter { input: s, pred: conds } = &**sv else { return Err(bad()) };
                    if !matches!(&**s, Rel::Scan { src: FileSource::Core(CoreRel::StringValue), .. }) {
                        return Err(bad());
                    }
                    let mut flat = Vec::new();
                    let mut stack = vec![conds];
                    while let Some(x) = stack.pop() {
                        match x {
                            Expr::And(a, b) => {
                                stack.push(b);
                                stack.push(a);
                            }
                            _ => flat.push(x),
                        }
                    }
                    if flat.is_empty() || !key(flat[0]) {
                        return Err(bad());
                    }
                    let (mut eq, mut rx) = (None, None);
                    for c in &flat[1..] {
                        match c {
                            Expr::Cmp(CmpOp::Eq, a, b, _) if is_col(a, 1) && lit_text(b).is_some() && eq.is_none() => eq = Some(lit_text(b).unwrap().into()),
                            Expr::Func(Func::RxMatches, a) if a.len() == 2 && is_col(&a[0], 1) && lit_text(&a[1]).is_some() && rx.is_none() => rx = Some(self.regex(lit_text(&a[1]).unwrap())?),
                            _ => return Err(bad()),
                        }
                    }
                    Pn::Value(eq, rx)
                }
                // A relation search over a three-valued rule: TRUE if a candidate holds, NULL if
                // none does and one is NULL.
                [(Expr::Exists { rel: hit, negated: false }, Expr::Lit(Lit::Int(1)), _), (Expr::Exists { rel: open, negated: false }, Expr::Lit(Lit::Null), _)] => {
                    let (Rel::Filter { input: c1, pred: v1 }, Rel::Filter { input: c2, pred: Expr::IsNull { e: v2, negated: false } }) = (&**hit, &**open) else {
                        return Err("physical: three-valued search form".into());
                    };
                    if dbg(c1) != dbg(c2) || dbg(v1) != dbg(&**v2) {
                        return Err("physical: three-valued search form".into());
                    }
                    self.search(hit)?
                }
                _ => return Err("physical: CASE predicate".into()),
            },
            other => return Err(format!("physical: predicate {}", dbg(other).chars().take(120).collect::<String>())),
        };
        Ok(self.intern(p))
    }

    /// `EXISTS(Filter(Filter(file ⋈ node, axis), v))`: a relation search.
    fn search(&mut self, rel: &R) -> Result<Pn, String> {
        let Rel::Filter { input, pred: v } = rel else { return Err("physical: EXISTS over a relation that is not an axis search".into()) };
        let Rel::Filter { input: base, pred: axis } = &**input else { return Err("physical: EXISTS over a relation that is not an axis search".into()) };
        if !is_rows(base) {
            return Err("physical: an axis search over something else than `file ⋈ node`".into());
        }
        Ok(match self.axis(axis)? {
            Some(ax) => Pn::Exists(ax, self.pred(v)?),
            // A field the grammar lacks: no candidate.
            None => Pn::Const(false),
        })
    }

    /// The axis an axis predicate selects (`pattern::axis_pred`), or `None` for a field the
    /// grammar lacks.
    fn axis(&mut self, a: &E) -> Result<Option<Ax>, String> {
        // Stop-rule axes: `base AND NOT EXISTS(file ⋈ node WHERE prefix AND S)`.
        if let Expr::And(base, rest) = a {
            if let Expr::Not(x) = &**rest {
                if let Expr::Exists { rel, negated: false } = &**x {
                    if let Rel::Filter { input, pred: Expr::And(prefix, stop) } = &**rel {
                        let marker = Expr::Lit(Lit::Int(7_777_777));
                        for ax in [Axis::DescendantsUntil, Axis::AncestorsUntil] {
                            let want = fp::axis_pred(&ax, Some(&marker));
                            let Expr::And(wb, wr) = &want else { unreachable!() };
                            let Expr::Not(wx) = &**wr else { unreachable!() };
                            let Expr::Exists { rel: wrel, .. } = &**wx else { unreachable!() };
                            let Rel::Filter { input: winput, pred: Expr::And(wprefix, _) } = &**wrel else { unreachable!() };
                            if dbg(base) == dbg(wb) && dbg(prefix) == dbg(wprefix) && dbg(input) == dbg(winput) {
                                let s = self.pred(stop)?;
                                // Mutant 1 (dropped side condition): the stop rule is dropped.
                                if crate::mutant::on(1) {
                                    return Ok(Some(if ax == Axis::DescendantsUntil { Ax::Descendants } else { Ax::Ancestors }));
                                }
                                return Ok(Some(if ax == Axis::DescendantsUntil { Ax::DescendantsUntil(s) } else { Ax::AncestorsUntil(s) }));
                            }
                        }
                    }
                }
            }
        }
        let s = dbg(a);
        for (ax, phys) in [(Axis::Children, Ax::Children), (Axis::Descendants, Ax::Descendants), (Axis::Parent, Ax::Parent), (Axis::Ancestors, Ax::Ancestors)] {
            if s == dbg(&fp::axis_pred(&ax, None)) {
                return Ok(Some(phys));
            }
        }
        let mut names = Vec::new();
        texts(a, &mut names);
        for f in names {
            let forms: [(Axis, fn(u16) -> Ax); 4] = [
                (Axis::FieldChild(f.clone()), Ax::FieldChild),
                (Axis::FieldSubtree(f.clone()), Ax::FieldSubtree),
                (Axis::InsideField(f.clone()), Ax::InsideField),
                (Axis::InsideFieldEnd(f.clone()), Ax::InsideFieldEnd),
            ];
            for (ax, mk) in forms {
                if s == dbg(&fp::axis_pred(&ax, None)) {
                    return Ok(field_code(&f).map(mk));
                }
            }
        }
        Err(format!("physical: axis predicate {}", s.chars().take(120).collect::<String>()))
    }
}

/// Whether the rewritten plan runs (`FACTS_REWRITES=0`: the unrewritten plan, every matcher
/// input on the leaf executor; the gate's differential reference).
pub fn rewrites() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("FACTS_REWRITES").map_or(true, |v| v.trim() != "0"));
    *ON
}

/// An evidence object's input (`ast` its query, `fields` its projected fields in name order)
/// lowered by the pattern front end and planned; `Err` names why it runs on the leaf executor.
pub fn lower_and_plan(
    ev: &serde_json::Value,
    ast: &serde_json::Value,
    fields: &[String],
    utils: &std::collections::BTreeMap<String, serde_json::Value>,
    grammar: &rerail_engine::code_rule_dsl::tape::Grammar,
    preds: &mut Preds,
) -> Result<Plan, String> {
    use rerail_facts::front::rule as fr;
    use rerail_facts::ir::{InputDef, InputKind};
    if !rewrites() {
        return Err("FACTS_REWRITES=0".into());
    }
    let project = ev.get("project").and_then(serde_json::Value::as_object);
    let mut fs = Vec::with_capacity(fields.len());
    for f in fields {
        let p = project.and_then(|o| o.get(f)).ok_or_else(|| format!("projection {f}"))?;
        fs.push((f.clone(), fr::projection(p)?));
    }
    let def = InputDef {
        name: String::new(),
        kind: InputKind::Pattern { query: ast.clone() },
        schema: fr::input_schema(&fs),
        fields: fs,
        anchor: fr::anchor(ev.get("anchor"))?,
        each: ev.get("each").and_then(serde_json::Value::as_str).map(str::to_owned),
        paths: ev.get("paths").cloned(),
        algebra: Err(String::new()),
    };
    let lowered = fp::lower_input(&def, utils, &|f: &str| Some(grammar.field_code(f).is_some()))?;
    plan(&lowered, preds)
}

/// Plans one lowered input. `Err`: the construct the physical layer lacks (the input runs on
/// the leaf executor).
pub fn plan(l: &Lowered, preds: &mut Preds) -> Result<Plan, String> {
    let mut refuse_syntax = false;
    let mut escape_refusal = false;
    let escape = dbg(&Expr::<FileSource>::Exists {
        rel: Box::new(Rel::Filter {
            input: Box::new(Rel::Scan { src: FileSource::Core(CoreRel::StringValueUnknown), schema: CoreRel::StringValueUnknown.schema() }),
            pred: Expr::And(
                Box::new(Expr::Cmp(CmpOp::Eq, Box::new(Expr::Col { depth: 0, col: 0 }), Box::new(Expr::Col { depth: 1, col: ID }), (rerail_facts::sql::value::Aff::None, rerail_facts::sql::value::Aff::None))),
                Box::new(Expr::Cmp(CmpOp::Eq, Box::new(Expr::Col { depth: 0, col: 1 }), Box::new(Expr::Lit(Lit::Int(1))), (rerail_facts::sql::value::Aff::None, rerail_facts::sql::value::Aff::None))),
            ),
        }),
        negated: false,
    });
    for (r, msg) in &l.refuse_if {
        match r {
            Rel::Scan { src: FileSource::Core(CoreRel::SyntaxError), .. } if msg == fp::NO_SYNTAX => refuse_syntax = true,
            Rel::Filter { input, pred: Expr::And(_, last) } if msg == fp::NAMED_ESCAPE && is_rows(input) && dbg(&**last) == escape => escape_refusal = true,
            _ => return Err(format!("physical: run-time refusal `{msg}`")),
        }
    }
    let Rel::Project { input, exprs, .. } = &l.rows else { return Err("physical: rows are not a projection".into()) };
    // The capture joins, innermost first.
    let mut joins: Vec<&E> = Vec::new();
    let mut cur: &R = input;
    while let Rel::Join { kind: JoinKind::Left, left, right, on: Some(Expr::Cmp(CmpOp::Eq, k, cap, _)) } = cur {
        if !matches!(&**right, Rel::Scan { src: FileSource::Core(CoreRel::Node), .. }) {
            return Err("physical: a join that is not a capture".into());
        }
        joins.push(cap);
        let _ = k;
        cur = left;
    }
    joins.reverse();
    // Each join's key is its own `node.id` column at its base.
    {
        let mut c2: &R = input;
        let mut base = ROW_W + NODE_W * joins.len() as u16;
        while let Rel::Join { left, on: Some(Expr::Cmp(_, k, _, _)), .. } = c2 {
            base -= NODE_W;
            if !is_col(k, base) {
                return Err("physical: a capture join's key".into());
            }
            c2 = left;
        }
    }
    let Rel::Filter { input: base, pred: keep } = cur else { return Err("physical: no selection".into()) };
    if !is_rows(base) {
        return Err("physical: candidates are not `file ⋈ node`".into());
    }
    let keep = preds.pred(keep)?;
    let mut caps = Vec::new();
    for cap in joins {
        caps.push(plan_cap(cap, preds)?);
    }
    if exprs.len() < 3 || !is_col(&exprs[0], PATH) {
        return Err("physical: rows do not start with the path".into());
    }
    let ncaps = caps.len();
    let anchor = (out(&exprs[1], ncaps)?, out(&exprs[2], ncaps)?);
    let outs = exprs[3..].iter().map(|e| out(e, ncaps)).collect::<Result<Vec<_>, _>>()?;
    Ok(Plan { keep, caps, anchor, outs, refuse_syntax, escape_refusal })
}

fn plan_cap(e: &E, preds: &mut Preds) -> Result<Cap, String> {
    let bad = || "physical: capture form".to_string();
    let Expr::Scalar(r) = e else { return Err(bad()) };
    let Rel::Project { input, exprs, .. } = &**r else { return Err(bad()) };
    if exprs.len() != 1 || !is_col(&exprs[0], ID) {
        return Err(bad());
    }
    let (found, sorted) = match &**input {
        Rel::Sort { input, keys, limit: Some(Expr::Lit(Lit::Int(1))), offset: None, .. } if keys.as_slice() == [(ID, true)] => (&**input, true),
        other => (other, false),
    };
    let Rel::Filter { input: cands, pred: v } = found else { return Err(bad()) };
    let Rel::Filter { input: base, pred: axis } = &**cands else { return Err(bad()) };
    if !is_rows(base) {
        return Err(bad());
    }
    let Some(ax) = preds.axis(axis)? else {
        // A field the grammar lacks: never binds.
        let never = preds.intern(Pn::Const(false));
        return Ok(Cap { ax: Ax::Parent, inner: never });
    };
    // The search order is the front end's: nearest first upwards (`ORDER BY id DESC LIMIT 1`),
    // preorder downwards.
    let upward = matches!(ax, Ax::Parent | Ax::Ancestors | Ax::AncestorsUntil(_) | Ax::InsideField(_) | Ax::InsideFieldEnd(_));
    if sorted != upward {
        return Err("physical: capture order".into());
    }
    if !matches!(ax, Ax::Parent | Ax::Children | Ax::FieldChild(_) | Ax::Ancestors | Ax::AncestorsUntil(_) | Ax::Descendants) {
        return Err(format!("physical: capture on axis {ax:?}"));
    }
    Ok(Cap { ax, inner: preds.pred(v)? })
}

/// An output expression: a root or capture column (`node` columns from 1 for the root, from
/// `12 + 11 k` for capture k+1), the path, a text, a 0-based line, a literal.
fn out(e: &E, ncaps: usize) -> Result<Out, String> {
    let at = |c: u16| -> Result<(u8, u16), String> {
        if (1..ROW_W).contains(&c) {
            return Ok((0, c - 1));
        }
        let k = (c - ROW_W) / NODE_W;
        if (k as usize) < ncaps {
            Ok((k as u8 + 1, (c - ROW_W) % NODE_W))
        } else {
            Err("physical: output column out of range".into())
        }
    };
    let ncol = |k: u16| -> Result<NodeCol, String> {
        Ok(match k {
            0 => NodeCol::Id,
            1 => NodeCol::Kind,
            3 => NodeCol::Start,
            4 => NodeCol::End,
            _ => return Err(format!("physical: output node column {k}")),
        })
    };
    Ok(match e {
        _ if is_col(e, PATH) => Out::Path,
        Expr::Col { depth: 0, col } => {
            let (w, k) = at(*col)?;
            Out::Node(w, ncol(k)?)
        }
        Expr::Lit(Lit::Int(i)) => Out::Int(*i),
        Expr::Func(Func::SourceText, a) if a.len() == 3 && is_col(&a[0], PATH) => match (&a[1], &a[2]) {
            (Expr::Col { depth: 0, col: s }, Expr::Col { depth: 0, col: t }) => {
                let ((w1, k1), (w2, k2)) = (at(*s)?, at(*t)?);
                if w1 != w2 || k1 != 3 || k2 != 4 {
                    return Err("physical: a text over mixed spans".into());
                }
                Out::Text(w1)
            }
            _ => return Err("physical: text form".into()),
        },
        Expr::Arith(ArithOp::Sub, a, b) if matches!(**b, Expr::Lit(Lit::Int(1))) => match &**a {
            Expr::Col { depth: 0, col } if at(*col)?.1 == 5 => Out::Line(at(*col)?.0),
            _ => return Err("physical: arithmetic form".into()),
        },
        Expr::Func(Func::Coalesce, a) if a.len() == 2 && lit_text(&a[1]) == Some("") && is_decoded(&a[0]) => Out::Decoded,
        Expr::Func(Func::Coalesce, a) if a.len() == 2 => Out::Coalesce(Box::new(out(&a[0], ncaps)?), Box::new(out(&a[1], ncaps)?)),
        Expr::Case { operand: None, whens, else_: Some(el) } if whens.len() == 1 => {
            let (Expr::IsNull { e: s, negated: false }, Expr::Lit(Lit::Null), _) = &whens[0] else { return Err("physical: CASE form".into()) };
            let Expr::Func(Func::RxExtract, a) = &**el else { return Err("physical: CASE form".into()) };
            if a.len() != 3 || dbg(&a[0]) != dbg(&**s) {
                return Err("physical: CASE form".into());
            }
            let (Some(p), Expr::Lit(Lit::Int(g))) = (lit_text(&a[1]), &a[2]) else { return Err("physical: regex projection form".into()) };
            let src = out(s, ncaps)?;
            if !matches!(src, Out::Text(_)) {
                return Err("physical: a regex projection over something else than a capture's text".into());
            }
            Out::RxExtract(Box::new(src), crate::rx::get(p).map_err(|e| e.to_string())?, *g as usize)
        }
        other => return Err(format!("physical: output {}", dbg(other).chars().take(120).collect::<String>())),
    })
}

// ---------------------------------------------------------------------------------------------
// Bitsets

#[derive(Clone)]
struct Bits(Vec<u64>);

impl Bits {
    fn zero(n: usize) -> Bits {
        Bits(vec![0; n.div_ceil(64)])
    }
    #[inline]
    fn get(&self, i: u32) -> bool {
        self.0[(i >> 6) as usize] >> (i & 63) & 1 != 0
    }
    #[inline]
    fn set(&mut self, i: u32) {
        self.0[(i >> 6) as usize] |= 1 << (i & 63);
    }
    fn set_range(&mut self, a: u32, b: u32) {
        for i in a..b {
            self.set(i);
        }
    }
    fn is_empty(&self) -> bool {
        self.0.iter().all(|w| *w == 0)
    }
    fn and(&self, o: &Bits) -> Bits {
        Bits(self.0.iter().zip(&o.0).map(|(a, b)| a & b).collect())
    }
    fn and_not(&self, o: &Bits) -> Bits {
        Bits(self.0.iter().zip(&o.0).map(|(a, b)| a & !b).collect())
    }
    fn or_assign(&mut self, o: &Bits) {
        for (a, b) in self.0.iter_mut().zip(&o.0) {
            *a |= b;
        }
    }
    fn and_not_assign(&mut self, o: &Bits) {
        for (a, b) in self.0.iter_mut().zip(&o.0) {
            *a &= !b;
        }
    }
    /// Set positions in increasing order.
    fn ones(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().enumerate().flat_map(|(wi, &w)| {
            let mut w = w;
            std::iter::from_fn(move || {
                if w == 0 {
                    return None;
                }
                let t = w.trailing_zeros();
                w &= w - 1;
                Some((wi as u32) << 6 | t)
            })
        })
    }
    /// The first set position in `[a, b)`.
    fn first_in(&self, a: u32, b: u32) -> Option<u32> {
        if a >= b {
            return None;
        }
        let (mut wi, last) = ((a >> 6) as usize, ((b - 1) >> 6) as usize);
        let mut w = self.0[wi] & (!0u64 << (a & 63));
        loop {
            if w != 0 {
                let i = (wi as u32) << 6 | w.trailing_zeros();
                return (i < b).then_some(i);
            }
            wi += 1;
            if wi > last {
                return None;
            }
            w = self.0[wi];
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Evaluation over one file

/// A predicate's verdicts over some nodes: where it is TRUE, and where it is NULL (`None`: nowhere).
struct Tv {
    t: Bits,
    u: Option<Bits>,
}

impl Tv {
    fn two(t: Bits) -> Tv {
        Tv { t, u: None }
    }
}

/// A literal's decoded value, as the value test reads it.
#[derive(Clone)]
enum Lv {
    Str(String),
    Unknown,
    Other,
}

/// One file's memoised predicate verdicts for one rule: per predicate, the nodes it was
/// evaluated on, where it is TRUE and where NULL.
pub struct FileState<'f> {
    f: &'f Facts,
    path: &'f str,
    memo: Vec<Option<Box<(Bits, Tv)>>>,
    named: Option<Bits>,
    /// Per node: it is the first child of its parent under its own field.
    first_field: Option<Vec<bool>>,
    values: FxHashMap<u32, Lv>,
    /// The source holds no `\N{` (no literal can decode a named escape).
    escape_free: Option<bool>,
}

impl<'f> FileState<'f> {
    pub fn new(f: &'f Facts, path: &'f str) -> Self {
        FileState { f, path, memo: Vec::new(), named: None, first_field: None, values: FxHashMap::default(), escape_free: None }
    }

    fn n(&self) -> usize {
        self.f.len()
    }

    fn named(&mut self) -> &Bits {
        if self.named.is_none() {
            let mut b = Bits::zero(self.n());
            for i in 0..self.n() as u32 {
                if self.f.named(i) {
                    b.set(i);
                }
            }
            self.named = Some(b);
        }
        self.named.as_ref().unwrap()
    }

    fn first_field(&mut self, v: u32) -> bool {
        if self.first_field.is_none() {
            let f = self.f;
            let mut out = vec![false; f.len()];
            let mut seen: Vec<u16> = Vec::new();
            for p in 0..f.len() as u32 {
                seen.clear();
                for &c in f.children(p) {
                    let code = f.field[c as usize];
                    if code != 0 && !seen.contains(&code) {
                        seen.push(code);
                        out[c as usize] = true;
                    }
                }
            }
            self.first_field = Some(out);
        }
        self.first_field.as_ref().unwrap()[v as usize]
    }

    /// The first child of `n` under field `code`.
    fn field_child(&self, n: u32, code: u16) -> Option<u32> {
        self.f.children(n).iter().copied().find(|&c| self.f.field[c as usize] == code)
    }

    fn parent(&self, n: u32) -> u32 {
        self.f.parent[n as usize]
    }

    /// A string literal's value (`views::string_values`: `str`, unknown, or neither).
    fn value(&mut self, x: u32) -> Lv {
        if let Some(v) = self.values.get(&x) {
            return v.clone();
        }
        let saved = crate::pyvalue::NAMED_ESCAPES.with(|n| n.replace(0));
        let v = match crate::pyvalue::decode(self.f, x) {
            Ok(crate::pyvalue::PyValue::Str(s)) => Lv::Str(s),
            Ok(crate::pyvalue::PyValue::Unknown) => Lv::Unknown,
            _ => Lv::Other,
        };
        crate::pyvalue::NAMED_ESCAPES.with(|n| n.set(saved));
        self.values.insert(x, v.clone());
        v
    }

    /// Where `id` is TRUE and NULL among `mask`.
    fn eval(&mut self, p: &Preds, id: Id, mask: &Bits) -> Tv {
        match &p.nodes[id as usize] {
            Pn::Const(_) | Pn::Kind(_) | Pn::KindIn(_) | Pn::Named | Pn::PathRx(_) | Pn::SyntaxError(_) => return self.raw(p, id, mask),
            _ => {}
        }
        if self.memo.len() <= id as usize {
            self.memo.resize_with(id as usize + 1, || None);
        }
        let todo = match &self.memo[id as usize] {
            Some(m) => {
                let todo = mask.and_not(&m.0);
                if todo.is_empty() {
                    return Tv { t: m.1.t.and(mask), u: m.1.u.as_ref().map(|u| u.and(mask)) };
                }
                todo
            }
            None => mask.clone(),
        };
        let r = self.raw(p, id, &todo);
        let n = self.f.len();
        let m = self.memo[id as usize].get_or_insert_with(|| Box::new((Bits::zero(n), Tv::two(Bits::zero(n)))));
        m.0.or_assign(&todo);
        m.1.t.or_assign(&r.t);
        if let Some(u) = &r.u {
            m.1.u.get_or_insert_with(|| Bits::zero(n)).or_assign(u);
        }
        Tv { t: m.1.t.and(mask), u: m.1.u.as_ref().map(|u| u.and(mask)) }
    }

    fn raw(&mut self, p: &Preds, id: Id, mask: &Bits) -> Tv {
        let n = self.n();
        let f = self.f;
        let two = Tv::two;
        match &p.nodes[id as usize] {
            Pn::Const(true) => two(mask.clone()),
            Pn::Const(false) => two(Bits::zero(n)),
            Pn::Kind(k) => {
                let mut out = Bits::zero(n);
                for &x in f.of_kind(*k) {
                    if mask.get(x) {
                        out.set(x);
                    }
                }
                two(out)
            }
            Pn::KindIn(ks) => {
                let mut out = Bits::zero(n);
                for k in ks {
                    for &x in f.of_kind(*k) {
                        if mask.get(x) {
                            out.set(x);
                        }
                    }
                }
                two(out)
            }
            Pn::Named => two(mask.and(self.named())),
            Pn::TextEq(t) => {
                let mut out = Bits::zero(n);
                for x in mask.ones() {
                    if f.text(x) == &**t {
                        out.set(x);
                    }
                }
                two(out)
            }
            Pn::TextRx(r) => {
                let rx = &p.regexes[*r as usize];
                let mut out = Bits::zero(n);
                for x in mask.ones() {
                    if rx.is_match(f.text(x)) {
                        out.set(x);
                    }
                }
                two(out)
            }
            Pn::PathRx(r) => two(if p.regexes[*r as usize].is_match(self.path) { mask.clone() } else { Bits::zero(n) }),
            Pn::SyntaxError(negated) => two(if f.syntax_errors != *negated { mask.clone() } else { Bits::zero(n) }),
            Pn::Value(eq, rx) => {
                let strings = [facts::intern("string"), facts::intern("concatenated_string")];
                let (mut t, mut u) = (Bits::zero(n), Bits::zero(n));
                for x in mask.ones() {
                    if !f.named(x) || !strings.contains(&f.kind[x as usize]) {
                        continue;
                    }
                    match self.value(x) {
                        Lv::Str(s) => {
                            if eq.as_deref().is_none_or(|w| s == w) && rx.is_none_or(|r| p.regexes[r as usize].is_match(&s)) {
                                t.set(x);
                            }
                        }
                        Lv::Unknown => u.set(x),
                        Lv::Other => {}
                    }
                }
                Tv { t, u: Some(u) }
            }
            Pn::And(items) => {
                // A conjunct runs where every earlier one is not FALSE.
                let mut m = mask.clone();
                let mut nulls: Option<Bits> = None;
                for &c in items {
                    if m.is_empty() {
                        break;
                    }
                    let r = self.eval(p, c, &m);
                    m = match &r.u {
                        Some(u) => {
                            nulls.get_or_insert_with(|| Bits::zero(n)).or_assign(u);
                            let mut nf = r.t.clone();
                            nf.or_assign(u);
                            nf
                        }
                        None => r.t,
                    };
                }
                match nulls {
                    Some(u) => Tv { t: m.and_not(&u), u: Some(m.and(&u)) },
                    None => two(m),
                }
            }
            Pn::Or(items) => {
                // Mutant 2 (lost union arm): the last disjunct is dropped.
                let items = &items[..items.len() - (crate::mutant::on(2) && items.len() > 1) as usize];
                // A disjunct runs where every earlier one is not TRUE.
                let mut acc = Bits::zero(n);
                let mut nulls: Option<Bits> = None;
                let mut rest = mask.clone();
                for &c in items {
                    if rest.is_empty() {
                        break;
                    }
                    let r = self.eval(p, c, &rest);
                    rest.and_not_assign(&r.t);
                    acc.or_assign(&r.t);
                    if let Some(u) = &r.u {
                        nulls.get_or_insert_with(|| Bits::zero(n)).or_assign(u);
                    }
                }
                Tv { u: nulls.map(|u| u.and_not(&acc)), t: acc }
            }
            Pn::Not(c) => {
                let r = self.eval(p, *c, mask);
                let mut decided = r.t.clone();
                if let Some(u) = &r.u {
                    decided.or_assign(u);
                }
                Tv { t: mask.and_not(&decided), u: r.u }
            }
            Pn::Exists(ax, inner) => {
                let (ax, inner) = (*ax, *inner);
                let cm = self.candidates(ax, mask);
                let r = self.eval(p, inner, &cm);
                // A stop rule is two-valued (`front::pattern` refuses a `value` test in one).
                let stop = match ax {
                    Ax::DescendantsUntil(s) | Ax::AncestorsUntil(s) => Some(self.eval(p, s, &cm).t),
                    _ => None,
                };
                let t = self.found(ax, mask, &cm, &r.t, stop.as_ref());
                let u = r.u.as_ref().map(|u| self.found(ax, mask, &cm, u, stop.as_ref()).and_not(&t));
                Tv { t, u }
            }
        }
    }

    /// The proper ancestors of the nodes of `mask`.
    fn ancestors_of(&self, mask: &Bits) -> Bits {
        let mut am = Bits::zero(self.n());
        for x in mask.ones() {
            let mut c = self.parent(x);
            while c != NONE && !am.get(c) {
                am.set(c);
                c = self.parent(c);
            }
        }
        am
    }

    /// The proper descendants of the nodes of `mask` (nested intervals, preorder).
    fn descendants_of(&self, mask: &Bits) -> Bits {
        let mut dm = Bits::zero(self.n());
        let mut covered = 0u32;
        for x in mask.ones() {
            let end = self.f.subtree_end[x as usize];
            let a = (x + 1).max(covered);
            if a < end {
                dm.set_range(a, end);
                covered = end;
            }
        }
        dm
    }

    /// Every candidate an axis search from a node of `mask` may test.
    fn candidates(&mut self, ax: Ax, mask: &Bits) -> Bits {
        let n = self.n();
        let f = self.f;
        let mut cm = Bits::zero(n);
        match ax {
            Ax::Children => {
                for x in mask.ones() {
                    for &c in f.children(x) {
                        cm.set(c);
                    }
                }
            }
            Ax::Parent => {
                for x in mask.ones() {
                    if self.parent(x) != NONE {
                        cm.set(self.parent(x));
                    }
                }
            }
            Ax::Descendants | Ax::DescendantsUntil(_) => cm = self.descendants_of(mask),
            Ax::Ancestors | Ax::AncestorsUntil(_) | Ax::InsideFieldEnd(_) => cm = self.ancestors_of(mask),
            Ax::FieldChild(code) => {
                for x in mask.ones() {
                    if let Some(c) = self.field_child(x, code) {
                        cm.set(c);
                    }
                }
            }
            Ax::FieldSubtree(code) => {
                for x in mask.ones() {
                    if let Some(c) = self.field_child(x, code) {
                        cm.set_range(c, f.subtree_end[c as usize]);
                    }
                }
            }
            Ax::InsideField(code) => {
                for x in mask.ones() {
                    let q = self.parent(x);
                    if q != NONE && f.field[x as usize] == code && self.first_field(x) {
                        cm.set(q);
                    }
                }
            }
        }
        cm
    }

    /// The nodes of `mask` whose search on `ax` finds a candidate in `hit` (`stop`: the stop
    /// rule's verdicts over the candidates, for `*Until`).
    fn found(&mut self, ax: Ax, mask: &Bits, cm: &Bits, hit: &Bits, stop: Option<&Bits>) -> Bits {
        let n = self.n();
        let f = self.f;
        let mut out = Bits::zero(n);
        match ax {
            Ax::Children => {
                for x in mask.ones() {
                    if f.children(x).iter().any(|&c| hit.get(c)) {
                        out.set(x);
                    }
                }
            }
            Ax::Parent => {
                for x in mask.ones() {
                    let q = self.parent(x);
                    if q != NONE && hit.get(q) {
                        out.set(x);
                    }
                }
            }
            Ax::Descendants => {
                for x in mask.ones() {
                    // Mutant 3 (wrong interval bound): the first descendant is skipped.
                    if hit.first_in(x + 1 + crate::mutant::on(3) as u32, f.subtree_end[x as usize]).is_some() {
                        out.set(x);
                    }
                }
            }
            Ax::DescendantsUntil(_) => {
                // Bottom-up: a node finds one when a child holds, or does not stop and finds one.
                let s = stop.expect("stop verdicts");
                let mut h = Bits::zero(n);
                let order: Vec<u32> = cm.ones().collect();
                for &c in order.iter().rev() {
                    let q = self.parent(c);
                    if q != NONE && (hit.get(c) || (!s.get(c) && h.get(c))) {
                        h.set(q);
                    }
                }
                out = h.and(mask);
            }
            Ax::Ancestors | Ax::AncestorsUntil(_) | Ax::InsideFieldEnd(_) => {
                // Top-down in preorder (a parent precedes its children), over the anchors and
                // their ancestors only (every parent met is a candidate).
                let mut h = Bits::zero(n);
                let mut walk = cm.clone();
                walk.or_assign(mask);
                for v in walk.ones() {
                    let q = self.parent(v);
                    if q == NONE {
                        continue;
                    }
                    let holds = match ax {
                        Ax::Ancestors => hit.get(q) || h.get(q),
                        Ax::AncestorsUntil(_) => hit.get(q) || (!stop.expect("stop verdicts").get(q) && h.get(q)),
                        Ax::InsideFieldEnd(code) => (f.field[v as usize] == code && self.first_field(v) && hit.get(q)) || h.get(q),
                        _ => unreachable!(),
                    };
                    if holds {
                        h.set(v);
                    }
                }
                out = h.and(mask);
            }
            Ax::FieldChild(code) => {
                for x in mask.ones() {
                    if self.field_child(x, code).is_some_and(|c| hit.get(c)) {
                        out.set(x);
                    }
                }
            }
            Ax::FieldSubtree(code) => {
                for x in mask.ones() {
                    if let Some(c) = self.field_child(x, code) {
                        if hit.first_in(c, f.subtree_end[c as usize]).is_some() {
                            out.set(x);
                        }
                    }
                }
            }
            Ax::InsideField(code) => {
                for x in mask.ones() {
                    let q = self.parent(x);
                    if q != NONE && f.field[x as usize] == code && self.first_field(x) && hit.get(q) {
                        out.set(x);
                    }
                }
            }
        }
        out
    }

    /// Per root (in order), the capture's node or `NONE`. Capture rules are two-valued.
    fn capture(&mut self, p: &Preds, cap: &Cap, roots: &[u32]) -> Vec<u32> {
        let n = self.n();
        let f = self.f;
        let mut roots_mask = Bits::zero(n);
        for &r in roots {
            roots_mask.set(r);
        }
        let cm = self.candidates(cap.ax, &roots_mask);
        let t = self.eval(p, cap.inner, &cm).t;
        match cap.ax {
            Ax::Parent => roots.iter().map(|&r| self.parent(r)).map(|q| if q != NONE && t.get(q) { q } else { NONE }).collect(),
            Ax::Children => roots.iter().map(|&r| f.children(r).iter().copied().find(|&c| t.get(c)).unwrap_or(NONE)).collect(),
            Ax::FieldChild(code) => roots.iter().map(|&r| self.field_child(r, code).filter(|&c| t.get(c)).unwrap_or(NONE)).collect(),
            Ax::Descendants => roots.iter().map(|&r| t.first_in(r + 1, f.subtree_end[r as usize]).unwrap_or(NONE)).collect(),
            Ax::Ancestors | Ax::AncestorsUntil(_) => {
                let sb = if let Ax::AncestorsUntil(s) = cap.ax { Some(self.eval(p, s, &cm).t) } else { None };
                // Per node in preorder: the nearest reachable ancestor where the rule holds, over
                // the roots and their ancestors only.
                let mut near = vec![NONE; n];
                let mut walk = cm.clone();
                walk.or_assign(&roots_mask);
                for v in walk.ones() {
                    let q = self.parent(v);
                    if q == NONE {
                        continue;
                    }
                    near[v as usize] = if t.get(q) {
                        q
                    } else if sb.as_ref().is_some_and(|s| s.get(q)) {
                        NONE
                    } else {
                        near[q as usize]
                    };
                }
                roots.iter().map(|&r| near[r as usize]).collect()
            }
            other => unreachable!("capture axis {other:?} refused by `plan_cap`"),
        }
    }
}

/// One input's rows over one file, as `Input::rows` produces them; `None` where the plan's
/// side condition fails (a `\N{` in a file whose input has a `value` test refusal: the caller
/// runs the leaf executor).
pub fn rows(plan: &Plan, preds: &Preds, st: &mut FileState<'_>, schema: &Schema, source: &Rc<str>) -> Option<Rows> {
    let f = st.f;
    if plan.escape_refusal && !*st.escape_free.get_or_insert_with(|| !f.source.contains("\\N{")) {
        return None;
    }
    let mut out = Rows { table: Rc::new(Table::empty(schema.clone())), anchors: Vec::new(), witnesses: Vec::new(), unsupported: None };
    if plan.refuse_syntax && f.syntax_errors {
        out.unsupported = Some(fp::NO_SYNTAX.into());
        return Some(out);
    }
    let mut all = Bits::zero(f.len());
    all.set_range(0, f.len() as u32);
    let roots: Vec<u32> = st.eval(preds, plan.keep, &all).t.ones().collect();
    let caps: Vec<Vec<u32>> = plan.caps.iter().map(|c| st.capture(preds, c, &roots)).collect();
    // Decoded values of the roots, where an output reads them (`string_value`: string kinds).
    let decoded: Vec<V> = if plan.outs.iter().any(|o| matches!(o, Out::Decoded)) {
        let strings = [facts::intern("string"), facts::intern("concatenated_string")];
        roots
            .iter()
            .map(|&r| match strings.contains(&f.kind[r as usize]).then(|| st.value(r)) {
                Some(Lv::Str(s)) => V::text(&s),
                _ => V::text(""),
            })
            .collect()
    } else {
        Vec::new()
    };
    // The node a column reads, per row.
    let node = |w: u8, i: usize| -> u32 { if w == 0 { roots[i] } else { caps[w as usize - 1][i] } };
    let line = |b: u32| f.lines.line(b as usize) as u32;
    let u32_of = |o: &Out, i: usize| -> u32 {
        fn go(o: &Out, i: usize, node: &dyn Fn(u8, usize) -> u32, f: &Facts, line: &dyn Fn(u32) -> u32) -> u32 {
            match o {
                Out::Int(v) => *v as u32,
                Out::Node(w, c) => {
                    let x = node(*w, i);
                    if x == NONE {
                        return U32_NULL;
                    }
                    match c {
                        NodeCol::Id => x,
                        NodeCol::Start => f.start[x as usize],
                        NodeCol::End => f.end[x as usize],
                        NodeCol::Kind => unreachable!("kind is a symbol column"),
                    }
                }
                Out::Line(w) => {
                    let x = node(*w, i);
                    if x == NONE { U32_NULL } else { line(f.start[x as usize]) }
                }
                Out::Coalesce(a, b) => {
                    let v = go(a, i, node, f, line);
                    if v == U32_NULL { go(b, i, node, f, line) } else { v }
                }
                _ => unreachable!("not an integer output"),
            }
        }
        go(o, i, &node, f, &line)
    };
    let k = roots.len();
    let path: Rc<str> = Rc::from(st.path);
    let mut cols: Vec<Rc<Column>> = Vec::with_capacity(3 + plan.outs.len());
    cols.push(Rc::new(Column::Const(V::Text(path.clone()))));
    let starts: Vec<u32> = (0..k).map(|i| u32_of(&plan.anchor.0, i)).collect();
    let ends: Vec<u32> = (0..k).map(|i| u32_of(&plan.anchor.1, i)).collect();
    for o in &plan.outs {
        cols.push(Rc::new(match o {
            Out::Path => Column::Const(V::Text(path.clone())),
            Out::Int(v) => Column::Const(V::Int(*v)),
            Out::Node(w, NodeCol::Kind) => Column::Sym((0..k).map(|i| node(*w, i)).map(|x| if x == NONE { NONE } else { f.kind[x as usize] }).collect()),
            Out::Node(..) | Out::Line(_) | Out::Coalesce(..) => Column::U32((0..k).map(|i| u32_of(o, i)).collect()),
            Out::Text(w) => Column::text(
                source.clone(),
                (0..k).map(|i| node(*w, i)).map(|x| if x == NONE { TextCell::Val(V::Null) } else { TextCell::Span(f.start[x as usize], f.end[x as usize]) }).collect(),
            ),
            Out::RxExtract(src, rx, g) => {
                let Out::Text(w) = &**src else { unreachable!("regex projections read a text") };
                Column::Vals(
                    (0..k)
                        .map(|i| node(*w, i))
                        .map(|x| if x == NONE { V::Null } else { rx.captures(f.text(x)).and_then(|c| c.get(*g)).map_or(V::Null, |m| V::text(m.as_str())) })
                        .collect(),
                )
            }
            Out::Decoded => Column::Vals(decoded.clone()),
        }));
    }
    for i in 0..k {
        let r = roots[i];
        out.anchors.push((starts[i], ends[i], line(starts[i])));
        out.witnesses.push((f.start[r as usize], f.end[r as usize]));
    }
    cols.insert(1, Rc::new(Column::U32(starts)));
    cols.insert(2, Rc::new(Column::U32(ends)));
    out.table = Rc::new(Table::new(schema.clone(), cols, k));
    Some(out)
}

/// R12/R13 on synthetic files: the physical plan's rows equal the leaf executor's (the truth) for
/// inputs that exercise each seeded mutant's site, where the corpus barely reaches it (mutant 1
/// survives all 9,010 files, mutant 3 shows on one row). Each test also asserts the input plans,
/// so a refusal cannot pass it vacuously.
#[cfg(test)]
mod tests {
    use crate::ast::{Compiler, FileMemo, Names};
    use crate::eval::{Anchor, FileCtx, Input, Rows};
    use crate::facts;
    use std::collections::BTreeMap;
    use std::rc::Rc;
    use std::sync::Arc;

    /// The roots `rule` matches in `src`, as rows `[path, start, end]`: (leaf executor, physical).
    fn both(src: &str, rule: serde_json::Value) -> (Vec<String>, Vec<String>) {
        let ast = serde_json::json!({ "language": "python", "syntax": "valid", "rule": rule });
        let ev = serde_json::json!({ "as": "row", "query": { "ast": ast } });
        let utils = BTreeMap::new();
        let mut names = Names::default();
        let query = Compiler::new(&mut names, &facts::GRAMMAR, &utils).query(&ast).unwrap();
        let mut preds = super::Preds::default();
        let plan = super::lower_and_plan(&ev, &ast, &[], &utils, &facts::GRAMMAR, &mut preds).expect("the input plans");
        let input = Input { query, fields: vec![], projs: vec![], each: None, anchor: Anchor::Span, paths: None, node_regexes: vec![], groups: vec![], phys: Err("leaf".into()) };
        let source = Arc::new(src.to_owned());
        let Ok(facts::Built::Facts(f)) = facts::build("t.py", source.clone()) else { panic!("facts") };
        let ctx = FileCtx { f: &f, path: "t.py", source: &source };
        let rc: Rc<str> = Rc::from(src);
        let schema = crate::rel::input_schema(&[]);
        let show = |r: &Rows| (0..r.table.len()).map(|i| format!("{:?}", r.table.row(i))).collect::<Vec<_>>();
        let leaf = input.rows(&ctx, &schema, &FileMemo::default(), &rc);
        let mut st = super::FileState::new(&f, "t.py");
        let phys = super::rows(&plan, &preds, &mut st, &schema, &rc).expect("the physical plan runs");
        assert!(leaf.unsupported.is_none() && phys.unsupported.is_none());
        (show(&leaf), show(&phys))
    }

    /// `inside` with a stop rule: the walk up stops at a function, so the method's names are not
    /// inside the class. Kills mutant 1 (stop rule dropped).
    #[test]
    fn stop_rule_bounds_the_walk() {
        let src = "class A:\n    x = 1\n    def f(self):\n        y = 2\n";
        let rule = serde_json::json!({ "all": [{ "kind": "identifier" }, { "inside": { "kind": "class_definition", "stopBy": { "kind": "function_definition" } } }] });
        let (leaf, phys) = both(src, rule);
        assert_eq!(leaf.len(), 2, "{leaf:?}");
        assert_eq!(leaf, phys);
    }

    /// An `any` whose last disjunct alone selects some roots. Kills mutant 2 (lost union arm).
    #[test]
    fn every_disjunct_selects() {
        let src = "a = 1\nb = 'x'\n";
        let rule = serde_json::json!({ "any": [{ "kind": "integer" }, { "all": [{ "kind": "identifier" }, { "regex": "^b$" }] }] });
        let (leaf, phys) = both(src, rule);
        assert_eq!(leaf.len(), 2, "{leaf:?}");
        assert_eq!(leaf, phys);
    }

    /// `has … stopBy: end` whose only witness is the root's first descendant (`f` of `f()`).
    /// Kills mutant 3 (descendant interval from `id + 2`).
    #[test]
    fn first_descendant_is_a_descendant() {
        let src = "f()\n";
        let rule = serde_json::json!({ "all": [{ "kind": "call" }, { "has": { "kind": "identifier", "stopBy": "end" } }] });
        let (leaf, phys) = both(src, rule);
        assert_eq!(leaf.len(), 1, "{leaf:?}");
        assert_eq!(leaf, phys);
    }
}
