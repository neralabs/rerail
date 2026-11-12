//! Bounded cross-file rounds (03 §13: "R1..Rk run across files over interface summaries from the previous round,
//! with an early cutoff on interface digests"; k = 2, I11; 05 Phase 10 row 2; list step 29).
//!
//! The cross-file part of a program (`file_local::programs`) is not run as one global fixpoint. Each of its
//! recursive atoms that may read another document's rows is made a *hop*:
//!
//! ```text
//! // calls.rdl: a class's attribute is found on its base, which may be defined in another file.
//! found($c, $a, $x) :- base($c, $b), !bound_in($c, $a), found($b, $a, $x).
//! // becomes two rules. Within the document, the fixpoint is exact:
//! found($c, $a, $x) :- base($c, $b), !bound_in($c, $a), found($b, $a, $x), @doc($b) = @doc($c).
//! // Across documents, the rule reads the previous round's rows:
//! found($c, $a, $x) :- base($c, $b), !bound_in($c, $a), found__prev($b, $a, $x), @doc($b) != @doc($c).
//! ```
//!
//! A recursive atom is a hop when its first node column's variable is not the head's: the two nodes may lie in
//! different documents (`@doc` is a node's document, `d` of `d << 32 | index`). A rule with several hops gets one
//! rule per choice of same or other document for each hop.
//!
//! Round 0 reads no previous rows: it is the cross-file part with no hop crossing a file. Round r reads round
//! r − 1's rows of the hop relations (`__prev`). The rounds stop when the hop relations' digest is unchanged (the
//! fixpoint is reached: the early cutoff), or after round `K`. Rows a round past `K` would still add are not
//! derived: the result is the fixpoint's under-approximation by at most `K` file crossings per chain, and the run
//! reports it unconverged (`Rounds::converged`), the state 03 §13 keeps apart from ⊤.
//!
//! A recursive atom whose node is joined to the head's through same-document atoms (`look($n, $x, $s)`, a file-local
//! relation, ties `$s` to `$n`) is not a hop: the two nodes are of one document, so the across rule would be empty.
//!
//! A recursive atom whose node cannot be tied to the head (no node column on either side) is left as it is and
//! named in `Bounded::unbounded`: within a round it is a fixpoint, as before.

use crate::code_rule_dsl::logic::front::{self, Atom, Literal, Program, Rule, Term};
use std::collections::{BTreeMap, BTreeSet};

/// The number of rounds after round 0 the typing programs declare (I11 adopted k = 2; 4 since 2026-11-06, decision
/// 0047: the CPython runtime oracle's package re-export needs a third file crossing, and its rounds converge after
/// round 4). Since decision 0056 a program states it (`PRAGMA`); this is the value they state.
pub(crate) const K: usize = 4;

/// A program's flag for the pending round (O41, decision 0047): a program that declares the fact `unconverged(0)`
/// gets one more round after round `K` when the rounds have not converged, with `unconverged(1)` added. Its rules
/// that read `unconverged(1)` make unknown what a missing row would otherwise decide.
pub(crate) const UNCONVERGED: &str = "unconverged";

pub(crate) use crate::code_rule_dsl::logic::front::PRAGMA;

/// The rounds `prog` declares (`PRAGMA`), if it declares them.

pub(crate) fn declared(prog: &Program) -> Option<usize> {
    prog.settings.iter().filter(|(key, _)| key == "pragma").find_map(|(_, text)| {
        let words: Vec<&str> = text.split('"').map(str::trim).filter(|w| !w.is_empty()).collect();
        match words.as_slice() {
            [key, k] if *key == PRAGMA => k.parse().ok(),
            _ => None,
        }
    })
}

/// The rounds bound of a program that declares `declared` rounds: that, or `RERAIL_CROSS_ROUNDS` (a diagnostic: how
/// many rounds a corpus needs), or a test's `with_k` on this thread.
pub(crate) fn k(declared: usize) -> usize {
    #[cfg(test)]
    if let Some(k) = TEST_K.with(std::cell::Cell::get) {
        return k;
    }
    std::env::var("RERAIL_CROSS_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(declared)
}

#[cfg(test)]
thread_local! {
    static TEST_K: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Runs `f` with the rounds bound `k` on this thread (tests: the environment variable would reach every test).
#[cfg(test)]
pub(crate) fn with_k<T>(k: usize, f: impl FnOnce() -> T) -> T {
    TEST_K.with(|c| c.set(Some(k)));
    let out = f();
    TEST_K.with(|c| c.set(None));
    out
}

/// The suffix of a hop relation's previous-round input.
pub(crate) const PREV: &str = "__prev";

/// More hops than this in one rule are left unbounded (2^n rules otherwise).
const MAX_HOPS: usize = 3;

pub(crate) struct Bounded {
    pub(crate) program: Program,
    /// The relations hops read: each round's rows of these are the next round's `<rel>__prev`.
    pub(crate) hops: BTreeSet<String>,
    /// The rules split (index in the original program).
    pub(crate) split: Vec<usize>,
    /// Rules with a recursive atom that cannot be tied to the head, by head relation.
    pub(crate) unbounded: BTreeSet<String>,
}

fn first_node_var(prog: &Program, a: &Atom) -> Option<String> {
    let cols = prog.decls.get(a.rel.as_str())?;
    a.args.iter().zip(cols).find_map(|(t, (_, ty))| match t {
        Term::Var(v) if ty == "node" => Some(v.clone()),
        _ => None,
    })
}

fn doc(v: &str) -> Term {
    Term::Call("doc".to_owned(), vec![Term::Var(v.to_owned())])
}

/// The variables of `body` that its positive atoms of `same_doc` relations join to `v` through their same-document
/// columns (`file_local::Programs::same_doc`): all of `v`'s document.
pub(crate) fn same_document(body: &[Literal], same_doc: &BTreeMap<String, Vec<usize>>, v: &str) -> BTreeSet<String> {
    let groups: Vec<BTreeSet<&str>> = body
        .iter()
        .filter_map(|l| match l {
            Literal::Atom(a) if !a.neg => Some(a),
            _ => None,
        })
        .filter_map(|a| {
            let cols = same_doc.get(a.rel.as_str())?;
            Some(cols.iter().filter_map(|&c| if let Some(Term::Var(x)) = a.args.get(c) { Some(x.as_str()) } else { None }).collect())
        })
        .collect();
    let mut reached: BTreeSet<&str> = BTreeSet::from([v]);
    loop {
        let before = reached.len();
        for g in &groups {
            if !g.is_disjoint(&reached) {
                reached.extend(g);
            }
        }
        if reached.len() == before {
            return reached.into_iter().map(str::to_owned).collect();
        }
    }
}

/// The cross-file part `prog` with its hops reading the previous round. `same_doc` holds the same-document columns of
/// the relations `prog` reads that cross no file (`file_local::Programs::same_doc`).
pub(crate) fn bound(prog: &Program, same_doc: &BTreeMap<String, Vec<usize>>) -> Result<Bounded, front::Error> {
    let (sccs, _) = front::strata(prog)?;
    let comp: BTreeMap<&str, usize> = sccs.iter().enumerate().flat_map(|(i, s)| s.iter().map(move |r| (r.as_str(), i))).collect();
    let mut out = prog.clone();
    out.rules.clear();
    let (mut hops, mut split, mut unbounded) = (BTreeSet::new(), Vec::new(), BTreeSet::new());
    for (ri, rule) in prog.rules.iter().enumerate() {
        let Some(&hc) = comp.get(rule.head.rel.as_str()) else {
            out.rules.push(rule.clone());
            continue;
        };
        let recursive: Vec<usize> =
            rule.body.iter().enumerate().filter(|(_, l)| l.atom().is_some_and(|a| comp.get(a.rel.as_str()) == Some(&hc))).map(|(i, _)| i).collect();
        let head_node = first_node_var(prog, &rule.head);
        let local_to_head = head_node.as_deref().map(|h| same_document(&rule.body, same_doc, h)).unwrap_or_default();
        let mut found = Vec::new();
        for &i in &recursive {
            let a = rule.body[i].atom().expect("recursive literals are atoms");
            match (&head_node, first_node_var(prog, a)) {
                (Some(h), Some(v)) if *h == v || local_to_head.contains(&v) => {}
                (Some(h), Some(v)) if matches!(rule.body[i], Literal::Atom(_)) => found.push((i, h.clone(), v)),
                _ => {
                    unbounded.insert(rule.head.rel.to_string());
                }
            }
        }
        if found.is_empty() || found.len() > MAX_HOPS {
            if !found.is_empty() {
                unbounded.insert(rule.head.rel.to_string());
            }
            out.rules.push(rule.clone());
            continue;
        }
        split.push(ri);
        for choice in 0..(1u32 << found.len()) {
            let mut body = rule.body.clone();
            for (bit, (i, h, v)) in found.iter().enumerate() {
                let across = choice & (1 << bit) != 0;
                if across {
                    let Literal::Atom(a) = &mut body[*i] else { unreachable!() };
                    hops.insert(a.rel.to_string());
                    a.rel = format!("{}{PREV}", a.rel).into();
                }
                body.push(Literal::Cmp { l: doc(v), op: if across { "!=" } else { "=" }, r: doc(h) });
            }
            out.rules.push(Rule { head: rule.head.clone(), body });
        }
    }
    for rel in &hops {
        let prev = format!("{rel}{PREV}");
        out.decls.insert(prev.clone(), prog.decls[rel.as_str()].clone());
        if let Some(l) = prog.lattice.get(rel.as_str()) {
            out.lattice.insert(prev.clone(), l.clone());
        }
        out.inputs.push(prev);
    }
    Ok(Bounded { program: out, hops, split, unbounded })
}

/// The suffix of a cone relation's final rows, read by the rest of the part (`cone_split`).
pub(crate) const DONE: &str = "__done";

/// A bounded program split at its hops' cone: `cone` derives the hop relations and every relation they read, and
/// runs in rounds; `rest` derives the others, once, after the last round, reading the cone's rows of `reads` as
/// inputs named `<rel>__done`. `None` when the cone is the whole program.
pub(crate) struct Split {
    pub(crate) cone: Program,
    pub(crate) rest: Program,
    pub(crate) reads: BTreeSet<String>,
}

pub(crate) fn cone_split(prog: &Program, hops: &BTreeSet<String>) -> Option<Split> {
    let derived: BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
    let mut cone: BTreeSet<&str> = hops.iter().map(String::as_str).filter(|r| derived.contains(r)).collect();
    let mut work: Vec<&str> = cone.iter().copied().collect();
    while let Some(rel) = work.pop() {
        for r in prog.rules.iter().filter(|r| r.head.rel == rel) {
            for a in r.body.iter().filter_map(Literal::atom) {
                if derived.contains(a.rel.as_str()) && cone.insert(a.rel.as_str()) {
                    work.push(a.rel.as_str());
                }
            }
        }
    }
    let in_cone = |rel: &str| cone.contains(rel);
    // Gates (whose sources and quiet sets may lie on both sides) are not split, nor is the whole program.
    if !prog.gates.is_empty() || prog.rules.iter().all(|r| in_cone(&r.head.rel)) {
        return None;
    }
    let reads: BTreeSet<String> = prog
        .rules
        .iter()
        .filter(|r| !in_cone(&r.head.rel))
        .flat_map(|r| r.body.iter().filter_map(Literal::atom))
        .filter(|a| in_cone(&a.rel))
        .map(|a| a.rel.to_string())
        .collect();
    // A lattice relation's rows are its cells, not a set: the rest does not read one as an input.
    if reads.iter().any(|r| prog.lattice.contains_key(r)) {
        return None;
    }
    let mut c = prog.clone();
    c.rules.retain(|r| in_cone(&r.head.rel));
    c.outputs.retain(|o| in_cone(o));
    c.lattice.retain(|rel, _| in_cone(rel));
    c.gates.retain(|(rel, _)| in_cone(rel));
    let mut rest = prog.clone();
    rest.rules.retain(|r| !in_cone(&r.head.rel));
    for r in &mut rest.rules {
        for l in &mut r.body {
            if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = l
                && reads.contains(a.rel.as_str())
            {
                a.rel = format!("{}{DONE}", a.rel).into();
            }
        }
    }
    rest.outputs.retain(|o| !in_cone(o));
    rest.lattice.retain(|rel, _| !in_cone(rel));
    rest.gates.retain(|(rel, _)| !in_cone(rel));
    for rel in &reads {
        rest.decls.insert(format!("{rel}{DONE}"), prog.decls[rel].clone());
        rest.inputs.push(format!("{rel}{DONE}"));
    }
    Some(Split { cone: c, rest, reads })
}

/// The suffix of a relation the rounds read and no round changes (`pre_split`), derived once before them.
pub(crate) const PRE: &str = "__pre";

/// A cone split before its rounds: `pre` derives each relation of the cone that reads no `__prev` row and no
/// `unconverged` fact, directly or through other relations, so every round would derive the same rows of it (a walk
/// over the tree, an import's binding). It runs once; `rounds` reads its rows of `reads` as inputs `<rel>__pre`.
/// `None` when every relation changes with the rounds, or the cone has gates or a lattice relation `rounds` reads.
pub(crate) struct PreSplit {
    pub(crate) pre: Program,
    pub(crate) rounds: Program,
    pub(crate) reads: BTreeSet<String>,
}

pub(crate) fn pre_split(cone: &Program) -> Option<PreSplit> {
    if !cone.gates.is_empty() {
        return None;
    }
    let changes_name = |r: &str| r.ends_with(PREV) || r == UNCONVERGED;
    let mut changing: BTreeSet<&str> = BTreeSet::new();
    loop {
        let before = changing.len();
        for r in &cone.rules {
            if !changing.contains(r.head.rel.as_str()) && r.body.iter().filter_map(Literal::atom).any(|a| changes_name(&a.rel) || changing.contains(a.rel.as_str())) {
                changing.insert(r.head.rel.as_str());
            }
        }
        if changing.len() == before {
            break;
        }
    }
    let in_pre = |rel: &str| !changing.contains(rel);
    if cone.rules.iter().all(|r| !in_pre(&r.head.rel)) {
        return None;
    }
    let reads: BTreeSet<String> = cone
        .rules
        .iter()
        .filter(|r| !in_pre(&r.head.rel))
        .flat_map(|r| r.body.iter().filter_map(Literal::atom))
        .filter(|a| in_pre(&a.rel) && cone.rules.iter().any(|r| r.head.rel == a.rel))
        .map(|a| a.rel.to_string())
        .collect();
    if reads.iter().any(|r| cone.lattice.contains_key(r)) {
        return None;
    }
    let mut pre = cone.clone();
    pre.rules.retain(|r| in_pre(&r.head.rel));
    pre.outputs.retain(|o| in_pre(o) && cone.rules.iter().any(|r| r.head.rel == *o));
    pre.lattice.retain(|rel, _| in_pre(rel));
    let mut rounds = cone.clone();
    rounds.rules.retain(|r| !in_pre(&r.head.rel));
    for r in &mut rounds.rules {
        for l in &mut r.body {
            if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = l
                && reads.contains(a.rel.as_str())
            {
                a.rel = format!("{}{PRE}", a.rel).into();
            }
        }
    }
    rounds.outputs.retain(|o| !(in_pre(o) && cone.rules.iter().any(|r| r.head.rel == *o)));
    rounds.lattice.retain(|rel, _| !in_pre(rel) || !cone.rules.iter().any(|r| r.head.rel == *rel));
    for rel in &reads {
        rounds.decls.insert(format!("{rel}{PRE}"), cone.decls[rel].clone());
        rounds.inputs.push(format!("{rel}{PRE}"));
    }
    Some(PreSplit { pre, rounds, reads })
}

/// The digest of one round's rows of the hop relations: the early cutoff compares consecutive rounds'.
pub(crate) fn digest(rows: &BTreeMap<String, Vec<Vec<i64>>>) -> u64 {
    let sorted = rows.iter().map(|(rel, rows)| {
        let mut sorted = rows.clone();
        sorted.sort_unstable();
        sorted.dedup();
        (rel.clone(), sorted)
    });
    digest_sorted(&sorted.collect())
}

/// `digest` of rows already sorted and distinct per relation: no copy.
pub(crate) fn digest_sorted(rows: &BTreeMap<String, Vec<Vec<i64>>>) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    for (rel, rows) in rows {
        (rel, rows).hash(&mut h);
    }
    h.finish()
}

/// What the rounds did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rounds {
    /// Rounds run, round 0 included.
    pub(crate) run: usize,
    /// The hop relations' rows stopped changing within `K` rounds: the result is the fixpoint.
    pub(crate) converged: bool,
    /// Each round's hop rows, summed over the hop relations.
    pub rows: Vec<usize>,
    /// The rounds did not converge, and the pending round ran (`UNCONVERGED`).
    pub(crate) pending: bool,
}

#[cfg(test)]
mod tests;
