//! Test vectors: Spike B's R1 (dead code: antijoin with counts) and R3 (reachability: demand-
//! driven recursion) written as IR repository plans over Spike B's per-file derivations, which
//! play the exported relations. The generator's output for them is compared, per worktree and
//! after every replayed edit, with Spike B's from-scratch oracle (`crate::oracle`) and with its
//! hand-written delta expansion's results (`crate::flow`), so the generator reproduces the
//! expansions Spike B proved exact. (R2's group logic is Rust closures, not IR; it stays a
//! vector of the hand-written flow only.)
use super::val::{Row, Val};
use crate::derive::World;
use crate::store::{Cid, Sym};
use rerail_facts::ir::{self, Aff, Binding, CmpOp, Column, Expr, Plan, Rel, RepoSource, Sink, Violation};

pub const R1_REF: u32 = 0;
pub const R1_CAND: u32 = 1;
pub const R3_SEED: u32 = 2;
pub const R3_TARGET: u32 = 3;
pub const R3_LOCAL: u32 = 4;
pub const R3_CALLSITE: u32 = 5;
pub const R3_INSCOPE: u32 = 6;

type X = Expr<RepoSource>;

fn c(depth: u16, col: u16) -> X {
    Expr::Col { depth, col }
}

fn eq(a: X, b: X) -> X {
    Expr::Cmp(CmpOp::Eq, Box::new(a), Box::new(b), (Aff::None, Aff::None))
}

fn and(a: X, b: X) -> X {
    Expr::And(Box::new(a), Box::new(b))
}

fn cols(names: &[&str]) -> Vec<Column> {
    names.iter().map(|n| Column::new(n, Aff::None)).collect()
}

fn scan(id: u32, names: &[&str]) -> Rel<RepoSource> {
    Rel::Scan { src: RepoSource::Exported(id), schema: cols(names) }
}

fn project(input: Rel<RepoSource>, exprs: Vec<X>, names: &[&str]) -> Rel<RepoSource> {
    Rel::Project { input: Box::new(input), exprs, schema: cols(names) }
}

fn join(l: Rel<RepoSource>, r: Rel<RepoSource>, on: X) -> Rel<RepoSource> {
    Rel::Join { kind: ir::JoinKind::Inner, left: Box::new(l), right: Box::new(r), on: Some(on) }
}

fn findings(clause: &str, rel: Rel<RepoSource>) -> Plan<RepoSource> {
    Plan { clause: clause.into(), syntax: None, stages: Vec::new(), sink: Sink::Findings { rel, key: Vec::new(), code: None, locations: Vec::new(), violation: Violation { code: clause.into(), anchor: ir::Anchor::Span, each: None } } }
}

/// R1: `SELECT path, qualname FROM cand WHERE NOT EXISTS (SELECT 1 FROM ref WHERE ref.name = cand.name)`.
pub fn r1() -> Plan<RepoSource> {
    let refs = scan(R1_REF, &["name"]);
    let sub = Rel::Filter { input: Box::new(refs), pred: eq(c(0, 0), c(1, 0)) };
    let cand = scan(R1_CAND, &["name", "path", "qualname"]);
    let dead = Rel::Filter { input: Box::new(cand), pred: Expr::Exists { rel: Box::new(sub), negated: true } };
    findings("r1", project(dead, vec![c(0, 1), c(0, 2)], &["path", "qualname"]))
}

/// R3:
/// ```sql
/// WITH RECURSIVE reach(np, nq, fam) AS (
///   SELECT np, nq, fam FROM seed
///   UNION SELECT l.ap, l.aq, r.fam FROM reach r JOIN local l ON l.bp = r.np AND l.bq = r.nq
///   UNION SELECT s.cp, s.cq, r.fam FROM reach r JOIN target t ON t.np = r.np AND t.nq = r.nq
///                                    JOIN callsite s ON s.k1 = t.k1 AND s.k2 = t.k2)
/// SELECT DISTINCT cp, cq, fam FROM (<both steps over reach>) f
///   WHERE EXISTS (SELECT 1 FROM inscope i WHERE i.path = f.cp)
/// ```
pub fn r3() -> Plan<RepoSource> {
    let reach = || Rel::Local { id: 0, schema: cols(&["np", "nq", "fam"]) };
    let via_local = |r: Rel<RepoSource>| {
        // r(np,nq,fam) ++ local(ap,aq,bp,bq)
        let j = join(r, scan(R3_LOCAL, &["ap", "aq", "bp", "bq"]), and(eq(c(0, 5), c(0, 0)), eq(c(0, 6), c(0, 1))));
        project(j, vec![c(0, 3), c(0, 4), c(0, 2)], &["np", "nq", "fam"])
    };
    let via_ext = |r: Rel<RepoSource>| {
        // r(np,nq,fam) ++ target(k1,k2,np,nq) ++ callsite(k1,k2,cp,cq)
        let j1 = join(r, scan(R3_TARGET, &["k1", "k2", "np", "nq"]), and(eq(c(0, 5), c(0, 0)), eq(c(0, 6), c(0, 1))));
        let j2 = join(j1, scan(R3_CALLSITE, &["k1", "k2", "cp", "cq"]), and(eq(c(0, 7), c(0, 3)), eq(c(0, 8), c(0, 4))));
        project(j2, vec![c(0, 9), c(0, 10), c(0, 2)], &["np", "nq", "fam"])
    };
    let fix = Binding::Fix { init: vec![project(scan(R3_SEED, &["np", "nq", "fam"]), vec![c(0, 0), c(0, 1), c(0, 2)], &["np", "nq", "fam"])], step: vec![via_local(reach()), via_ext(reach())], set: true, schema: cols(&["np", "nq", "fam"]) };
    let found = Rel::Union { all: false, left: Box::new(via_local(reach())), right: Box::new(via_ext(reach())) };
    let scoped = Rel::Filter { input: Box::new(found), pred: Expr::Exists { rel: Box::new(Rel::Filter { input: Box::new(scan(R3_INSCOPE, &["path"])), pred: eq(c(0, 0), c(1, 0)) }), negated: false } };
    findings("r3", Rel::With { bindings: vec![(0, fix)], body: Box::new(scoped) })
}

/// RT, a constructed tie (gate mutant 9): `SELECT path, qualname FROM (SELECT *, row_number()
/// OVER (PARTITION BY path, name ORDER BY name) AS rn FROM cand) WHERE rn = 1`. The order is not
/// total (marked `Checked`): a file defining one name under two qualnames (methods of two
/// classes) is a tie between distinct rows, so the worktree's findings must be a live runtime
/// refusal while such a file is a member, and the plain rows otherwise.
pub fn rt() -> Plan<RepoSource> {
    let cand = scan(R1_CAND, &["name", "path", "qualname"]);
    let win = Rel::Window { input: Box::new(cand), funcs: vec![ir::WindowFn { kind: ir::WindowKind::RowNumber, partition: vec![c(0, 1), c(0, 0)], order: vec![(c(0, 0), false)], total: ir::Total::Checked }] };
    let first = Rel::Filter { input: Box::new(win), pred: eq(c(0, 3), Expr::Lit(ir::Lit::Int(1))) };
    findings("rt", project(first, vec![c(0, 1), c(0, 2)], &["path", "qualname"]))
}

/// RT from scratch over a worktree's files: whether a tie exists, and the rows when none does.
pub fn rt_oracle(w: &mut World, files: &[(Sym, Cid)]) -> (bool, Vec<Row>) {
    let mut by_key: std::collections::BTreeMap<(Sym, Sym), std::collections::BTreeSet<Sym>> = std::collections::BTreeMap::new();
    for &(p, c) in files {
        for (n, pp, q) in w.r1_cand(p, c) {
            by_key.entry((pp, n)).or_default().insert(q);
        }
    }
    if by_key.values().any(|qs| qs.len() > 1) {
        return (true, Vec::new());
    }
    let rows: std::collections::BTreeSet<(Sym, Sym)> = by_key.iter().map(|((pp, _), qs)| (*pp, *qs.iter().next().unwrap())).collect();
    (false, rows.into_iter().map(|(pp, q)| Row::from([t(pp), t(q)])).collect())
}

pub fn exported() -> Vec<ir::ExportedRelation> {
    let e = |id, name: &str, n: &[&str]| ir::ExportedRelation { id, name: name.into(), schema: cols(n), distinct: false, ordered: false, producer: 0 };
    vec![
        e(R1_REF, "r1_ref", &["name"]),
        e(R1_CAND, "r1_cand", &["name", "path", "qualname"]),
        e(R3_SEED, "r3_seed", &["np", "nq", "fam"]),
        e(R3_TARGET, "r3_target", &["k1", "k2", "np", "nq"]),
        e(R3_LOCAL, "r3_local", &["ap", "aq", "bp", "bq"]),
        e(R3_CALLSITE, "r3_callsite", &["k1", "k2", "cp", "cq"]),
        e(R3_INSCOPE, "r3_inscope", &["path"]),
    ]
}

/// A world symbol as a dataflow text value: the store's symbols are the dataflow's (one table).
fn t(s: Sym) -> Val {
    Val::text(s)
}

/// Exported rows of one (path, content): Spike B's derivations, as rows.
pub fn rows(w: &mut World, export: u32, p: Sym, c: Cid) -> Vec<Row> {
    match export {
        R1_REF => w.r1_refs(c).into_iter().map(|n| Row::from([t(n)])).collect(),
        R1_CAND => w.r1_cand(p, c).into_iter().map(|(n, pp, q)| Row::from([t(n), t(pp), t(q)])).collect(),
        R3_SEED => w.r3_seeds(p, c).into_iter().map(|((np, nq), f)| Row::from([t(np), t(nq), t(f)])).collect(),
        R3_TARGET => w.r3_target(p, c).into_iter().map(|((k1, k2), (np, nq))| Row::from([t(k1), t(k2), t(np), t(nq)])).collect(),
        R3_LOCAL => w.r3_calls(p, c).0.into_iter().map(|((ap, aq), (bp, bq))| Row::from([t(ap), t(aq), t(bp), t(bq)])).collect(),
        R3_CALLSITE => w.r3_calls(p, c).1.into_iter().map(|((k1, k2), (cp, cq))| Row::from([t(k1), t(k2), t(cp), t(cq)])).collect(),
        R3_INSCOPE => {
            if w.info(p).r3_scope {
                vec![Row::from([t(p)])]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

/// The oracle's findings of a vector as dataflow rows (R1: (path, qualname); R3: (path,
/// qualname, family)).
pub fn oracle_rows(f: &crate::oracle::Findings, plan: &str) -> Vec<Row> {
    match plan {
        "r1" => f.r1.iter().map(|(p, q)| Row::from([t(*p), t(*q)])).collect(),
        "r3" => f.r3.iter().map(|(p, q, fam)| Row::from([t(*p), t(*q), t(*fam)])).collect(),
        _ => Vec::new(),
    }
}
