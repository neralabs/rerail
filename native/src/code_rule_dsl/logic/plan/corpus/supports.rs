//! Exact maintenance without delete-and-rederive, for comparing with it (`RERAIL_MAINTAIN=count`): counting, held
//! as support sets, because the engine holds sets.
//!
//! Each rule of a non-recursive stratum gets its own relation of derivations: the head's columns, then every other
//! variable its positive atoms bind (a wildcard there becomes a variable). A row of it has exactly one derivation,
//! so its deletes are exact and it needs no rederivation (`maintain` reads `__sup` relations so). The head is their
//! projection, and a head row is gone when its last support is: one probe, a count reaching zero.
//!
//! ```text
//! h(x) :- p(x, y), q(y, _).      becomes    h__sup0(x, y, w0) :- p(x, y), q(y, w0).
//!                                           h(x) :- h__sup0(x, _, _).
//! ```
//!
//! A recursive stratum is derived again (`maintain`): counting over cycles needs a witness of each row's level,
//! which this comparison leaves out. A rule with an aggregate, `nearest`, a matcher, or a variable no positive atom
//! binds keeps its form; its stratum is maintained as before.

use super::lower;
use crate::code_rule_dsl::logic::front::{self, Atom, Literal, Rule, Term};
use std::collections::{BTreeMap, BTreeSet};

/// Whether `RERAIL_MAINTAIN=count` asks for counting rather than delete-and-rederive.
pub(in crate::code_rule_dsl) fn counting() -> bool {
    std::env::var("RERAIL_MAINTAIN").is_ok_and(|v| v == "count")
}

/// A support relation of `rel`'s: a row of it is one derivation.
pub(in crate::code_rule_dsl) fn is_support(rel: &str) -> bool {
    rel.contains("__sup")
}

/// `cross` with each rule of a non-recursive stratum through its own support relation; with how many rules were
/// so, and how many kept their form.
pub(in crate::code_rule_dsl) fn supports(cross: &lower::Lowered) -> (lower::Lowered, usize, usize) {
    let mut out = cross.clone();
    let prog = &cross.program;
    let Ok((sccs, _)) = front::strata(prog) else { return (out, 0, 0) };
    let gated: BTreeSet<&String> = prog.gates.iter().map(|(g, _)| g).collect();
    let mut rules: Vec<Rule> = Vec::new();
    let (mut through, mut kept) = (0, 0);
    let recursive: BTreeSet<&String> = sccs
        .iter()
        .filter(|scc| scc.len() > 1 || prog.rules.iter().any(|r| scc.contains(&r.head.rel) && r.body.iter().any(|l| l.atom().is_some_and(|a| scc.contains(&a.rel)))))
        .flatten()
        .collect();
    let mut n: BTreeMap<String, usize> = BTreeMap::new();
    for rule in &prog.rules {
        let h = &rule.head.rel;
        let eligible = !recursive.contains(h) && !gated.contains(h) && !prog.natives.contains_key(h) && !prog.lattice.contains_key(h) && !prog.nullary.contains(h);
        match eligible.then(|| support(prog, rule, n.get(h).copied().unwrap_or(0))).flatten() {
            Some((name, decl, made)) => {
                *n.entry(h.clone()).or_default() += 1;
                let width = decl.len() - rule.head.args.len();
                let mut args = rule.head.args.clone();
                args.extend((0..width).map(|_| Term::Wild));
                out.program.decls.insert(name.clone(), decl);
                if let Some(d) = cross.domains.get(h).cloned() {
                    out.domains.entry(name.clone()).or_insert(d);
                }
                rules.push(made);
                rules.push(Rule { head: rule.head.clone(), body: vec![Literal::Atom(Atom { rel: name, args, neg: false, gated: false })] });
                through += 1;
            }
            None => {
                rules.push(rule.clone());
                kept += 1;
            }
        }
    }
    out.program.rules = rules;
    (out, through, kept)
}

/// Rule `i` of a head as a support relation: its name, columns and rule. `None`: the rule keeps its form.
fn support(prog: &front::Program, rule: &Rule, i: usize) -> Option<(String, Vec<(String, String)>, Rule)> {
    let h = &rule.head.rel;
    let hdecl = prog.decls.get(h)?;
    if !rule.head.args.iter().all(|t| matches!(t, Term::Var(_) | Term::Int(_) | Term::Str(_))) {
        return None;
    }
    // Each variable's type, from the first positive atom column binding it; wildcards there become variables.
    let mut types: BTreeMap<String, String> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut body = Vec::new();
    let mut fresh = 0;
    for l in &rule.body {
        match l {
            Literal::Atom(a) if !a.neg => {
                let decl = prog.decls.get(&a.rel)?;
                let mut args = Vec::new();
                for (c, t) in a.args.iter().enumerate() {
                    let t = match t {
                        Term::Wild => {
                            fresh += 1;
                            Term::Var(format!("rr_w{fresh}"))
                        }
                        t => t.clone(),
                    };
                    if let Term::Var(v) = &t
                        && !types.contains_key(v)
                    {
                        types.insert(v.clone(), decl.get(c)?.1.clone());
                        order.push(v.clone());
                    }
                    args.push(t);
                }
                body.push(Literal::Atom(Atom { args, ..a.clone() }));
            }
            Literal::Atom(_) | Literal::Cmp { .. } => body.push(l.clone()),
            _ => return None,
        }
    }
    // Every variable is bound by a positive atom (a comparison's `$x = e` would bind one without a type).
    for l in &rule.body {
        if !l.vars().iter().all(|v| types.contains_key(v)) {
            return None;
        }
    }
    let head_vars: BTreeSet<String> = rule.head.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(v.clone()) } else { None }).collect();
    let mut decl = hdecl.clone();
    let mut args = rule.head.args.clone();
    for v in order.iter().filter(|v| !head_vars.contains(*v)) {
        decl.push((v.clone(), types[v].clone()));
        args.push(Term::Var(v.clone()));
    }
    let name = format!("{h}__sup{i}");
    Some((name.clone(), decl, Rule { head: Atom { rel: name, args, neg: false, gated: false }, body }))
}
