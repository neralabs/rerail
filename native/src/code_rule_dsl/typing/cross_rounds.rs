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
/// round 4). Since decision 0052 a program states it (`PRAGMA`); this is the value they state.
pub(crate) const K: usize = 4;

/// A program's flag for the pending round (O41, decision 0047): a program that declares the fact `unconverged(0)`
/// gets one more round after round `K` when the rounds have not converged, with `unconverged(1)` added. Its rules
/// that read `unconverged(1)` make unknown what a missing row would otherwise decide.
pub(crate) const UNCONVERGED: &str = "unconverged";

/// The pragma by which a program asks for bounded rounds (decision 0052): `.pragma "cross-rounds" "4"`. A typing
/// program declares it: its cross-file recursion is over interface summaries, cut at `k` file crossings, with the
/// pending round when it also declares `unconverged`. A program that does not declare it (a rule) has its
/// cross-file part run as one fixpoint, which no chain length cuts.
pub(crate) const PRAGMA: &str = "cross-rounds";

/// The rounds `prog` declares (`PRAGMA`), if it declares them.
/// The program setting `.pragma "cross-rounds" "k"` lowers to (from `.rr`, or the `.rdl` copies' same line).
pub(crate) fn setting(k: &str) -> (String, String) {
    ("pragma".to_owned(), format!("\"{PRAGMA}\" \"{k}\""))
}

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
    let cols = prog.decls.get(&a.rel)?;
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
fn same_document(body: &[Literal], same_doc: &BTreeMap<String, Vec<usize>>, v: &str) -> BTreeSet<String> {
    let groups: Vec<BTreeSet<&str>> = body
        .iter()
        .filter_map(|l| match l {
            Literal::Atom(a) if !a.neg => Some(a),
            _ => None,
        })
        .filter_map(|a| {
            let cols = same_doc.get(&a.rel)?;
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
                    unbounded.insert(rule.head.rel.clone());
                }
            }
        }
        if found.is_empty() || found.len() > MAX_HOPS {
            if !found.is_empty() {
                unbounded.insert(rule.head.rel.clone());
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
                    hops.insert(a.rel.clone());
                    a.rel = format!("{}{PREV}", a.rel);
                }
                body.push(Literal::Cmp { l: doc(v), op: if across { "!=" } else { "=" }, r: doc(h) });
            }
            out.rules.push(Rule { head: rule.head.clone(), body });
        }
    }
    for rel in &hops {
        let prev = format!("{rel}{PREV}");
        out.decls.insert(prev.clone(), prog.decls[rel].clone());
        if let Some(l) = prog.lattice.get(rel) {
            out.lattice.insert(prev.clone(), l.clone());
        }
        out.inputs.push(prev);
    }
    Ok(Bounded { program: out, hops, split, unbounded })
}

/// The digest of one round's rows of the hop relations: the early cutoff compares consecutive rounds'.
pub(crate) fn digest(rows: &BTreeMap<String, Vec<Vec<i64>>>) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    for (rel, rows) in rows {
        let mut sorted = rows.clone();
        sorted.sort_unstable();
        sorted.dedup();
        (rel, sorted).hash(&mut h);
    }
    h.finish()
}

/// What the rounds did.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Rounds {
    /// Rounds run, round 0 included.
    pub(crate) run: usize,
    /// The hop relations' rows stopped changing within `K` rounds: the result is the fixpoint.
    pub(crate) converged: bool,
    /// Each round's hop rows, summed over the hop relations.
    pub(crate) rows: Vec<usize>,
    /// The rounds did not converge, and the pending round ran (`UNCONVERGED`).
    pub(crate) pending: bool,
}

#[cfg(test)]
mod tests;
