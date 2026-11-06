//! Demand for negated relations (decision 0046): a relation computed for every node that rules only negate is
//! computed for the keys those rules reach, not for every node.
//!
//! An imperfect conversion leaves relations like
//!
//! ```text
//! blocked($low, $high) :- nearest_k11($low, $s), inside_k2($s, $high).
//! reader(…) :- <body binding $low, $high>, !blocked($low, $high).
//! ```
//!
//! where `blocked` holds a row for every node (1.8M rows on 1000 files) and the reader asks about a few. Each reader
//! is split into its body less the negations it demands, and the negations:
//!
//! ```text
//! pre_<i>(vars) :- <the reader's body without its negations of demanded relations>.
//! demand_blocked($low, $high) :- pre_<i>(vars).
//! blocked($low, $high) :- demand_blocked($low, $high), nearest_k11($low, $s), inside_k2($s, $high).
//! reader(…) :- pre_<i>(vars), !blocked($low, $high).
//! ```
//!
//! Still a keyed anti-join, run set at a time (the spike: a per-row probe re-runs the body per outer row, S203/S064;
//! a keyed anti-join beats a subplan rescan, S254/S272). The guard is structural, not statistics (history index
//! §4): `pre_<i>` is the reader's own work less some anti-joins, planned freely in every chunk, so the demand never
//! exceeds what the reader binds. No plan is frozen into the split (a plan taken from one chunk ordered the body
//! badly in others: 320M rows), and a relation negated twice in one rule needs no copy, since every demand comes from
//! the one `pre_<i>`.
//!
//! A relation is demanded when it is derived and not kept, not recursive, every rule of it starts from every node
//! (a `nearest_<set>`, `inside_<set>` or kind-free `node` atom on its key), it is read, and every read of it is a
//! negation with variable or constant arguments. Corpus runs only, so the golden plans are unchanged.

use super::super::front::{Atom, Literal, Program, Rule, Term};
use std::collections::{BTreeMap, BTreeSet};

/// `prog` with each demandable relation computed for the keys its readers reach (the module's split), and what was
/// demanded; `None` where nothing is, or where the split would not stratify or type.
pub(super) fn split_negations(prog: &Program) -> Option<(Program, Vec<String>)> {
    let kept = |rel: &str| {
        prog.outputs.iter().any(|o| o == rel)
            || prog.inputs.iter().any(|i| i == rel)
            || prog.lattice.contains_key(rel)
            || prog.facts.get(rel).is_some_and(|f| !f.is_empty())
            || prog.demand.iter().any(|d| d == rel)
            || prog.gates.iter().any(|(g, _)| g == rel)
    };
    let mut by_rel: BTreeMap<&str, Vec<&Rule>> = BTreeMap::new();
    for rule in &prog.rules {
        by_rel.entry(rule.head.rel.as_str()).or_default().push(rule);
    }
    let per_node = |rule: &Rule| -> bool {
        let Some(Term::Var(key)) = rule.head.args.first() else { return false };
        let on_key = |a: &Atom, c: usize| a.args.get(c) == Some(&Term::Var(key.clone()));
        rule.body.iter().any(|l| match l {
            Literal::Atom(a) if !a.neg => {
                ((a.rel.starts_with("nearest_") || a.rel.starts_with(super::super::front::AXIS)) && on_key(a, 0))
                    || (a.rel == "node" && on_key(a, 1) && a.args.get(2) == Some(&Term::Wild))
            }
            Literal::Nearest { n, .. } => n == key,
            _ => false,
        })
    };
    let recursive = |rel: &str, rules: &[&Rule]| rules.iter().any(|r| r.body.iter().any(|l| matches!(l, Literal::Atom(a) | Literal::Agg { atom: a, .. } if a.rel == rel)));
    let mut candidates: BTreeSet<&str> = by_rel
        .iter()
        .filter(|(rel, rules)| !kept(rel) && !recursive(rel, rules) && rules.iter().all(|r| per_node(r) && !reads_demand(r)))
        .map(|(rel, _)| *rel)
        .collect();
    // Read only by negations whose arguments are variables or constants.
    for rule in &prog.rules {
        for l in &rule.body {
            match l {
                Literal::Atom(a) if a.neg && a.args.iter().all(|t| matches!(t, Term::Var(_) | Term::Int(_) | Term::Str(_))) => {}
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => {
                    candidates.remove(a.rel.as_str());
                }
                _ => {}
            }
        }
    }
    // Negated somewhere: a relation nothing reads stays as it is.
    let read: BTreeSet<&str> = prog.rules.iter().flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.as_str())).collect();
    candidates.retain(|r| read.contains(r));
    if candidates.is_empty() {
        return None;
    }
    // A candidate whose split fails (a variable of no known type, or a cycle through a negation) is left as it was,
    // and the rest are split again: one rule must not keep every other from its demand (in a combined program, one
    // component closing a cycle left all 100 rules without one).
    loop {
        match split_with(prog, &candidates) {
            Ok(split) => return Some(split),
            Err(failed) => {
                let before = candidates.len();
                candidates.retain(|c| !failed.contains(*c));
                if candidates.is_empty() || candidates.len() == before {
                    return None;
                }
            }
        }
    }
}

/// `prog` split for `candidates`, or the candidates whose split failed.
fn split_with(prog: &Program, candidates: &BTreeSet<&str>) -> Result<(Program, Vec<String>), BTreeSet<String>> {
    // Each reader: its negations of candidates, whose variables the rest of its body binds (a safe rule's do).
    let negated = |rule: &Rule, i: usize| matches!(&rule.body[i], Literal::Atom(a) if a.neg && candidates.contains(a.rel.as_str()));
    let readers: Vec<usize> = (0..prog.rules.len()).filter(|&ri| (0..prog.rules[ri].body.len()).any(|i| negated(&prog.rules[ri], i))).collect();
    let mut out = prog.clone();
    let mut rules: Vec<Rule> = Vec::new();
    let mut demand_rules: Vec<Rule> = Vec::new();
    for (ri, rule) in prog.rules.iter().enumerate() {
        if !readers.contains(&ri) {
            rules.push(rule.clone());
            continue;
        }
        let (mut cut, mut kept): (Vec<&Literal>, Vec<&Literal>) = (Vec::new(), Vec::new());
        for (i, l) in rule.body.iter().enumerate() {
            if negated(rule, i) { cut.push(l) } else { kept.push(l) }
        }
        let mut bound = BTreeSet::new();
        for l in &kept {
            bound.extend(l.vars());
        }
        let mut later = rule.head.vars();
        for l in &cut {
            later.extend(l.vars());
        }
        let vars: Vec<String> = bound.intersection(&later).cloned().collect();
        let Some(types) = vars.iter().map(|v| var_type(prog, &kept, v)).collect::<Option<Vec<String>>>() else {
            // A variable of no known type (bound by a comparison alone): nothing is split.
            super::super::profile::decision("demand (decision 0046)", || format!("not demanded: a variable of a rule for {} has no known type", rule.head.rel));
            return Err(cut.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone()).collect());
        };
        let pre = format!("pre_{ri}");
        out.decls.insert(pre.clone(), vars.iter().zip(&types).map(|(v, t)| (v.clone(), t.clone())).collect());
        let pre_atom = Atom { rel: pre, args: vars.iter().map(|v| Term::Var(v.clone())).collect(), neg: false, gated: false };
        rules.push(Rule { head: pre_atom.clone(), body: kept.into_iter().cloned().collect() });
        let mut body = vec![Literal::Atom(pre_atom.clone())];
        for l in cut {
            let Literal::Atom(neg) = l else { unreachable!("a negated atom") };
            demand_rules.push(Rule {
                head: Atom { rel: format!("demand_{}", neg.rel), args: neg.args.clone(), neg: false, gated: false },
                body: vec![Literal::Atom(pre_atom.clone())],
            });
            body.push(l.clone());
        }
        rules.push(Rule { head: rule.head.clone(), body });
    }
    let demanded: Vec<String> = candidates.iter().map(|r| (*r).to_owned()).collect();
    for rel in &demanded {
        let name = format!("demand_{rel}");
        out.decls.insert(name.clone(), prog.decls[rel].clone());
        for rule in rules.iter_mut().filter(|r| &r.head.rel == rel) {
            rule.body.insert(0, Literal::Atom(Atom { rel: name.clone(), args: rule.head.args.clone(), neg: false, gated: false }));
        }
    }
    rules.extend(demand_rules);
    out.rules = rules;
    // A split that closes a cycle through a negation (a reader's body reading what the negated relation depends on)
    // leaves the program as it was.
    match super::super::front::strata(&out) {
        Ok(_) => Ok((out, demanded)),
        Err(e) => {
            // The candidates in the failing component (a `demand_<c>` there names `c`).
            let failed: BTreeSet<String> = match &e {
                super::super::front::Error::NotStratifiable { component, .. } => component
                    .iter()
                    .map(|r| r.strip_prefix("demand_").unwrap_or(r))
                    .filter(|r| candidates.contains(r))
                    .map(str::to_owned)
                    .collect(),
                _ => BTreeSet::new(),
            };
            super::super::profile::decision("demand (decision 0046)", || format!("not demanded: {} (their split closes a cycle through a negation)", failed.iter().cloned().collect::<Vec<_>>().join(", ")));
            Err(failed)
        }
    }
}

/// Whether `rule` already reads a demand (`demand_<r>`, made before the run).
fn reads_demand(rule: &Rule) -> bool {
    rule.body.iter().any(|l| matches!(l, Literal::Atom(a) if a.rel.starts_with("demand_")))
}

/// A variable's type: its column's in the first positive atom of `body` that names it.
fn var_type(prog: &Program, body: &[&Literal], var: &str) -> Option<String> {
    body.iter().find_map(|l| match l {
        Literal::Atom(a) if !a.neg => {
            let c = a.args.iter().position(|t| matches!(t, Term::Var(v) if v == var))?;
            prog.decls.get(&a.rel)?.get(c).map(|(_, t)| t.clone())
        }
        Literal::Nearest { n, a, .. } if n == var || a == var => Some("node".to_owned()),
        _ => None,
    })
}
