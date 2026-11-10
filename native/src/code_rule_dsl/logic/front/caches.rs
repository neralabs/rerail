//! A cache is two rules (decision 0103): `.cache C by K` names a cache `C` and its key `K`.
//!
//! ```text
//! .cache ann_tree by ann_key
//! ann_key(s) :- struct__vann(_, _, s).                              the key rule: the key, from the input facts
//! ann_tree(s, t) :- ann_key(s), parse_of(s, "annotation", t).       the output rule: the result, from the key
//! ```
//!
//! The engine keeps `K` and `C` as it keeps every relation, and runs the output rule only for keys that are new: a
//! gone key's rows are removed from what was kept, a key in both runs keeps its rows, and nothing the output rule
//! reads besides `K` is looked at. In a session `maintain::rewrite` writes that as its rules (`K`'s delta is the
//! maintenance's own); a run without one reads the last run's rows as `K__kept` and `C__kept` (`kept_form`).
//!
//! The form is checked here: every rule of `C` reads `K` once, positively, with variables the head has, at the same
//! head places in every rule (`C`'s key columns); `C`'s stratum holds `C` alone.

use super::{Atom, Error, Literal, Program, Rule, Term};
use std::collections::BTreeSet;

/// The suffix of the input that holds a relation's rows as the last run of the program left them.
pub(in crate::code_rule_dsl) const KEPT: &str = "__kept";

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

/// `rel`'s rows as the last run of the program left them: `rel__kept`.
pub(in crate::code_rule_dsl) fn kept(rel: &str) -> String {
    format!("{rel}{KEPT}")
}

/// The relation whose last rows input `rel` holds (`rel` named `x__kept`), if it is one.
pub(in crate::code_rule_dsl) fn kept_of(rel: &str) -> Option<&str> {
    rel.strip_suffix(KEPT)
}

fn atom(rel: String, args: Vec<Term>, neg: bool) -> Literal {
    Literal::Atom(Atom { rel, args, neg, gated: false })
}

fn row(width: usize) -> Vec<Term> {
    (0..width).map(|i| Term::Var(format!("rr_c{i}"))).collect()
}

/// The program with each cache in the form a run without a session runs (decision 0103 §2): the last run's rows of
/// the key and of the cache are inputs `K__kept` and `C__kept` (empty when nothing was kept), and
///
/// ```text
/// K__plus(k…)  :- K(k…), !K__kept(k…).          K__minus(k…) :- K__kept(k…), !K(k…).
/// C__del(c…)   :- K__minus(k…), C__kept(c…).    C__ins(h…)   :- K__plus(k…), <the output rule's other literals>.
/// C(c…)        :- C__kept(c…), !C__del(c…).     C(c…)        :- C__ins(c…).
/// ```
///
/// The output rule runs for the new keys only; a gone key's rows are dropped from the kept rows. Gives each
/// relation made, with the relation it is made from (for the caller's domains).
pub(in crate::code_rule_dsl) fn kept_form(prog: &mut Program, caches: &[Cache]) -> Vec<(String, String)> {
    let mut made_from = Vec::new();
    for cache in caches {
        let (c, k) = (&cache.out, &cache.key);
        let cw = prog.decls[c].len();
        let kw = prog.decls[k].len();
        let named = |rel: &str, s: &str| format!("{rel}__{s}");
        let mut decl = |prog: &mut Program, from: &str, made: String| {
            made_from.push((from.to_owned(), made.clone()));
            let d = prog.decls[from].clone();
            prog.decls.insert(made.clone(), d);
            if prog.nullary.contains(from) {
                prog.nullary.insert(made);
            }
        };
        for (from, made) in [(k.as_str(), kept(k)), (c.as_str(), kept(c))] {
            decl(prog, from, made.clone());
            if !prog.inputs.contains(&made) {
                prog.inputs.push(made);
            }
        }
        for s in ["plus", "minus"] {
            decl(prog, k, named(k, s));
        }
        for s in ["del", "ins"] {
            decl(prog, c, named(c, s));
        }
        let (kv, cv) = (row(kw), row(cw));
        let key_of_c: Vec<Term> = cache.cols.iter().map(|&p| cv[p].clone()).collect();
        let mut rules: Vec<Rule> = Vec::new();
        let head = |rel: String, args: Vec<Term>| Atom { rel, args, neg: false, gated: false };
        rules.push(Rule { head: head(named(k, "plus"), kv.clone()), body: vec![atom(k.clone(), kv.clone(), false), atom(kept(k), kv.clone(), true)] });
        rules.push(Rule { head: head(named(k, "minus"), kv.clone()), body: vec![atom(kept(k), kv.clone(), false), atom(k.clone(), kv.clone(), true)] });
        rules.push(Rule { head: head(named(c, "del"), cv.clone()), body: vec![atom(named(k, "minus"), key_of_c, false), atom(kept(c), cv.clone(), false)] });
        for rule in prog.rules.iter().filter(|r| &r.head.rel == c) {
            let body = rule
                .body
                .iter()
                .map(|l| match l {
                    Literal::Atom(a) if &a.rel == k => atom(named(k, "plus"), a.args.clone(), false),
                    other => other.clone(),
                })
                .collect();
            rules.push(Rule { head: Atom { rel: named(c, "ins"), ..rule.head.clone() }, body });
        }
        rules.push(Rule { head: head(c.clone(), cv.clone()), body: vec![atom(kept(c), cv.clone(), false), atom(named(c, "del"), cv.clone(), true)] });
        rules.push(Rule { head: head(c.clone(), cv.clone()), body: vec![atom(named(c, "ins"), cv, false)] });
        prog.rules.retain(|r| &r.head.rel != c);
        prog.rules.extend(rules);
    }
    made_from
}

#[cfg(test)]
#[path = "caches_tests.rs"]
mod tests;
