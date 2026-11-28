//! The design's S1 ("functional chain"), its recognizer: a relation `r` functional in columns `K` (a key the layouts
//! hold, decision 0361), not recursive, every literal that reads it able to bind `K` before it (each of `K`'s terms a
//! constant, or a variable another positive atom of the reader binds): no reader scans `r`. Its readers are then its
//! call sites, each giving at most one row a key. Structure only: never a relation's name.

use super::super::super::program_image::{Lit, RelId, RuleRef, TermRef, Version};
use std::collections::BTreeSet;

/// A relation of S1's shape: `rel`, its key `key` (columns), and how many literals read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chain {
    pub rel: RelId,
    pub key: Vec<usize>,
    pub reads: usize,
}

/// The program's relations of S1's shape. `keys(rel)` gives the column sets the layouts hold as `rel`'s keys;
/// `recursive` holds the relations of a recursive stratum; `kept(rel)` tells a relation that must stay stored (an
/// output, `no_inline`, the engine's, a native operator's input).
pub fn chains(v: &Version, keys: &dyn Fn(RelId) -> Vec<Vec<usize>>, recursive: &BTreeSet<RelId>, kept: &dyn Fn(RelId) -> bool) -> Vec<Chain> {
    let heads: BTreeSet<RelId> = v.rules().map(|r| r.head().rel()).collect();
    let mut out = Vec::new();
    for rel in heads {
        if recursive.contains(&rel) || kept(rel) {
            continue;
        }
        let reads: Vec<(RuleRef<'_>, usize)> = v
            .rules()
            .flat_map(|r| r.body().enumerate().filter(move |(_, l)| matches!(l.view(), Lit::Atom(a) if a.rel() == rel)).map(move |(i, _)| (r, i)))
            .collect();
        if reads.is_empty() {
            continue;
        }
        // The smallest key every read can bind.
        let mut ks = keys(rel);
        ks.sort_by_key(Vec::len);
        if let Some(key) = ks.into_iter().find(|k| !k.is_empty() && reads.iter().all(|(r, i)| binds(*r, *i, k))) {
            out.push(Chain { rel, key, reads: reads.len() });
        }
    }
    out
}

/// Whether rule `r`'s literal `at` (a read) can have columns `key` bound before it: each a constant, or a variable
/// that another positive atom of the body binds.
pub fn binds(r: RuleRef<'_>, at: usize, key: &[usize]) -> bool {
    let lits: Vec<_> = r.body().collect();
    let Lit::Atom(read) = lits[at].view() else { return false };
    let args: Vec<TermRef<'_>> = read.args().collect();
    key.iter().all(|&c| match args.get(c) {
        Some(TermRef::Int(_) | TermRef::Str(_) | TermRef::Branded(_)) => true,
        Some(TermRef::Var(s)) => lits.iter().enumerate().any(|(j, l)| j != at && matches!(l.view(), Lit::Atom(a) if !a.neg() && a.args().any(|t| t.var() == Some(*s)))),
        _ => false,
    })
}

/// S1's relations in the placed version `p`, by name: `chains` with the layouts' keys (decision 0361), the strata's
/// recursion, and the program's outputs, `no_inline` and the engine's relations kept.
pub fn of(p: &super::super::body::Placed) -> Vec<(String, Chain)> {
    use super::super::super::program_image::{analyses, layouts::Layouts, manager};
    let Some(f) = super::super::fields::Fields::of(p) else { return Vec::new() };
    let v: &Version = &p.v;
    let Ok(layouts) = manager::analysis::<Layouts>(&p.v) else { return Vec::new() };
    let Ok(strata) = analyses::strata(v, f.about) else { return Vec::new() };
    let recursive: BTreeSet<RelId> = strata
        .sccs
        .iter()
        .filter(|s| s.len() > 1 || strata.edges.of_head(s[0]).any(|(d, _)| d == s[0]))
        .flat_map(|s| s.iter().copied())
        .collect();
    let keys = |rel: RelId| -> Vec<Vec<usize>> {
        layouts.of(rel).filter_map(|x| match x {
            crate::code_rule_dsl::logic::front::LayoutFact::Key(cols) => Some(cols.iter().copied().collect()),
            _ => None,
        }).collect()
    };
    // A relation a native operator reads (`Native::reads`: `dfs_first_memo`'s roots) is kept too: the operator reads
    // it by its name, so it is made where it stands, not in its readers (`fork`).
    let natives_read = f.natives_read();
    let kept = |rel: RelId| {
        let name = v.rel_name(rel);
        f.is_output(name) || f.qualified(name, "no_inline") || name.contains("__") || natives_read.contains(name)
    };
    chains(v, &keys, &recursive, &kept).into_iter().map(|c| (v.rel_name(c.rel).to_owned(), c)).collect()
}

/// `of(p)` logged as the profile's decision "functional chain (the design's S1)": each relation with its key and
/// its reads.
pub fn report(p: &super::super::body::Placed) {
    let found = of(p);
    super::super::super::profile::decision("functional chain (the design's S1)", || {
        let shown: Vec<String> = found.iter().map(|(name, c)| format!("{name}{:?} ({} reads)", c.key, c.reads)).collect();
        format!("{} relations every reader reads by a key: {}", found.len(), shown.join(", "))
    });
}

/// Whether S1's relations are made in their readers (`RERAIL_SHAPE_CHAIN=on`): each read is a call site, its rules
/// run there with the key bound.
pub fn on() -> bool {
    std::env::var("RERAIL_SHAPE_CHAIN").is_ok_and(|v| v == "on")
}
