//! Exact maintenance without delete-and-rederive, a session's default (`RERAIL_MAINTAIN=dred` compares with
//! delete-and-rederive): counting, held as support sets, because the engine holds sets.
//!
//! Each rule of a non-recursive stratum gets its own relation of derivations: the head's columns, then every other
//! variable its positive atoms bind (a wildcard there becomes a variable). A row of it has exactly one derivation,
//! so its deletes are exact and it needs no rederivation (`maintain` reads `__sup` relations so). The head is their
//! projection, and a head row is gone when its last support is: one probe, a count reaching zero (`maintain`: a
//! head row that keeps a support is neither deleted nor derived again).
//!
//! ```text
//! h(x) :- p(x, y), q(y, _).      becomes    h__sup0(x, y, w0) :- p(x, y), q(y, w0).
//!                                           h(x) :- h__sup0(x, _, _).
//! ```
//!
//! A recursive stratum keeps its rules and is maintained by delete-and-rederive (`maintain`): counting over cycles
//! needs a witness of each row's level (decision 0090's levels spike), not built yet. A rule with an aggregate,
//! `nearest`, a matcher, or a variable no positive atom binds keeps its form; its stratum is maintained as before.

use super::lower;
use crate::code_rule_dsl::logic::front::{self, Atom, Literal, Rule, Term};
use std::collections::{BTreeMap, BTreeSet};

/// Whether a session maintains by counting: the default. `RERAIL_MAINTAIN=dred` asks for delete-and-rederive, to
/// compare with.
pub(in crate::code_rule_dsl) fn counting() -> bool {
    std::env::var("RERAIL_MAINTAIN").map_or(true, |v| v != "dred")
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
    let gated: BTreeSet<&str> = prog.gates.iter().map(|(g, _)| g.as_str()).collect();
    let mut rules: Vec<Rule> = Vec::new();
    let (mut through, mut kept) = (0, 0);
    // A stratum of one relation is recursive when a rule of it reads it: found in one pass over the rules (a pass
    // per stratum was quadratic, thousands of strata by tens of thousands of rules).
    let reads_itself: BTreeSet<&str> = prog
        .rules
        .iter()
        .filter(|r| r.body.iter().any(|l| l.atom().is_some_and(|a| a.rel == r.head.rel)))
        .map(|r| r.head.rel.as_str())
        .collect();
    let recursive: BTreeSet<&str> = sccs
        .iter()
        .filter(|scc| scc.len() > 1 || scc.iter().any(|r| reads_itself.contains(r.as_str())))
        .flatten()
        .map(String::as_str)
        .collect();
    let mut n: BTreeMap<String, usize> = BTreeMap::new();
    for rule in &prog.rules {
        let h = &rule.head.rel;
        let eligible = !recursive.contains(h.as_str()) && !gated.contains(h.as_str()) && !prog.natives.contains_key(h.as_str()) && !prog.lattice.contains_key(h.as_str()) && !prog.nullary.contains(h.as_str());
        match eligible.then(|| support(prog, rule, n.get(h.as_str()).copied().unwrap_or(0))).flatten() {
            Some((name, decl, made)) => {
                *n.entry(h.to_string()).or_default() += 1;
                let width = decl.len() - rule.head.args.len();
                let mut args = rule.head.args.clone();
                args.extend((0..width).map(|_| Term::Wild));
                out.program.decls.insert(name.clone().into(), decl.iter().map(|(c, t)| (c.into(), t.into())).collect());
                if let Some(d) = cross.domains.get(h.as_str()).cloned() {
                    out.domains.entry(name.clone()).or_insert(d);
                }
                rules.push(made);
                rules.push(Rule { head: rule.head.clone(), body: [Literal::Atom(Atom { rel: name.into(), args, neg: false, gated: false })].into() });
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
fn support(prog: &front::Program, rule: &Rule, i: usize) -> Option<(String, Vec<(front::Name, front::Name)>, Rule)> {
    let h = &rule.head.rel;
    let hdecl = prog.decls.get(h.as_str())?;
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
                let decl = prog.decls.get(a.rel.as_str())?;
                let mut args = Vec::new();
                for (c, t) in a.args.iter().enumerate() {
                    let t = match t {
                        Term::Wild => {
                            fresh += 1;
                            Term::Var(format!("rr_w{fresh}").into())
                        }
                        t => t.clone(),
                    };
                    if let Term::Var(v) = &t
                        && !types.contains_key(v.as_str())
                    {
                        types.insert(v.to_string(), decl.get(c)?.1.to_string());
                        order.push(v.to_string());
                    }
                    args.push(t);
                }
                body.push(Literal::Atom(Atom { args: args.into(), ..a.clone() }));
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
    let head_vars: BTreeSet<String> = rule.head.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(v.clone()) } else { None }).map(String::from).collect();
    let mut decl = hdecl.clone();
    let mut args = rule.head.args.clone();
    for v in order.iter().filter(|v| !head_vars.contains(*v)) {
        decl.push((v.clone().into(), types[v].clone().into()));
        args.push(Term::Var(v.clone().into()));
    }
    let name = format!("{h}__sup{i}");
    Some((name.clone(), decl.to_vec(), Rule { head: Atom { rel: name.into(), args, neg: false, gated: false }, body: body.into() }))
}
