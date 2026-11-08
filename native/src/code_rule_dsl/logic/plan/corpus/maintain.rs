//! A session's changed strata maintain their rows from deltas (decision 0077 §3, stage 2): delete-and-rederive,
//! written as rules the native engine runs as it runs any program. One program, one evaluation, no second engine.
//!
//! For a changed relation `r` (an input whose rows changed, or a relation of a stratum that runs) the program has its
//! new rows under its own name, its old rows as input `r__old` (from what the last run kept), and its deltas:
//!
//! ```text
//! r__minus(x) :- r__old(x), !r(x).        rows gone
//! r__plus(x)  :- r(x), !r__old(x).        rows new
//! ```
//!
//! A stratum `S` whose rules are joins, negations and comparisons is maintained. For each rule `h :- B` and each
//! position `k` of `B` that can change (an atom of `S`, or of a changed relation):
//!
//! ```text
//! overdelete  h__del(..) :- B over old rows, k as its loss     (q__del, r__minus; a negated !n as n__plus)
//! survivors   h(..)      :- h__old(..), !h__del(..).
//! rederive    h(..)      :- h__del(..), B over new rows.
//! insert      h__ins(..) :- B over new rows, k as its gain     (q__ins, r__plus; a negated !n as n__minus)
//!             h(..)      :- h__ins(..).
//! ```
//!
//! An overdeleted row is a superset of the rows that lost every derivation; a survivor holds in the new rows. A new
//! row not among the old has a derivation using a gained row at some position, by induction on its derivation, so
//! the insert rules reach it.
//!
//! An aggregate over a changed relation `a` is a position too, by its groups: the atom's correlated variables `g`
//! over `a`'s rows gone and new are the groups whose value may have changed.
//!
//! ```text
//! grp(g)      :- a__minus(..).   grp(g) :- a__plus(..).
//! overdelete  h__del(..) :- grp(g), B over old rows (the aggregate over a__old)
//! insert      h__ins(..) :- grp(g), B over new rows
//! ```
//!
//! A head row that read an unchanged group read the same value before and after, so the other positions' rules cover
//! it (with the aggregate over old rows in an overdelete). A stratum with a native operator, a lattice, an `ast` or
//! `nearest` literal, or a changed read whose old rows were not kept, is derived again from its inputs: its rules
//! unchanged.
//! Either way its relations' deltas feed the strata after it, and an empty delta derives nothing there.

use super::retain::{Cut, Retained};
use super::{Error, Result, lower};
use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::logic::front::{self, Atom, Literal, Rule, Term};
use std::collections::BTreeSet;
use std::sync::Arc;

/// The part as a run evaluates it, the old rows it reads, and how its strata were evaluated.
pub(in crate::code_rule_dsl) struct Maintained {
    pub(in crate::code_rule_dsl) program: lower::Lowered,
    /// `r__old` for each changed relation read, from what was kept.
    pub(in crate::code_rule_dsl) sources: Vec<(String, Arc<dyn Source>)>,
    /// Strata maintained from deltas, and derived again.
    pub(in crate::code_rule_dsl) maintained: usize,
    pub(in crate::code_rule_dsl) derived_again: usize,
    /// Of those maintained, strata that are recursive.
    pub(in crate::code_rule_dsl) recursive: usize,
    /// Each stratum derived again, by one of its relations, and why (the O125 spike's report).
    pub(in crate::code_rule_dsl) again: Vec<(String, &'static str)>,
}

fn named(rel: &str, suffix: &str) -> String {
    format!("{rel}__{suffix}")
}

fn atom(rel: String, args: Vec<Term>, neg: bool) -> Literal {
    Literal::Atom(Atom { rel, args, neg, gated: false })
}

fn row(width: usize) -> Vec<Term> {
    (0..width).map(|i| Term::Var(format!("_m{i}"))).collect()
}

/// Whether a stratum's rules can be maintained: joins, negations and comparisons only, heads of variables and
/// constants, and every read that changed has its old rows.
fn maintainable(prog: &front::Program, scc: &BTreeSet<String>, rules: &[&Rule], changed: &BTreeSet<String>, old: &dyn Fn(&str) -> bool) -> bool {
    scc.iter().all(|r| old(r) && !prog.natives.contains_key(r) && !prog.lattice.contains_key(r))
        && rules.iter().all(|rule| {
            rule.head.args.iter().all(|t| matches!(t, Term::Var(_) | Term::Int(_) | Term::Str(_)))
                && rule.body.iter().all(|l| match l {
                    Literal::Atom(a) => !a.gated && (!changed.contains(&a.rel) || scc.contains(&a.rel) || old(&a.rel)),
                    Literal::Cmp { .. } => true,
                    // An aggregate over an earlier stratum's relation (never this one's: aggregation is not recursive).
                    Literal::Agg { atom: a, .. } => {
                        !a.gated && !scc.contains(&a.rel) && a.args.iter().all(|t| !matches!(t, Term::Call(..))) && (!changed.contains(&a.rel) || old(&a.rel))
                    }
                    _ => false,
                })
        })
}

/// `part` (the cut's program) with each changed stratum maintained where it can be, and the old rows it reads.
pub(in crate::code_rule_dsl) fn rewrite(cut: &Cut, kept: Option<&Retained>, cross: &lower::Lowered) -> Result<Maintained> {
    let mut part = cut.program(cross);
    let (sccs, _) = front::strata(&cross.program).map_err(|e| Error::Plan(e.to_string()))?;
    let dirty: Vec<&BTreeSet<String>> = sccs.iter().filter(|scc| scc.iter().all(|r| !cut.clean.contains(r))).collect();
    let why = if kept.is_none() { "nothing kept" } else { "the program has gates" };
    let Some(kept) = kept.filter(|_| cross.program.gates.is_empty()) else {
        let again = dirty.iter().map(|scc| (scc.iter().next().cloned().unwrap_or_default(), why)).collect();
        return Ok(Maintained { program: part, sources: Vec::new(), maintained: 0, derived_again: dirty.len(), recursive: 0, again });
    };
    let old = |rel: &str| kept.old(rel).is_some();
    let mut changed: BTreeSet<String> = cut.changed.iter().cloned().collect();
    let mut rules: Vec<Rule> = Vec::new();
    let mut olds: BTreeSet<String> = BTreeSet::new();
    let mut deltas: BTreeSet<String> = BTreeSet::new();
    let (mut maintained, mut derived_again, mut recursive) = (0, 0, 0);
    let mut again: Vec<(String, &'static str)> = Vec::new();
    let mut fresh: Vec<(String, String)> = Vec::new();
    for scc in dirty {
        let given: Vec<&Rule> = part.program.rules.iter().filter(|r| scc.contains(&r.head.rel)).collect();
        let named_rules: Vec<Rule> = given.iter().map(|r| named_heads(r)).collect();
        let mine: Vec<&Rule> = named_rules.iter().collect();
        if let Some(why) = unmaintainable(&part.program, scc, &mine, &changed, &old) {
            *again.entry(why).or_default() += 1;
            derived_again += 1;
            let name = scc.iter().max_by_key(|r| part.program.rules.iter().filter(|x| &x.head.rel == *r).count()).cloned().unwrap_or_default();
            let reason = if !scc.iter().all(|r| old(r)) {
                "its old rows were not kept"
            } else if scc.iter().any(|r| part.program.natives.contains_key(r) || part.program.lattice.contains_key(r)) {
                "a native or lattice relation"
            } else if mine.iter().any(|r| r.body.iter().any(|l| matches!(l, Literal::Atom(a) if a.gated))) {
                "a gated atom"
            } else if mine.iter().any(|r| r.body.iter().any(|l| !matches!(l, Literal::Atom(_) | Literal::Cmp { .. }))) {
                "an aggregate or other literal"
            } else if mine.iter().any(|r| !r.head.args.iter().all(|t| matches!(t, Term::Var(_) | Term::Int(_) | Term::Str(_)))) {
                "a computed head"
            } else {
                "a changed read whose old rows were not kept"
            };
            again.push((format!("{name} ({} relations)", scc.len()), reason));
            rules.extend(mine.into_iter().cloned());
            changed.extend(scc.iter().cloned());
            continue;
        }
        maintained += 1;
        if mine.iter().any(|r| r.body.iter().any(|l| l.atom().is_some_and(|a| scc.contains(&a.rel)))) {
            recursive += 1;
        }
        // A read that can change: an atom of this stratum, or a changed relation (positive or negated).
        let moves = |a: &Atom| scc.contains(&a.rel) || changed.contains(&a.rel);
        let old_of = |a: &Atom| if moves(a) { named(&a.rel, "old") } else { a.rel.clone() };
        // A literal over old rows: an atom, or an aggregate's atom, that moves reads `r__old`.
        let old_lit = |l: &Literal| match l {
            Literal::Atom(b) => atom(old_of(b), b.args.clone(), b.neg),
            Literal::Agg { var, op, over, atom: b } => {
                Literal::Agg { var: var.clone(), op: op.clone(), over: over.clone(), atom: Atom { rel: old_of(b), ..b.clone() } }
            }
            other => other.clone(),
        };
        for (r, rule) in mine.iter().enumerate() {
            let h = &rule.head.rel;
            for (k, lit) in rule.body.iter().enumerate() {
                if let Literal::Agg { var, op, over, atom: at } = lit
                    && moves(at)
                {
                    // The groups whose aggregate changed: the atom's correlated variables (all but the aggregated
                    // ones, as `plan::ready` reads them) over its rows gone and new. A row of the head that read one
                    // is deleted with the old value and inserted with the new; the other groups' values held.
                    let mut g = at.vars();
                    g.remove(var);
                    if let Some(o) = over {
                        g.remove(o);
                    }
                    if let front::AggOp::Join { by, .. } = op {
                        by.iter().for_each(|b| {
                            g.remove(b);
                        });
                    }
                    let grp = format!("{h}__grp{r}_{k}");
                    let cols = &part.program.decls[&at.rel];
                    let mut decl: Vec<(String, String)> = g
                        .iter()
                        .map(|v| {
                            let c = at.args.iter().position(|t| matches!(t, Term::Var(x) if x == v)).expect("a group variable is the atom's");
                            (v.clone(), cols[c].1.clone())
                        })
                        .collect();
                    let mut key: Vec<Term> = g.iter().cloned().map(Term::Var).collect();
                    if decl.is_empty() {
                        decl.push(("rr_unit".to_owned(), "int".to_owned()));
                        key.push(Term::Int(1));
                        part.program.nullary.insert(grp.clone());
                    }
                    part.program.decls.insert(grp.clone(), decl);
                    for d in ["minus", "plus"] {
                        rules.push(Rule { head: Atom { rel: grp.clone(), args: key.clone(), neg: false, gated: false }, body: vec![atom(named(&at.rel, d), at.args.clone(), false)] });
                    }
                    deltas.insert(at.rel.clone());
                    let mut over_old = vec![atom(grp.clone(), key.clone(), false)];
                    over_old.extend(rule.body.iter().map(old_lit));
                    rules.push(Rule { head: Atom { rel: named(h, "del"), args: rule.head.args.clone(), neg: false, gated: false }, body: over_old });
                    let mut over_new = vec![atom(grp, key, false)];
                    over_new.extend(rule.body.iter().cloned());
                    rules.push(Rule { head: Atom { rel: named(h, "ins"), args: rule.head.args.clone(), neg: false, gated: false }, body: over_new });
                    continue;
                }
                let Literal::Atom(at) = lit else { continue };
                if !moves(at) {
                    continue;
                }
                let (lose, gain) = match (at.neg, scc.contains(&at.rel)) {
                    (false, true) => (named(&at.rel, "del"), named(&at.rel, "ins")),
                    (false, false) => (named(&at.rel, "minus"), named(&at.rel, "plus")),
                    (true, _) => (named(&at.rel, "plus"), named(&at.rel, "minus")),
                };
                if !scc.contains(&at.rel) {
                    deltas.insert(at.rel.clone());
                }
                let over: Vec<Literal> = rule
                    .body
                    .iter()
                    .enumerate()
                    .map(|(j, l)| match l {
                        Literal::Atom(b) if j == k => atom(lose.clone(), b.args.clone(), false),
                        other => old_lit(other),
                    })
                    .collect();
                rules.push(Rule { head: Atom { rel: named(h, "del"), args: rule.head.args.clone(), neg: false, gated: false }, body: over });
                let ins: Vec<Literal> = rule.body.iter().enumerate().map(|(j, l)| if j == k { atom(gain.clone(), at.args.clone(), false) } else { l.clone() }).collect();
                rules.push(Rule { head: Atom { rel: named(h, "ins"), args: rule.head.args.clone(), neg: false, gated: false }, body: ins });
            }
            for b in rule.body.iter().filter_map(|l| match l {
                Literal::Atom(b) | Literal::Agg { atom: b, .. } if moves(b) && !scc.contains(&b.rel) => Some(b),
                _ => None,
            }) {
                olds.insert(b.rel.clone());
            }
            let mut red = vec![atom(named(h, "del"), rule.head.args.clone(), false)];
            red.extend(rule.body.iter().cloned());
            rules.push(Rule { head: rule.head.clone(), body: red });
        }
        for rel in scc {
            let width = part.program.decls[rel].len();
            let v = row(width);
            olds.insert(rel.clone());
            rules.push(Rule { head: Atom { rel: rel.clone(), args: v.clone(), neg: false, gated: false }, body: vec![atom(named(rel, "old"), v.clone(), false), atom(named(rel, "del"), v.clone(), true)] });
            rules.push(Rule { head: Atom { rel: rel.clone(), args: v.clone(), neg: false, gated: false }, body: vec![atom(named(rel, "ins"), v, false)] });
            fresh.push((rel.clone(), named(rel, "del")));
            fresh.push((rel.clone(), named(rel, "ins")));
        }
        changed.extend(scc.iter().cloned());
    }
    for rel in &deltas {
        let v = row(part.program.decls[rel].len());
        rules.push(Rule { head: Atom { rel: named(rel, "minus"), args: v.clone(), neg: false, gated: false }, body: vec![atom(named(rel, "old"), v.clone(), false), atom(rel.clone(), v.clone(), true)] });
        rules.push(Rule { head: Atom { rel: named(rel, "plus"), args: v.clone(), neg: false, gated: false }, body: vec![atom(rel.clone(), v.clone(), false), atom(named(rel, "old"), v, true)] });
        fresh.push((rel.clone(), named(rel, "minus")));
        fresh.push((rel.clone(), named(rel, "plus")));
    }
    let mut sources = Vec::new();
    for rel in &olds {
        let columns = kept.old(rel).expect("a maintained read's old rows");
        // Named as the planner reads its statistics (`sql_col`).
        let names = (0..columns.len()).map(|c| crate::code_rule_dsl::logic::plan::sql_col(&named(rel, "old"), c)).collect();
        sources.push((named(rel, "old"), crate::code_rule_dsl::logic::native::component::columns_source(names, columns.iter().cloned().map(Some).collect())));
        fresh.push((rel.clone(), named(rel, "old")));
        part.program.inputs.push(named(rel, "old"));
    }
    for (rel, made) in fresh {
        let decl = part.program.decls[&rel].clone();
        part.program.decls.insert(made.clone(), decl);
        if let Some(d) = part.domains.get(&rel).cloned() {
            part.domains.insert(made.clone(), d);
        }
        if part.program.nullary.contains(&rel) {
            part.program.nullary.insert(made);
        }
    }
    part.program.rules = rules;
    Ok(Maintained { program: part, sources, maintained, derived_again, recursive, again })
}
