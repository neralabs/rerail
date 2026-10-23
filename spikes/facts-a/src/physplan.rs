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
use crate::sql::engine::{closure_applies, Lit, PBody, PCte, PE, PItem, PQuery, PSelect, Src, Statement};
use crate::sql::parse::SetOp;
use crate::{arg, eval};
use rerail_facts::phys::{self, Item, Node, Op, PipelineItem};
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
    h.update(phys::explain(&Node { op: op.clone(), digest: String::new(), children: vec![] }).as_bytes());
    for c in &children {
        h.update(c.digest.as_bytes());
    }
    Node { op, digest: h.finalize().to_hex()[..12].to_string(), children }
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
                PCte::Plain { q, .. } => self.query(q),
                PCte::Recursive { init, rec, distinct, .. } => {
                    node(Op::Fixpoint { seminaive: true, set: *distinct, closure: closure_applies(c) }, init.iter().chain(rec).map(|b| self.body(b)).collect())
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
                    children.push(self.query(&self.st.subs[*i].q));
                    format!("derived{i}")
                }
                Src::JsonEach(_) => String::new(),
            };
            let preds: Vec<String> = s.level.get(k).map(|v| v.iter().map(|e| self.pe(e)).collect()).unwrap_or_default();
            let item = match (src, s.probes.get(k).and_then(Option::as_ref), s.or_probes.get(k).and_then(Option::as_ref)) {
                (Src::JsonEach(arg), _, _) if *lateral || k > 0 => Item::LateralJsonEach { arg: self.pe(arg) },
                (Src::JsonEach(arg), _, _) => Item::Constant { rel: format!("json_each({})", self.pe(arg)) },
                // A probe at item 0: a correlated subquery's lookup on its correlation.
                (_, Some(p), _) => Item::IndexNestedLoop { rel, key_cols: p.cols.clone(), key_affs: p.col_affs.iter().map(|a| format!("{a:?}")).collect(), residual: preds },
                (_, _, Some(ps)) => Item::OrIndexProbe { rel, branches: ps.iter().map(|p| p.cols.clone()).collect(), residual: preds },
                _ => Item::Scan { rel, pred: preds },
            };
            items.push(PipelineItem { item, left: *left });
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
        }
        for w in &s.wins {
            n = node(Op::Window { partition: w.partition.iter().map(|e| self.pe(e)).collect(), order: w.order.iter().map(|(e, d)| format!("{}{}", self.pe(e), if *d { " DESC" } else { "" })).collect() }, vec![n]);
        }
        n = node(Op::Project { exprs: s.proj.iter().map(|e| self.pe(e)).collect() }, vec![n]);
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
                let _ = forest;
                out.push((name.clone(), node(op, ch)));
            }
        }
    }
    out.push(("query".into(), statement(&plan.fin)));
    out
}

/// The EXPLAIN text of one rule's relational clauses that apply to `path`.
fn explain_rule(rule: &eval::Rule, path: &str, audit: bool, out: &mut String, counts: &mut (usize, usize)) {
    if !rule.scope.matches(path) {
        return;
    }
    for c in &rule.clauses {
        let eval::Kind::Relational(plan) = &c.kind else { continue };
        let _ = writeln!(out, "clause {}/{}", rule.check, c.id);
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
    let mut counts = (0usize, 0usize);
    for f in &files {
        let mut out = String::new();
        let _ = writeln!(out, "== file {f}");
        for r in &rules {
            if only.as_ref().is_some_and(|o| *o != r.check && *o != r.id) {
                continue;
            }
            explain_rule(r, f, audit, &mut out, &mut counts);
        }
        print!("{out}");
    }
    eprintln!("explain --physical: {} files, {} stage plans, {} inner scans (audit)", files.len(), counts.0, counts.1);
}
