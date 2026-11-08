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
//! the insert rules reach it. A stratum with an aggregate, a native operator, a lattice, an `ast` or `nearest`
//! literal, or a changed read whose old rows were not kept, is derived again from its inputs: its rules unchanged.
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
                    _ => false,
                })
        })
}

/// `part` (the cut's program) with each changed stratum maintained where it can be, and the old rows it reads.
pub(in crate::code_rule_dsl) fn rewrite(cut: &Cut, kept: Option<&Retained>, cross: &lower::Lowered) -> Result<Maintained> {
    let mut part = cut.program(cross);
    let (sccs, _) = front::strata(&cross.program).map_err(|e| Error::Plan(e.to_string()))?;
    let dirty: Vec<&BTreeSet<String>> = sccs.iter().filter(|scc| scc.iter().all(|r| !cut.clean.contains(r))).collect();
    let Some(kept) = kept.filter(|_| cross.program.gates.is_empty()) else {
        return Ok(Maintained { program: part, sources: Vec::new(), maintained: 0, derived_again: dirty.len(), recursive: 0 });
    };
    let old = |rel: &str| kept.old(rel).is_some();
    let mut changed: BTreeSet<String> = cut.changed.iter().cloned().collect();
    let mut rules: Vec<Rule> = Vec::new();
    let mut olds: BTreeSet<String> = BTreeSet::new();
    let mut deltas: BTreeSet<String> = BTreeSet::new();
    let (mut maintained, mut derived_again, mut recursive) = (0, 0, 0);
    let mut fresh: Vec<(String, String)> = Vec::new();
    for scc in dirty {
        let mine: Vec<&Rule> = part.program.rules.iter().filter(|r| scc.contains(&r.head.rel)).collect();
        if !maintainable(&part.program, scc, &mine, &changed, &old) {
            derived_again += 1;
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
        for rule in &mine {
            let h = &rule.head.rel;
            for (k, lit) in rule.body.iter().enumerate() {
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
                        Literal::Atom(b) => atom(old_of(b), b.args.clone(), b.neg),
                        other => other.clone(),
                    })
                    .collect();
                rules.push(Rule { head: Atom { rel: named(h, "del"), args: rule.head.args.clone(), neg: false, gated: false }, body: over });
                let ins: Vec<Literal> = rule.body.iter().enumerate().map(|(j, l)| if j == k { atom(gain.clone(), at.args.clone(), false) } else { l.clone() }).collect();
                rules.push(Rule { head: Atom { rel: named(h, "ins"), args: rule.head.args.clone(), neg: false, gated: false }, body: ins });
            }
            for b in rule.body.iter().filter_map(|l| match l {
                Literal::Atom(b) if moves(b) && !scc.contains(&b.rel) => Some(b),
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
        let names = (0..columns.len()).map(|c| format!("c{c}")).collect();
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
    Ok(Maintained { program: part, sources, maintained, derived_again, recursive })
}
