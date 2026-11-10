//! A cache is two rules (decision 0103): `.cache C by K` names a cache `C` and its key `K`.
//!
//! ```text
//! .cache ann_tree by ann_key
//! ann_key(s) :- struct__vann(_, _, s).                              the key rule: the key, from the input facts
//! ann_tree(s, t) :- ann_key(s), parse_of(s, "annotation", t).       the output rule: the result, from the key
//! ```
//!
//! The engine keeps `K` and `C` as it keeps every relation, and runs the output rule only for keys that are new: a
//! gone key's rows are not among the run's, a key in both runs keeps its rows, and nothing the output rule reads
//! besides `K` is looked at. A run reads the store's rows of its keys (`kept_form`; `content_store::cache_rows`); a
//! session maintains a cache by its key's delta (`maintain::rewrite`) and writes its new keys to the store too.
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
    /// What names the cache's rows in the store: a digest of the engine's build, both relations' names and columns,
    /// and the output rules. Another program's cache of the same rules shares them.
    pub(in crate::code_rule_dsl) id: [u8; 32],
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
        let id = {
            use sha2::Digest as _;
            let mut h = sha2::Sha256::new();
            h.update(b"rerail.cache/v1\0");
            h.update(env!("RERAIL_ENGINE_BUILD_ID").as_bytes());
            for rel in [&c, &k] {
                h.update(format!("\0{rel}{:?}", prog.decls[rel]).as_bytes());
            }
            for rule in &rules {
                h.update(format!("\0{rule}").as_bytes());
            }
            h.finalize().into()
        };
        out.push(Cache { out: c, key: k, cols: cols.unwrap_or_default(), id });
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

/// `rel`'s kept rows, as the store has them for this run's keys: `rel__kept`.
pub(in crate::code_rule_dsl) fn kept(rel: &str) -> String {
    format!("{rel}{KEPT}")
}

/// The relation whose kept rows input `rel` holds (`rel` named `x__kept`), if it is one.
pub(in crate::code_rule_dsl) fn kept_of(rel: &str) -> Option<&str> {
    rel.strip_suffix(KEPT)
}

/// `rel__suffix`, a relation a cache's run makes.
pub(in crate::code_rule_dsl) fn named(rel: &str, suffix: &str) -> String {
    format!("{rel}__{suffix}")
}

/// The setting a run of a cache carries (`running`): the program's rules for the cache were rewritten (`kept_form`, or
/// a session's maintenance), so the form is no longer checked, but the run still knows the cache.
const RUNNING: &str = "cache_run";

/// The caches whose rules a run carries rewritten, by the setting `mark` left.
pub(in crate::code_rule_dsl) fn running(prog: &Program) -> Vec<Cache> {
    prog.settings
        .iter()
        .filter(|(k, _)| k == RUNNING)
        .filter_map(|(_, v)| {
            let w: Vec<&str> = v.split_whitespace().collect();
            let [out, key, id, cols] = w.as_slice() else { return None };
            let mut digest = [0u8; 32];
            for (i, b) in digest.iter_mut().enumerate() {
                *b = u8::from_str_radix(id.get(2 * i..2 * i + 2)?, 16).ok()?;
            }
            let cols = cols.split(',').filter(|c| !c.is_empty()).map(|c| c.parse().ok()).collect::<Option<Vec<usize>>>()?;
            Some(Cache { out: (*out).to_owned(), key: (*key).to_owned(), cols, id: digest })
        })
        .collect()
}

/// Marks `prog` as running `caches` (`running`).
pub(in crate::code_rule_dsl) fn mark(prog: &mut Program, caches: &[Cache]) {
    for c in caches {
        let id: String = c.id.iter().map(|b| format!("{b:02x}")).collect();
        let cols = c.cols.iter().map(usize::to_string).collect::<Vec<_>>().join(",");
        prog.settings.push((RUNNING.to_owned(), format!("{} {} {id} {cols}", c.out, c.key)));
    }
    prog.settings.retain(|(k, _)| k != "cache");
}

fn atom(rel: String, args: Vec<Term>, neg: bool) -> Literal {
    Literal::Atom(Atom { rel, args, neg, gated: false })
}

fn row(width: usize) -> Vec<Term> {
    (0..width).map(|i| Term::Var(format!("rr_c{i}"))).collect()
}

/// The program with each cache in the form a run reads from the store (decision 0103 §2). The run binds `K__kept`
/// and `C__kept` when it reaches them: the store's keys among this run's `K`, and their rows of `C`. Then
///
/// ```text
/// K__plus(k…) :- K(k…), !K__kept(k…).                    the keys the store has not seen
/// C__ins(h…)  :- K__plus(k…), <the output rule's other literals>.
/// C(c…)       :- C__kept(c…).                           the store's rows of this run's keys
/// C(c…)       :- C__ins(c…).
/// ```
///
/// The output rule runs for the new keys only, and the run writes their rows to the store. A key gone from `K` is
/// not looked up, so `C` has no rows of it. Gives each relation made with the relation it is made from (for the
/// caller's domains).
pub(in crate::code_rule_dsl) fn kept_form(prog: &mut Program, caches: &[Cache]) -> Vec<(String, String)> {
    let mut made_from = Vec::new();
    for cache in caches {
        let (c, k) = (&cache.out, &cache.key);
        let cw = prog.decls[c].len();
        let kw = prog.decls[k].len();
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
        decl(prog, k, named(k, "plus"));
        decl(prog, c, named(c, "ins"));
        let (kv, cv) = (row(kw), row(cw));
        let head = |rel: String, args: Vec<Term>| Atom { rel, args, neg: false, gated: false };
        let mut rules: Vec<Rule> = vec![Rule { head: head(named(k, "plus"), kv.clone()), body: vec![atom(k.clone(), kv.clone(), false), atom(kept(k), kv, true)] }];
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
        rules.push(Rule { head: head(c.clone(), cv.clone()), body: vec![atom(kept(c), cv.clone(), false)] });
        rules.push(Rule { head: head(c.clone(), cv.clone()), body: vec![atom(named(c, "ins"), cv, false)] });
        prog.rules.retain(|r| &r.head.rel != c);
        prog.rules.extend(rules);
    }
    mark(prog, caches);
    made_from
}

#[cfg(test)]
#[path = "caches_tests.rs"]
mod tests;
