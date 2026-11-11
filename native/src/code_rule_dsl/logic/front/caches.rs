//! A cache is two ordinary rules (decision 0103): a key rule, whose head carries a version tag, and an output rule
//! that reads the key.
//!
//! ```text
//! .cache ann_tree by ann_key
//! .decl ann_key(s: symbol, v: symbol)
//! ann_key(s, "v1") :- struct__vann(_, _, s).                              the key rule, version "v1"
//! .decl ann_tree(s: symbol, v: symbol, t: tape)
//! ann_tree(s, v, t) :- ann_key(s, v), parse_of(s, "annotation", t).       the output rule, by the key
//! ```
//!
//! Cached facts are just facts: `K` and `C` are evaluated, stored, held in memory, written back and maintained as
//! every relation is. Bumping the version ("v1" to "v2") changes every key row, and so every output row: the cache
//! is derived again. `.cache C by K` is a declaration for the lint here and for retention; no evaluation reads it
//! (`corpus::cache_architecture`).
//!
//! The lint: `C` and `K` are declared and differ; every rule of `K` in the program puts a constant in its head (the
//! version tag); every rule of `C` reads `K`, positively.

use super::{Error, Literal, Program, Term};
use std::collections::BTreeSet;

/// A cache `out` by its key `key`.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) struct Cache {
    pub out: String,
    pub key: String,
}

/// The program's caches, linted (decision 0103); an error names what the lint refuses.
pub fn caches(prog: &Program) -> Result<Vec<Cache>, Error> {
    let bad = |msg: String| Error::Syntax(format!(".cache: {msg} (decision 0103: a cache is a key rule with a version tag and an output rule that reads it)"));
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
        for rule in prog.rules.iter().filter(|r| r.head.rel == k) {
            if !rule.head.args.iter().any(|t| matches!(t, Term::Str(_) | Term::Int(_))) {
                return Err(bad(format!("the key rule `{rule}` has no version tag: put a constant in its head, as `{k}(x, \"v1\")`")));
            }
        }
        for rule in prog.rules.iter().filter(|r| r.head.rel == c) {
            if !rule.body.iter().any(|l| matches!(l, Literal::Atom(a) if a.rel == k && !a.neg)) {
                return Err(bad(format!("the output rule `{rule}` does not read {k}")));
            }
        }
        out.push(Cache { out: c, key: k });
    }
    Ok(out)
}

// cratify: `tests` is rerail-logic's (it mounts it)
