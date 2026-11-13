//! Liveness: which of a program's facts its outputs read (note t26).
//!
//! A fact is live if some derivation of an output row uses it. Liveness flows backwards through the rules, so it is
//! itself a program over the program's relations, run to its fixed point after them:
//!
//! ```text
//! // a rule of the program                 // the rules `program` makes from it
//! calls(c, f) :- call(c, x), look(c, x, s), bv(s, f).
//!                                          live_call(c, x)    :- live_calls(c, f), call(c, x), look(c, x, s), bv(s, f).
//!                                          live_look(c, x, s) :- live_calls(c, f), call(c, x), look(c, x, s), bv(s, f).
//!                                          live_bv(s, f)      :- live_calls(c, f), call(c, x), look(c, x, s), bv(s, f).
//! // an output                              live_calls(c, f)   :- calls(c, f).
//! ```
//!
//! - A read inside an aggregate (`n = count : { r(x, _) }`) makes every row of its group live.
//! - A negated read `!r(k)` makes live the rows of `r` that block a derivation: those matching `k` where the rest
//!   of the body holds. Without them the negation would pass, and the outputs would change. These are the keys a
//!   demand path for negations asks for (decision 0048's `demand_R`, `magic::gate_negations`), so the counts show
//!   what such a path would compute.
//! - A head's computed argument (`@dot(q, x)`) is matched by computing it again; a lattice relation is matched by its
//!   keys alone (its value is the join of every derivation's).
//!
//! The counts (`COUNTS`, one output): for every relation read or derived, its rows and live rows, its keys (first
//! column) and live keys, and, for a relation whose first column is a node, the rows about loaded documents (a
//! library's, such as typeshed's stubs: `doc(d, p, _), loaded(p, _, _)`) and the live ones among them. The keys are
//! what `.demand r` (`magic::magic_sets`) restricts a relation to: live keys against all keys is what demand by key
//! would save.
//!
//! This is the measurement before deriving less: `Mode::After` runs the backward rules over the finished fixpoint.
//! The demand path (`Mode::Before`, not built) is the same rules with each body cut to the literals before the read
//! (the sideways order `magic` passes), so that the head is asked for before it is derived.

use std::collections::BTreeSet;

use super::front::{AggOp, Atom, Literal, Program, Rule, Term};

/// The counts' relation: `liveof_count(rel, what, n)`.
pub const COUNTS: &str = "liveof_count";

/// What `COUNTS` counts, in the order the report prints it.
pub const MEASURES: [&str; 6] = ["rows", "live", "keys", "live keys", "library", "library live"];

/// `rel`'s liveness relation, under a prefix no program uses: a program may name its own relations `live_…`
/// (calls_cx has `live_def`).
pub(in crate::code_rule_dsl) fn live(rel: &str) -> String {
    format!("liveof_{rel}")
}

fn atom(rel: String, args: Vec<Term>) -> Atom {
    Atom { rel: rel.into(), args, neg: false, gated: false }
}

/// The backward rules and the counts for `prog` (its rules, declarations and outputs, as `rr::parse` gives them): a
/// program of their own, to print after `prog`'s text, which declares what they read.
pub fn program(prog: &Program) -> Program {
    // A program naming a relation as these do would read the liveness rules' rows as its own.
    if let Some(r) = prog.decls.keys().find(|r| r.starts_with(&live(""))) {
        panic!("{r}: the liveness rules' prefix is taken");
    }
    let mut out = Program::default();
    let mut fresh = 0usize;
    let mut var = || {
        fresh += 1;
        format!("liveof_v{fresh}")
    };
    let declared = |r: &str| prog.decls.contains_key(r) && !prog.nullary.contains(r);
    let mut targets: BTreeSet<String> = BTreeSet::new();
    for o in prog.outputs.iter().filter(|o| declared(o)) {
        let args: Vec<Term> = (0..prog.decls[o.as_str()].len()).map(|i| Term::Var(format!("liveof_vo{i}").into())).collect();
        out.rules.push(Rule { head: atom(live(o), args.clone()), body: vec![Literal::Atom(atom(o.clone(), args))] });
        targets.insert(o.clone());
    }
    for rule in &prog.rules {
        let h = &rule.head.rel;
        if !declared(h) {
            continue;
        }
        // Every derived relation is counted, read or not: a relation nothing reads is all dead rows.
        targets.insert(h.to_string());
        // The head as a read of its liveness: a computed argument is a variable the body computes again.
        let mut computed = Vec::new();
        let lattice_keys = prog.lattice.get(h.as_str()).map(|l| l.keys);
        let head_args: Vec<Term> = rule
            .head
            .args
            .iter()
            .enumerate()
            .map(|(i, t)| match t {
                _ if lattice_keys.is_some_and(|k| i >= k) => Term::Var(var().into()),
                Term::Call(..) => {
                    let v = var();
                    computed.push(Literal::Cmp { l: Term::Var(v.clone().into()), op: "=", r: t.clone() });
                    Term::Var(v.into())
                }
                Term::Wild | Term::Top => Term::Var(var().into()),
                other => other.clone(),
            })
            .collect();
        let live_head = Literal::Atom(atom(live(h), head_args));
        for (i, lit) in rule.body.iter().enumerate() {
            let (read, negated, aggregated) = match lit {
                Literal::Atom(a) => (a, a.neg, false),
                Literal::Agg { atom, .. } => (atom, false, true),
                _ => continue,
            };
            if !declared(&read.rel) {
                continue;
            }
            let args: Vec<Term> = read.args.iter().map(|t| if matches!(t, Term::Wild) { Term::Var(var().into()) } else { t.clone() }).collect();
            let positive = Literal::Atom(atom(read.rel.to_string(), args.clone()));
            let mut body = Vec::with_capacity(rule.body.len() + 2);
            if !negated {
                body.push(live_head.clone());
            }
            for (j, l) in rule.body.iter().enumerate() {
                if j != i {
                    body.push(l.clone());
                } else if aggregated {
                    body.push(l.clone());
                    body.push(positive.clone());
                } else {
                    body.push(positive.clone());
                }
            }
            if !negated {
                body.extend(computed.iter().cloned());
            }
            out.rules.push(Rule { head: atom(live(&read.rel), args), body });
            targets.insert(read.rel.to_string());
        }
    }
    let count = |n: &str, rel: &str, what: &str, of: Atom| Rule {
        head: atom(COUNTS.to_owned(), vec![Term::Str(rel.to_owned().into()), Term::Str(what.to_owned().into()), Term::Var(n.to_owned().into())]),
        body: vec![Literal::Agg { var: n.to_owned(), op: AggOp::Count, over: None, atom: of }],
    };
    out.decls.insert(COUNTS.to_owned().into(), vec![("rel".into(), "symbol".into()), ("what".into(), "symbol".into()), ("n".into(), "number".into())]);
    let library = prog.decls.contains_key("doc") && prog.decls.contains_key("loaded");
    for t in &targets {
        let cols = &prog.decls[t.as_str()];
        let vars = |prefix: &str| -> Vec<Term> { (0..cols.len()).map(|i| Term::Var(format!("{prefix}{i}").into())).collect() };
        out.decls.insert(live(t).into(), cols.clone());
        out.rules.push(count("n", t, MEASURES[0], atom(t.clone(), vars("c"))));
        out.rules.push(count("n", t, MEASURES[1], atom(live(t), vars("c"))));
        // Keys: the first column's values, of all rows and of the live ones.
        let key = |of: &str| format!("liveof_key{of}_{t}");
        for (name, rel) in [(key(""), t.clone()), (key("l"), live(t))] {
            out.decls.insert(name.clone().into(), vec![cols[0].clone()]);
            let mut args = vec![Term::Var("k".into())];
            args.extend((1..cols.len()).map(|_| Term::Wild));
            out.rules.push(Rule { head: atom(name.clone(), vec![Term::Var("k".into())]), body: vec![Literal::Atom(atom(rel, args))] });
        }
        out.rules.push(count("n", t, MEASURES[2], atom(key(""), vec![Term::Var("k".into())])));
        out.rules.push(count("n", t, MEASURES[3], atom(key("l"), vec![Term::Var("k".into())])));
        // Rows about a loaded document: a library's.
        if library && cols[0].1 == "node" {
            for (name, rel, what) in [(format!("liveof_lib_{t}"), t.clone(), MEASURES[4]), (format!("liveof_libl_{t}"), live(t), MEASURES[5])] {
                out.decls.insert(name.clone().into(), cols.clone());
                let args = vars("c");
                let d = Term::Var("liveof_vd".into());
                let p = Term::Var("liveof_vp".into());
                out.rules.push(Rule {
                    head: atom(name.clone(), args.clone()),
                    body: vec![
                        Literal::Atom(atom(rel, args.clone())),
                        Literal::Cmp { l: d.clone(), op: "=", r: Term::Call("doc".into(), vec![args[0].clone()]) },
                        Literal::Atom(atom("doc".into(), vec![d, p.clone(), Term::Wild])),
                        Literal::Atom(atom("loaded".into(), vec![p, Term::Wild, Term::Wild])),
                    ],
                });
                out.rules.push(count("n", t, what, atom(name, vars("c"))));
            }
        }
    }
    out.outputs.push(COUNTS.to_owned());
    out
}

/// `COUNTS`' rows (`liveof_count.csv`), by relation: each measure's count, absent where the relation had no row.
pub fn read_counts(csv: &str) -> std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>> {
    let mut out: std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>> = Default::default();
    for line in csv.lines() {
        let mut f = line.split('\t');
        if let (Some(rel), Some(what), Some(n)) = (f.next(), f.next(), f.next()) {
            out.entry(rel.to_owned()).or_default().insert(what.to_owned(), n.parse().unwrap_or(0));
        }
    }
    out
}

// Its tests run in the engine's test binary (its facade of this module mounts `tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rr` libraries).

