//! Physical `EXPLAIN` per file (INVARIANTS I5): the plans this executor runs for one content,
//! printed with `rerail_facts::phys` (stable text) and checked by its inner-scan audit.
//!
//! The statements of a relational clause are planned once per rule (`rel::RelPlan::compile`),
//! so for every file the same clause prints the same text; what varies per file is which clauses
//! apply (the policy scope). S5's `explain-stable` compares each clause's text across the files
//! of the subset.
//!
//! `facts-a explain --physical [--rules DIR] (--file PATH … | --root DIR [--limit N]) [--audit]`
//! prints, per file, `== file <path>` then for each clause in scope `clause <check>/<clause>`
//! and each stage's plan. `--audit` adds the audit's findings under each stage (`! …`).
use crate::rel::{RelPlan, StagePlan};
use crate::sql::engine::{closure_applies, Cmp, Lit, PBody, PCte, PE, PItem, PQuery, PSelect, Src, Statement};
use crate::sql::parse::SetOp;
use crate::{arg, eval};
use rerail_facts::phys::{self, Bound, Card, Item, Meta, Mono, Node, Op, PipelineItem, PlanCost};
use std::fmt::Write;

/// An expression's text; text literals are resolved through their statement's `texts` and print
/// as `Text("…")`.
fn pe(st: &Statement, e: &PE) -> String {
    let b = |x: &PE| pe(st, x);
    match e {
        PE::Lit(Lit::Text(i)) => format!("Text({:?})", &*st.texts[*i as usize]),
        PE::Lit(l) => format!("{l:?}"),
        PE::Col { depth, item, col } => format!("{}i{item}.{col}", "^".repeat(*depth as usize)),
        PE::Agg(i) => format!("agg{i}"),
        PE::Win(i) => format!("win{i}"),
        PE::Not(x) => format!("NOT {}", b(x)),
        PE::Neg(x) => format!("-{}", b(x)),
        PE::Cmp(op, l, r, a) => format!("({} {op:?}[{:?},{:?}] {})", b(l), a.0, a.1, b(r)),
        PE::Arith(op, l, r) => format!("({} {op} {})", b(l), b(r)),
        PE::Concat(l, r) => format!("({} || {})", b(l), b(r)),
        PE::And(l, r) => format!("({} AND {})", b(l), b(r)),
        PE::Or(l, r) => format!("({} OR {})", b(l), b(r)),
        PE::Func(f, a) => format!("{f:?}({})", a.iter().map(b).collect::<Vec<_>>().join(", ")),
        PE::Case { operand, whens, else_, .. } => format!(
            "CASE{} {}{} END",
            operand.as_ref().map(|o| format!(" {}", b(o))).unwrap_or_default(),
            whens.iter().map(|(w, t)| format!("WHEN {} THEN {}", b(w), b(t))).collect::<Vec<_>>().join(" "),
            else_.as_ref().map(|x| format!(" ELSE {}", b(x))).unwrap_or_default()
        ),
        PE::Cast(x, a) => format!("CAST({} AS {a:?})", b(x)),
        PE::InList { e, list, not } => format!("{} {}IN ({})", b(e), if *not { "NOT " } else { "" }, list.iter().map(|(x, _)| b(x)).collect::<Vec<_>>().join(", ")),
        PE::InSub { e, sub, not, .. } => format!("{} {}IN $sub{sub}", b(e), if *not { "NOT " } else { "" }),
        PE::Exists { sub, not } => format!("{}EXISTS $sub{sub}", if *not { "NOT " } else { "" }),
        PE::Scalar(sub) => format!("$sub{sub}"),
        PE::IsNull(x, not) => format!("{} IS {}NULL", b(x), if *not { "NOT " } else { "" }),
        PE::Is(l, r, not) => format!("{} IS {}{}", b(l), if *not { "NOT " } else { "" }, b(r)),
        PE::Like(l, r, not) => format!("{} {}LIKE {}", b(l), if *not { "NOT " } else { "" }, b(r)),
    }
}

/// The subqueries an expression references, in order of first reference.
fn subs_of(e: &PE, out: &mut Vec<(usize, u8)>) {
    let mut push = |s: usize, k: u8| {
        if !out.iter().any(|(x, _)| *x == s) {
            out.push((s, k));
        }
    };
    match e {
        PE::InSub { e, sub, not, .. } => {
            subs_of(e, out);
            if !out.iter().any(|(x, _)| x == sub) {
                out.push((*sub, if *not { 1 } else { 0 }));
            }
        }
        PE::Exists { sub, not } => push(*sub, if *not { 1 } else { 0 }),
        PE::Scalar(sub) => push(*sub, 2),
        PE::Not(x) | PE::Neg(x) | PE::Cast(x, _) | PE::IsNull(x, _) => subs_of(x, out),
        PE::Cmp(_, l, r, _) | PE::Arith(_, l, r) | PE::Concat(l, r) | PE::And(l, r) | PE::Or(l, r) | PE::Is(l, r, _) | PE::Like(l, r, _) => {
            subs_of(l, out);
            subs_of(r, out)
        }
        PE::Func(_, a) => a.iter().for_each(|x| subs_of(x, out)),
        PE::Case { operand, whens, else_, .. } => {
            if let Some(o) = operand {
                subs_of(o, out)
            }
            for (w, t) in whens {
                subs_of(w, out);
                subs_of(t, out)
            }
            if let Some(x) = else_ {
                subs_of(x, out)
            }
        }
        PE::InList { e, list, .. } => {
            subs_of(e, out);
            list.iter().for_each(|(x, _)| subs_of(x, out))
        }
        PE::Lit(_) | PE::Col { .. } | PE::Agg(_) | PE::Win(_) => {}
    }
}

fn node(op: Op, children: Vec<Node>) -> Node {
    // The digest: the operator's text over its children's digests (stable, no addresses).
    let mut h = blake3::Hasher::new();
    h.update(phys::explain(&Node { op: op.clone(), digest: String::new(), children: vec![], meta: Meta::default() }).as_bytes());
    for c in &children {
        h.update(c.digest.as_bytes());
    }
    Node { op, digest: h.finalize().to_hex()[..12].to_string(), children, meta: Meta::default() }
}

fn with_meta(mut n: Node, meta: Meta) -> Node {
    n.meta = meta;
    n
}

/// A plain column (`(depth, item, column)`), for the cost model.
fn col_ref(e: &PE) -> Option<phys::Ref> {
    match e {
        PE::Col { depth, item, col } => Some((*depth, *item, *col)),
        _ => None,
    }
}

/// The items an expression reads at depth 0 (`u16::MAX` for an enclosing row, a subquery, an
/// aggregate or a window value), for the cost model.
fn items_read(e: &PE, out: &mut Vec<u16>) {
    let mut sub = |x: &PE, out: &mut Vec<u16>| items_read(x, out);
    match e {
        PE::Lit(_) => {}
        PE::Col { depth: 0, item, .. } => out.push(*item),
        PE::Col { .. } | PE::Agg(_) | PE::Win(_) | PE::Exists { .. } | PE::Scalar(_) | PE::InSub { .. } => out.push(u16::MAX),
        PE::Not(x) | PE::Neg(x) | PE::Cast(x, _) | PE::IsNull(x, _) => sub(x, out),
        PE::Cmp(_, l, r, _) | PE::Arith(_, l, r) | PE::Concat(l, r) | PE::And(l, r) | PE::Or(l, r) | PE::Is(l, r, _) | PE::Like(l, r, _) => {
            sub(l, out);
            sub(r, out)
        }
        PE::Func(_, a) => a.iter().for_each(|x| sub(x, out)),
        PE::Case { operand, whens, else_, .. } => {
            operand.iter().chain(else_.iter()).for_each(|x| sub(x, out));
            whens.iter().for_each(|(w, t)| {
                sub(w, out);
                sub(t, out)
            });
        }
        PE::InList { e, list, .. } => {
            sub(e, out);
            list.iter().for_each(|(x, _)| sub(x, out))
        }
    }
}

/// A bag recursive CTE's bound (`slot`: the slot its recursive arms read), with the most rows one
/// initial row derives: W1's walk along a forest (`sql::engine::walk_bound`, checked per content
/// by the executor; where it fails the recursion runs under the budget, counted): `d`; M1, a
/// shrinking text (checked per round): `n`; M2, a counter up to a value of the input (its probe a
/// key checked per content, K1): `n`; or a counter: every recursive arm reads the CTE as its
/// first item, keeps only rows whose column `c` exceeds an integer constant and projects `c - k`
/// (`k > 0`) back into position `c`: `n` (the values of the content). The flag: the measure
/// holds only where its per-content check passes (W1, M1, M2).
fn measure(rec: &[PBody], slot: usize) -> Option<(String, Mono, bool)> {
    use crate::sql::engine::{counter_up, shrinking_text, walk_bound};
    if let Some(w) = walk_bound(slot, rec) {
        let cols = |c: &[u16]| c.iter().map(|c| format!("#{c}")).collect::<Vec<_>>().join(", ");
        return Some((format!("W1 walk along a forest ({}) → ({}), checked per content", cols(&w.from), cols(&w.to)), Mono { n: 0, d: 1, log: 0 }, true));
    }
    if let Some(c) = shrinking_text(slot, rec) {
        return Some((format!("M1 shrinking text #{c}, checked per round"), Mono::N, true));
    }
    // M2's key is checked per content on a stored relation (K1, `rel::run`); a CTE or FROM
    // subquery is checked nowhere, so its counter is not a measure.
    let stored = matches!(rec, [PBody::Select(s)] if matches!(s.items.get(1).map(|i| &i.src), Some(Src::Global(_))));
    if let Some((c, _)) = counter_up(slot, rec).filter(|_| stored) {
        return Some((format!("M2 counter #{c} up to a value of the probed input"), Mono::N, true));
    }
    let counter = |s: &PSelect| -> Option<usize> {
        if !matches!(s.items.first().map(|i| &i.src), Some(Src::Cte(x)) if *x == slot) {
            return None;
        }
        let is_c = |e: &PE, c: usize| matches!(e, PE::Col { depth: 0, item: 0, col } if *col as usize == c);
        let int = |e: &PE| matches!(e, PE::Lit(Lit::Int(_)));
        (0..s.proj.len()).find(|&c| {
            let dec = matches!(&s.proj[c], PE::Arith('-', l, r) if is_c(l, c) && matches!(**r, PE::Lit(Lit::Int(k)) if k > 0));
            let floor = s.level.first().is_some_and(|l| {
                l.iter().any(|e| match e {
                    PE::Cmp(Cmp::Gt | Cmp::Ge, a, b, _) => is_c(a, c) && int(b),
                    PE::Cmp(Cmp::Lt | Cmp::Le, a, b, _) => int(a) && is_c(b, c),
                    _ => false,
                })
            });
            dec && floor
        })
    };
    let found: Option<Vec<usize>> = rec.iter().map(|b| if let PBody::Select(s) = b { counter(s) } else { None }).collect();
    found.filter(|f| !f.is_empty()).map(|f| (format!("counter #{}", f[0]), Mono::N, false))
}

/// The keys the measure (`measure`, same order) relies on, checked per content: W1's lookups
/// (items 2.. of its step, checked by the executor before the recursion runs), or M2's probed
/// stored relation (item 1, checked by `rel::run` as K1). `(relation, probed columns)`.
fn checked_keys(rec: &[PBody], batch: usize) -> Vec<(String, Vec<u16>)> {
    use crate::sql::engine::{counter_up, shrinking_text, walk_bound};
    let [PBody::Select(s)] = rec else { return Vec::new() };
    if walk_bound(batch, rec).is_some() {
        return (2..s.items.len())
            .filter_map(|k| {
                let name = match &s.items[k].src {
                    Src::Global(n) => n.clone(),
                    Src::Cte(slot) => format!("cte{slot}"),
                    Src::Derived(i) => format!("derived{i}"),
                    Src::JsonEach(_) => return None,
                };
                Some((name, s.probes[k].as_ref()?.cols.clone()))
            })
            .collect();
    }
    match (shrinking_text(batch, rec), counter_up(batch, rec), &s.items.get(1).map(|i| &i.src)) {
        (None, Some((_, cols)), Some(Src::Global(n))) => vec![(n.clone(), cols)],
        _ => Vec::new(),
    }
}

struct Ctx<'s> {
    st: &'s Statement,
}

impl Ctx<'_> {
    fn pe(&self, e: &PE) -> String {
        pe(self.st, e)
    }

    fn query(&self, q: &PQuery) -> Node {
        let mut n = self.body(&q.body);
        if !q.order.is_empty() || q.limit.is_some() || q.offset.is_some() {
            n = node(
                Op::Sort { keys: q.order.iter().map(|(c, d)| format!("#{c}{}", if *d { " DESC" } else { "" })).collect(), limit: q.limit.as_ref().map(|e| self.pe(e)), offset: q.offset.as_ref().map(|e| self.pe(e)) },
                vec![n],
            );
        }
        if q.ctes.is_empty() {
            return n;
        }
        let mut children: Vec<Node> = q
            .ctes
            .iter()
            .map(|c| match c {
                PCte::Plain { q, slot } => with_meta(self.query(q), Meta { binds: Some(format!("cte{slot}")), ..Meta::default() }),
                PCte::Recursive { init, rec, distinct, slot, batch, .. } => {
                    let m = if *distinct { None } else { measure(rec, *batch) };
                    let measure_checked = m.as_ref().is_some_and(|m| m.2);
                    with_meta(
                        node(Op::Fixpoint { seminaive: true, set: *distinct, closure: closure_applies(c) }, init.iter().chain(rec).map(|b| self.body(b)).collect()),
                        Meta { binds: Some(format!("cte{slot}")), delta: Some(format!("cte{batch}")), init: init.len(), measure: m.map(|(s, mono, _)| (s, mono)), measure_checked, checked_keys: if *distinct { vec![] } else { checked_keys(rec, *batch) }, paths: crate::sql::engine::extends_path(*batch, rec), ..Meta::default() },
                    )
                }
            })
            .collect();
        children.push(n);
        node(Op::Project { exprs: vec!["with".into()] }, children)
    }

    fn body(&self, b: &PBody) -> Node {
        match b {
            PBody::Select(s) => self.select(s),
            PBody::Compound(l, op, r, _) => node(Op::Union { all: matches!(op, SetOp::UnionAll) }, vec![self.body(l), self.body(r)]),
        }
    }

    fn select(&self, s: &PSelect) -> Node {
        let mut items = Vec::new();
        let mut children = Vec::new();
        for (k, it) in s.items.iter().enumerate() {
            let PItem { src, left, lateral, .. } = it;
            let rel = match src {
                Src::Global(n) => n.clone(),
                Src::Cte(slot) => format!("cte{slot}"),
                Src::Derived(i) => {
                    children.push(with_meta(self.query(&self.st.subs[*i].q), Meta { binds: Some(format!("derived{i}")), ..Meta::default() }));
                    format!("derived{i}")
                }
                Src::JsonEach(_) => String::new(),
            };
            let preds: Vec<String> = s.level.get(k).map(|v| v.iter().map(|e| self.pe(e)).collect()).unwrap_or_default();
            let item = match (src, s.probes.get(k).and_then(Option::as_ref), s.or_probes.get(k).and_then(Option::as_ref)) {
                (Src::JsonEach(arg), _, _) if *lateral || k > 0 => Item::LateralJsonEach { arg: self.pe(arg) },
                (Src::JsonEach(arg), _, _) => Item::Constant { rel: format!("json_each({})", self.pe(arg)) },
                // R14: a range probe, within the equality probe's group if there is one.
                (_, p, _) if s.ranges.get(k).and_then(Option::as_ref).is_some() => {
                    let r = s.ranges[k].as_ref().unwrap();
                    let bounds = match (&r.lo, &r.hi) {
                        (Some(_), Some(_)) => "lo,hi",
                        (Some(_), None) => "lo",
                        _ => "hi",
                    };
                    Item::RangeProbe { rel, key_cols: p.map(|p| p.cols.clone()).unwrap_or_default(), range_col: r.col, range_aff: format!("{:?}", r.col_aff), bounds: bounds.into(), residual: preds }
                }
                // A probe at item 0: a correlated subquery's lookup on its correlation.
                (_, Some(p), _) => Item::IndexNestedLoop { rel, key_cols: p.cols.clone(), key_affs: p.col_affs.iter().map(|a| format!("{a:?}")).collect(), residual: preds },
                (_, _, Some(ps)) => Item::OrIndexProbe { rel, branches: ps.iter().map(|p| p.cols.clone()).collect(), residual: preds },
                _ => Item::Scan { rel, pred: preds },
            };
            let probe_refs = |p: &crate::sql::engine::Probe| p.exprs.iter().map(col_ref).collect::<Vec<_>>();
            let refs = match (s.probes.get(k).and_then(Option::as_ref), s.or_probes.get(k).and_then(Option::as_ref)) {
                (Some(p), _) => vec![probe_refs(p)],
                (None, Some(ps)) => ps.iter().map(probe_refs).collect(),
                _ => vec![],
            };
            items.push(PipelineItem { item, left: *left, refs });
        }
        // Subqueries, in order of first reference.
        let mut refs = Vec::new();
        for e in s.level.iter().flatten().chain(s.post.iter().flatten()).chain(&s.konst).chain(&s.proj).chain(&s.group_by).chain(s.having.iter()).chain(&s.order_keys) {
            subs_of(e, &mut refs);
        }
        for (sub, kind) in refs {
            let sp = &self.st.subs[sub];
            let correlated = !sp.cacheable;
            let op = match kind {
                0 => Op::SemiJoin { correlated },
                1 => Op::AntiJoin { correlated },
                _ => Op::Scalar { correlated },
            };
            children.push(node(op, vec![self.query(&sp.q)]));
        }
        let mut n = node(Op::Pipeline { items }, children);
        if s.grouped || !s.aggs.is_empty() {
            n = node(Op::Group { keys: s.group_by.iter().map(|e| self.pe(e)).collect(), aggs: s.aggs.iter().map(|a| format!("{:?}({}{})", a.f, if a.distinct { "DISTINCT " } else { "" }, a.arg.as_ref().map(|e| self.pe(e)).unwrap_or_default())).collect() }, vec![n]);
            n.meta.cols = s.group_by.iter().map(col_ref).collect();
        }
        for w in &s.wins {
            n = node(Op::Window { partition: w.partition.iter().map(|e| self.pe(e)).collect(), order: w.order.iter().map(|(e, d)| format!("{}{}", self.pe(e), if *d { " DESC" } else { "" })).collect() }, vec![n]);
        }
        n = node(Op::Project { exprs: s.proj.iter().map(|e| self.pe(e)).collect() }, vec![n]);
        n.meta.cols = s.proj.iter().map(col_ref).collect();
        n.meta.reads = s.proj.iter().map(|e| {
            let mut v = Vec::new();
            items_read(e, &mut v);
            v
        }).collect();
        if s.distinct {
            n = node(Op::Distinct { prefix: s.out.cols.len() as u16 }, vec![n]);
        }
        n
    }
}

/// The physical plan of one statement.
pub fn statement(st: &Statement) -> Node {
    Ctx { st }.query(&st.root)
}

/// The physical plans of a relational clause: per stage `(name, plan)`, then the final query.
pub fn clause(plan: &RelPlan) -> Vec<(String, Node)> {
    let mut out = Vec::new();
    for st in &plan.stages {
        match st {
            StagePlan::Query { name, stmt, deferred, .. } => {
                let mut n = statement(stmt);
                for (pos, _, d) in deferred {
                    n = node(Op::Project { exprs: vec![format!("deferred #{pos}")] }, vec![n, statement(d)]);
                }
                out.push((name.clone(), n));
            }
            StagePlan::Reach { name, roots, edges, forest, syntax } => {
                let mut ch = vec![statement(roots)];
                if let Some(e) = edges {
                    ch.push(statement(e));
                }
                // A reachability stage runs its own fixpoint, not the executor's closure operator.
                let op = if *syntax { Op::IntervalProbe { root_col: 1 } } else { Op::Fixpoint { seminaive: true, set: true, closure: false } };
                out.push((name.clone(), with_meta(node(op, ch), Meta { forest: *forest, ..Meta::default() })));
            }
        }
    }
    out.push(("query".into(), statement(&plan.fin)));
    out
}

/// The declared sizes of a relational clause's leaves (SCHEMA § 1.2.1; docs/facts/PHASE-3.md
/// § P3b): an input is a bag of at most one row per matched root (`n`), or per root and `each`
/// element (`n·d`: an element is a node under its root); the syntax tables hold one row per kept
/// node keyed by `id`, one per edge keyed by `dst`, one per kept node keyed by `id` (ownership).
fn declare_leaves(a: &mut phys::Analyzer, rule: &eval::Rule, plan: &RelPlan) {
    for (name, i) in &plan.inputs {
        let rows = if rule.inputs[*i].each.is_some() { Bound::of(Mono::ND) } else { Bound::n() };
        // `__path` (column 0): every row of one content holds the content's path.
        a.declare(name, Card { consts: [0].into(), ..Card::new(rows, &[]) });
    }
    if let Some(s) = &plan.syntax {
        for (name, schema) in s.schemas() {
            let col = |c: &str| schema.cols.iter().position(|x| x == c).map(|p| p as u16);
            let key = if s.edges.as_ref() == Some(&name) { col("dst") } else { col("id") };
            let consts = col("__path").into_iter().collect();
            a.declare(&name, Card { rows: Bound::n(), keys: key.map(|k| vec![[k].into()]).unwrap_or_default(), width: Some(schema.cols.len()), consts });
        }
    }
}

/// A pattern input's matcher work per content: its lowered plan's bound, or the leaf
/// executor's (`ast::Eval`: per root, relation searches over its subtree and ancestors; verdicts
/// of capture-free utilities memoised per node).
pub fn input_cost(rule: &eval::Rule, i: usize) -> (Bound, String) {
    match &rule.inputs[i].phys {
        Ok(p) => p.cost(&rule.preds),
        Err(_) => (Bound::of(Mono::ND), "leaf executor: relation searches over subtrees and ancestors".into()),
    }
}

/// The cost of one clause per content (docs/facts/PHASE-3.md § P3b): its inputs' matcher work,
/// then, for a relational clause, every stage and the final query; analysed twice, with the
/// per-content checks (K1 keys, W1/M1/M2 measures) and without them (the fallback class).
pub fn clause_cost(rule: &eval::Rule, c: &eval::Clause) -> PlanCost {
    let run = |mut a: phys::Analyzer| {
        let input = |a: &mut phys::Analyzer, i: usize, name: &str| {
            let (w, what) = input_cost(rule, i);
            a.charge_external(w, format!("input {name}: {what}"));
        };
        match &c.kind {
            eval::Kind::Relational(plan) => {
                declare_leaves(&mut a, rule, plan);
                for (name, i) in &plan.inputs {
                    input(&mut a, *i, name);
                }
                for (name, n) in clause(plan) {
                    a.stage(&name, &n);
                }
            }
            eval::Kind::ForbidAst { input: i } => input(&mut a, *i, "ast"),
            eval::Kind::Budget { input: i, .. } => {
                input(&mut a, *i, "evidence");
                a.charge_external(Bound::n(), "budget: rows grouped by hashing".into());
            }
            eval::Kind::ForbidRegex(_) => a.charge_external(Bound::n(), "regex over the content (linear in its bytes)".into()),
        }
        a.finish()
    };
    run(phys::Analyzer::new()).with_fallback(&run(phys::Analyzer::unchecked()))
}

/// One entry of the class overlay (`packs/data-layer/candidates/classes.json`, keyed
/// `check/clause`; docs/facts/PHASE-3.md § P3b): the class a clause is admitted at, where the
/// census finds the class set (its stage and operator), and why the rule keeps that shape. The
/// rule files carry no `class`: the declarations live on the facts side.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Declared {
    pub class: String,
    pub stage: String,
    pub operator: String,
    pub reason: String,
}

impl Declared {
    pub fn class(&self) -> Option<phys::Class> {
        phys::Class::parse(&self.class)
    }
}

/// The class overlay next to the rule files' directory (`<rules>/../classes.json`), by
/// `check/clause`; none when the file is absent. A malformed overlay is a compile error.
pub fn overlay(rules_dir: &str) -> std::collections::BTreeMap<String, Declared> {
    let path = std::path::Path::new(rules_dir).parent().map(|p| p.join("classes.json"));
    let Some(text) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) else { return Default::default() };
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.as_ref().unwrap().display()));
    let out: std::collections::BTreeMap<String, Declared> = serde_json::from_value(doc["clauses"].clone()).unwrap_or_else(|e| panic!("{}: clauses: {e}", path.as_ref().unwrap().display()));
    for (k, d) in &out {
        assert!(d.class().is_some(), "{}: {k}: class {:?} is not a class", path.as_ref().unwrap().display(), d.class);
    }
    out
}

/// The census's and `EXPLAIN`'s warning for a clause: an unbounded class the overlay does not
/// declare, or a class above the overlay's declaration. Never a refusal: the clause runs under
/// its algebraic budget (`rerail_facts::budget`) and is gate residue (`class-residue`).
pub fn warning(c: &eval::Clause, cost: &PlanCost) -> Option<String> {
    match c.declared.as_ref().and_then(Declared::class) {
        None if cost.class == phys::Class::Unbounded => Some(format!("warning: unbounded class without a class overlay entry (packs/data-layer/candidates/classes.json), set by {}", cost.setter)),
        Some(d) if cost.class > d => Some(format!("warning: class {} above the overlay's declared {d}, set by {}", cost.class, cost.setter)),
        _ => None,
    }
}

/// The EXPLAIN text of one rule's relational clauses that apply to `path`; `classes[ci]` is
/// clause `ci`'s class line (and its warning, if any).
fn explain_rule(rule: &eval::Rule, classes: &[String], path: &str, audit: bool, out: &mut String, counts: &mut (usize, usize)) {
    if !rule.scope.matches(path) {
        return;
    }
    for (ci, c) in rule.clauses.iter().enumerate() {
        let eval::Kind::Relational(plan) = &c.kind else { continue };
        let _ = writeln!(out, "clause {}/{}", rule.check, c.id);
        for l in classes[ci].lines() {
            let _ = writeln!(out, "  {l}");
        }
        for (name, n) in clause(plan) {
            let _ = writeln!(out, "  stage {name}");
            for l in phys::explain(&n).lines() {
                let _ = writeln!(out, "    {l}");
            }
            let found = phys::audit(&n);
            counts.0 += 1;
            counts.1 += found.len();
            if audit {
                for f in found {
                    let _ = writeln!(out, "    ! {f}");
                }
            }
        }
    }
}

/// Every rule file's check (`data-layer-<check>.json`), sorted.
fn all_checks(rules_dir: &str) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(rules_dir)
        .unwrap_or_else(|e| panic!("{rules_dir}: {e}"))
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("data-layer-")?.strip_suffix(".json").map(|c| c.replace('-', "_")))
        .collect();
    out.sort();
    out
}

/// `facts-a explain --classes [--rules DIR] [--checks a,b] [--json FILE]`: the class census
/// (docs/facts/PHASE-3.md G3b.1). Every clause of every selected rule (default: every rule file
/// of the pack) with its kind, class, the operator that sets it and its work bound; a rule that
/// does not compile is listed with its refusal. Markdown on stdout, the rows as JSON in `--json`.
pub fn census(args: &[String]) {
    let rules_dir = arg(args, "--rules").unwrap_or_else(|| "packs/data-layer/candidates/rules".into());
    let checks = if arg(args, "--checks").is_some() { crate::repo::selected_checks(args) } else { all_checks(&rules_dir) };
    let (rules, refused) = crate::repo::compile_checks(&rules_dir, &checks);
    let mut rows = Vec::new();
    for r in &rules {
        for c in &r.clauses {
            let kind = match &c.kind {
                eval::Kind::Relational(p) if p.per_file => "relational, per file",
                eval::Kind::Relational(_) => "relational, repository",
                eval::Kind::ForbidAst { .. } => "forbid ast",
                eval::Kind::ForbidRegex(_) => "forbid regex",
                eval::Kind::Budget { .. } => "budget",
            };
            let cost = clause_cost(r, c);
            // Every fixpoint's bound, and every term beyond O(n²) (what the class leaves unbounded).
            let fixpoints: Vec<String> = cost.terms.iter().filter(|t| t.what.contains(": fixpoint ") && !t.what.contains("per derivation")).map(|t| format!("{}: {}", t.what, t.work.text())).collect();
            let unbounded: Vec<String> = cost.terms.iter().filter(|t| t.work.class() == phys::Class::Unbounded).map(|t| format!("{} ({})", t.what, t.work.text())).collect();
            // Where the class is set: `stage: operator`.
            let (stage, operator) = cost.setter.split_once(": ").map_or(("", cost.setter.as_str()), |(s, o)| (s, o));
            rows.push(serde_json::json!({"check": r.check, "clause": c.id, "kind": kind, "class": cost.class.name(), "set_by": cost.setter, "stage": stage, "operator": operator,
                "proven": cost.proven(), "fallback": cost.fallback.name(), "fallback_set_by": cost.fallback_setter, "checked_keys": cost.checked_keys,
                "work": cost.work.text(), "declared": c.declared, "warning": warning(c, &cost), "fixpoints": fixpoints, "unbounded_terms": unbounded}));
        }
    }
    // Overlay entries that name no clause, or a clause that is no longer above O(n²).
    let overlay = overlay(&rules_dir);
    let stale: Vec<String> = overlay
        .keys()
        .filter(|k| !rows.iter().any(|r| format!("{}/{}", r["check"].as_str().unwrap(), r["clause"].as_str().unwrap()) == **k && r["class"] == "unbounded"))
        .cloned()
        .collect();
    let mut by: std::collections::BTreeMap<&str, (usize, usize)> = Default::default();
    for row in &rows {
        let e = by.entry(row["class"].as_str().unwrap()).or_default();
        if row["proven"] == true { e.0 += 1 } else { e.1 += 1 }
    }
    let warnings: Vec<&serde_json::Value> = rows.iter().filter(|r| !r["warning"].is_null()).collect();
    let mut md = format!(
        "{} rules ({} refused at compile), {} clauses: {}\n\nproven: the class holds on every content; assumed: it rests on per-content checks (K1 keys, W1/M1/M2 measures), and a content where one fails is counted (`bounds_unproven`) and budgeted at the fallback class.\n\n",
        checks.len(),
        refused.len(),
        rows.len(),
        by.iter().map(|(k, (p, a))| if *k == "unbounded" { format!("{k} {}", p + a) } else { format!("{k} {} (proven {p}, assumed {a})", p + a) }).collect::<Vec<_>>().join(", ")
    );
    let _ = writeln!(md, "{} warnings (unbounded without a class overlay entry, or above it); {} stale overlay entries{}\n", warnings.len(), stale.len(), if stale.is_empty() { String::new() } else { format!(": {}", stale.join(", ")) });
    for w in &warnings {
        let _ = writeln!(md, "- {}/{}: {}", w["check"].as_str().unwrap(), w["clause"].as_str().unwrap(), w["warning"].as_str().unwrap());
    }
    md.push_str("\n| check | clause | kind | class | proven | set by |\n|---|---|---|---|---|---|\n");
    for row in &rows {
        let proven = if row["proven"] == true { "proven".to_string() } else { format!("assumed ({} where a check fails)", row["fallback"].as_str().unwrap()) };
        let _ = writeln!(md, "| {} | {} | {} | {} | {proven} | {} |", row["check"].as_str().unwrap(), row["clause"].as_str().unwrap(), row["kind"].as_str().unwrap(), row["class"].as_str().unwrap(), row["set_by"].as_str().unwrap().replace('|', "\\|"));
    }
    for f in &refused {
        let _ = writeln!(md, "| {} | — | refused at compile | — | — | {} |", f.check, f.message.replace('|', "\\|"));
    }
    print!("{md}");
    if let Some(p) = arg(args, "--json") {
        let refused: Vec<_> = refused.iter().map(|f| serde_json::json!({"check": f.check, "message": f.message})).collect();
        std::fs::write(&p, serde_json::to_string_pretty(&serde_json::json!({"format": "rerail.facts-classes/v2", "clauses": rows, "refused": refused, "stale_overlay": stale})).unwrap()).unwrap();
    }
}

/// `facts-a explain --physical …` (see the module doc).
pub fn run(args: &[String]) {
    let rules_dir = arg(args, "--rules").unwrap_or_else(|| "packs/data-layer/candidates/rules".into());
    let audit = args.iter().any(|a| a == "--audit");
    let mut files: Vec<String> = crate::args_all(args, "--file");
    if let Some(root) = arg(args, "--root") {
        let mut all = crate::repo::walk(std::path::Path::new(&root));
        if let Some(l) = arg(args, "--limit") {
            all.truncate(l.parse().expect("--limit"));
        }
        files.extend(all);
    }
    let (rules, refused) = crate::repo::compile_checks(&rules_dir, &crate::repo::selected_checks(args));
    for f in &refused {
        eprintln!("explain --physical: {} refused: {}", f.check, f.message);
    }
    let only = arg(args, "--rule");
    // Classes are per clause (plans are compiled once per rule): computed once, printed per file.
    let classes: Vec<Vec<String>> = rules
        .iter()
        .map(|r| {
            r.clauses
                .iter()
                .map(|c| {
                    let cost = clause_cost(r, c);
                    warning(c, &cost).map_or_else(|| cost.line(), |w| format!("{}\n{w}", cost.line()))
                })
                .collect()
        })
        .collect();
    let mut counts = (0usize, 0usize);
    for f in &files {
        let mut out = String::new();
        let _ = writeln!(out, "== file {f}");
        for (r, cl) in rules.iter().zip(&classes) {
            if only.as_ref().is_some_and(|o| *o != r.check && *o != r.id) {
                continue;
            }
            explain_rule(r, cl, f, audit, &mut out, &mut counts);
        }
        print!("{out}");
    }
    eprintln!("explain --physical: {} files, {} stage plans, {} inner scans (audit)", files.len(), counts.0, counts.1);
}

/// `name=value` pairs of a counts object (JSON field order).
fn pairs<T: serde::Serialize>(c: &T) -> String {
    match serde_json::to_value(c) {
        Ok(serde_json::Value::Object(m)) => m.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    }
}

/// `facts-a explain --analyze FILE [--rules DIR] [--checks a,b] [--rule ID]`: EXPLAIN ANALYZE for
/// one content. The selected checks (default batch 6, production configuration, as `repo`) run
/// over `FILE` alone (its repository clauses over a partition of that one file); per clause in
/// scope, its counts (`crate::counts`: algebraic, then physical), then per stage its counts and
/// its physical plan; then each input's counts. Two runs print the same text (no timing).
pub fn analyze(args: &[String]) {
    let file = arg(args, "--analyze").expect("--analyze FILE");
    let rules_dir = arg(args, "--rules").unwrap_or_else(|| "packs/data-layer/candidates/rules".into());
    let (rules, refused) = crate::repo::compile_checks(&rules_dir, &crate::repo::selected_checks(args));
    let only = arg(args, "--rule");
    let rules: Vec<eval::Rule> = rules.into_iter().filter(|r| only.as_ref().is_none_or(|o| *o == r.check || *o == r.id)).collect();
    for f in &refused {
        println!("refused {}: {}", f.check, f.message);
    }
    let source = std::sync::Arc::new(std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{file}: {e}")));
    let mut out = eval::Output::default();
    let mut repo = crate::spill::Memory::default();
    let mut budget = std::collections::BTreeMap::new();
    let nodes = match crate::facts::build(&file, source.clone()) {
        Ok(crate::facts::Built::Facts(f)) => {
            let ctx = eval::FileCtx { f: &f, path: &file, source: &source };
            eval::eval_file(&rules, &ctx, &mut out, &mut repo, &mut budget);
            f.len()
        }
        _ => {
            println!("== file {file}: could not be parsed");
            return;
        }
    };
    eval::finish(&rules, |k| repo.0.remove(&k).unwrap_or_default(), budget, &mut out);
    let mut text = format!("== file {file} ({nodes} nodes)\n");
    for (ri, rule) in rules.iter().enumerate() {
        for (ci, c) in rule.clauses.iter().enumerate() {
            let Some(counts) = out.clause_counts.get(&(ri, ci)) else { continue };
            let kind = match &c.kind {
                eval::Kind::Relational(p) if p.per_file => "relational, per file",
                eval::Kind::Relational(_) => "relational, repository",
                eval::Kind::ForbidAst { .. } => "forbid ast",
                eval::Kind::ForbidRegex(_) => "forbid regex",
                eval::Kind::Budget { .. } => "budget",
            };
            let found = out.findings.iter().filter(|x| x.check == rule.check && x.clause == c.id).count();
            let failed = out.failures.iter().filter(|x| x.check == rule.check && x.clause == c.id).count();
            let _ = writeln!(text, "clause {}/{} ({kind}): {found} findings, {failed} failures", rule.check, c.id);
            let _ = writeln!(text, "  counts {}", pairs(counts));
            let eval::Kind::Relational(plan) = &c.kind else { continue };
            let plans: Vec<(String, Node)> = clause(plan);
            for (name, n) in plans.iter().map(|(n, p)| (if n == "query" { "<final>" } else { n.as_str() }, p)) {
                match out.stage_counts.get(&(ri, ci, name.to_owned())) {
                    Some(s) => {
                        let phys: Vec<String> = crate::counts::PHYSICAL.iter().zip(&s.phys).filter(|(_, v)| **v > 0).map(|(k, v)| format!("{k}={v}")).collect();
                        let _ = writeln!(text, "  stage {name}: rows={} fix={:?} {}", s.rows, s.fix, phys.join(" "));
                    }
                    None => {
                        let _ = writeln!(text, "  stage {name}: not run");
                    }
                }
                for l in phys::explain(n).lines() {
                    let _ = writeln!(text, "    {l}");
                }
            }
        }
        for ((r, i), c) in out.input_counts.iter().filter(|((r, _), _)| *r == ri) {
            let _ = writeln!(text, "input {}#{i}: {}", rules[*r].check, pairs(c));
        }
    }
    print!("{text}");
}
