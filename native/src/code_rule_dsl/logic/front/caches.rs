//! A cache is two rules (decision 0103): `.cache C by K` names a cache `C` and its key `K`.
//!
//! ```text
//! .cache ann_tree by ann_key
//! ann_key(s) :- struct__vann(_, _, s).                              the key rule: the key, from the input facts
//! ann_tree(s, t) :- ann_key(s), parse_of(s, "annotation", t).       the output rule: the result, from the key
//! ```
//!
//! `K` and `C` are relations, kept as every relation is: stored, held in memory and written back the same way as any
//! other fact. The cache is only a rewrite of `C`'s rules into ordinary rules: the output rule runs for the keys that
//! are new, a gone key's rows go, and a key in both runs keeps its rows (a session's maintenance, `maintain::rewrite`).
//! Nothing else in the engine knows a relation is a cache (`corpus::cache_architecture`).
//!
//! The form is checked here: every rule of `C` reads `K` once, positively, with variables the head has, at the same
//! head places in every rule (`C`'s key columns); `C`'s stratum holds `C` alone.

use super::{Atom, Error, Literal, Program, Rule, Term};
use std::collections::BTreeSet;

/// A cache `out` by its key `key`: `out`'s column `cols[i]` is the key's column `i`.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) struct Cache {
    pub(in crate::code_rule_dsl) out: String,
    pub(in crate::code_rule_dsl) key: String,
    pub(in crate::code_rule_dsl) cols: Vec<usize>,
}

/// The program's caches, checked (decision 0103 §1); an error names what breaks the form.
pub(in crate::code_rule_dsl) fn caches(prog: &Program) -> Result<Vec<Cache>, Error> {
    let bad = |msg: String| Error::Syntax(format!(".cache: {msg} (decision 0103: a cache is a key rule and an output rule)"));
    let mut out = Vec::new();
    let mut named = BTreeSet::new();
    for (_, text) in prog.settings.iter().filter(|(k, _)| k == "cache") {
        let words: Vec<&str> = text.split_whitespace().collect();
        let [c, "by", k] = words.as_slice() else { return Err(bad(format!("`.cache {text}`: write `.cache C by K`"))) };
        let (c, k) = (c.trim_end_matches('.').to_owned(), k.trim_end_matches('.').to_owned());
        if !named.insert(c.clone()) {
            return Err(bad(format!("{c} is named twice")));
        }
        if c == k {
            return Err(bad(format!("{c} is its own key")));
        }
        for rel in [&c, &k] {
            if !prog.decls.contains_key(rel) {
                return Err(bad(format!("{rel} is not declared")));
            }
        }
        if prog.inputs.contains(&c) || prog.lattice.contains_key(&c) || prog.natives.contains_key(&c) {
            return Err(bad(format!("{c} is an input, a lattice or made natively: its rows must be its output rules'")));
        }
        if !prog.rules.iter().any(|r| r.head.rel == k) {
            return Err(bad(format!("{k} has no key rule: derive it from the input facts")));
        }
        let rules: Vec<&Rule> = prog.rules.iter().filter(|r| r.head.rel == c).collect();
        if rules.is_empty() {
            return Err(bad(format!("{c} has no output rule")));
        }
        let mut cols: Option<Vec<usize>> = None;
        for rule in &rules {
            let keys: Vec<&Atom> = rule.body.iter().filter_map(|l| if let Literal::Atom(a) = l { (a.rel == k).then_some(a) } else { None }).collect();
            let [key] = keys.as_slice() else { return Err(bad(format!("the rule `{rule}` reads {k} {} times, not once", keys.len()))) };
            if key.neg {
                return Err(bad(format!("the rule `{rule}` reads {k} negated")));
            }
            if rule.body.iter().any(|l| matches!(l, Literal::Agg { atom, .. } if atom.rel == k || atom.rel == c)) {
                return Err(bad(format!("the rule `{rule}` aggregates over {k} or {c}")));
            }
            let mut these = Vec::new();
            for t in &key.args {
                let Term::Var(v) = t else { return Err(bad(format!("the rule `{rule}`: {k}'s arguments must be variables")) ) };
                let Some(p) = rule.head.args.iter().position(|h| matches!(h, Term::Var(x) if x == v)) else {
                    return Err(bad(format!("the rule `{rule}`: the head does not have the key's {v}")));
                };
                these.push(p);
            }
            match &cols {
                Some(c0) if *c0 != these => return Err(bad(format!("the rule `{rule}` puts the key in other columns of {c} than its first rule")) ),
                Some(_) => {}
                None => cols = Some(these),
            }
        }
        let (sccs, _) = super::strata(prog)?;
        if let Some(scc) = sccs.iter().find(|s| s.contains(&c))
            && scc.len() > 1
        {
            return Err(bad(format!("{c} is in a stratum with {}: a cache's stratum holds it alone", scc.iter().filter(|r| **r != c).cloned().collect::<Vec<_>>().join(", "))));
        }
        if rules.iter().any(|r| r.body.iter().any(|l| l.atom().is_some_and(|a| a.rel == c))) {
            return Err(bad(format!("{c} reads itself")));
        }
        out.push(Cache { out: c, key: k, cols: cols.unwrap_or_default() });
    }
    Ok(out)
}

/// The caches of a part of a program (the file-local or the cross-file part of a split, `typing::file_local`): those
/// whose output rules are in it, checked.
pub(in crate::code_rule_dsl) fn caches_here(prog: &Program) -> Result<Vec<Cache>, Error> {
    let here = |text: &str| text.split_whitespace().next().is_some_and(|c| prog.rules.iter().any(|r| r.head.rel == c.trim_end_matches('.')));
    if !prog.settings.iter().any(|(k, v)| k == "cache" && here(v)) {
        return Ok(Vec::new());
    }
    let mut part = prog.clone();
    part.settings.retain(|(k, v)| k != "cache" || here(v));
    caches(&part)
}

/// Marks `prog`'s caches as rewritten into ordinary rules (a session's maintenance): their form is not checked again.
pub(in crate::code_rule_dsl) fn mark(prog: &mut Program, _caches: &[Cache]) {
    prog.settings.retain(|(k, _)| k != "cache");
}

#[cfg(test)]
#[path = "caches_tests.rs"]
mod tests;
