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

/// The pragma by which a program asks for bounded rounds (decision 0056): `.pragma "cross-rounds" "4"`. A typing
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
    /// The dependency relations (decision 0059), when `cutoff()` asks for them: what each document's across rules
    /// may read of each hop relation's previous rows.
    pub(crate) deps: Vec<Dep>,
    /// Hop relations read by an across rule whose reader cannot be named (its head node is bound only by the hop
    /// atom): a change to one makes every document dirty.
    pub(crate) global: BTreeSet<String>,
}

/// The prefix of a dependency relation (decision 0059): `rr_dep__<rel>__<cols>(reader, key...)` holds the keys of
/// `rel`'s previous rows (its columns `cols`) that the across rules deriving rows about node `reader` may probe.
pub(crate) const DEP: &str = "rr_dep__";

/// A dependency relation: `name` holds `(doc, key...)` rows, a key being `rel`'s columns `cols`. Or, when `head` is
/// set, no relation: the reader's head node is `rel`'s column `head` itself (`qv(..., v) :- bv(m, x, k, v)`), so a
/// changed row of `rel` makes that column's document dirty.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Dep {
    pub(crate) rel: String,
    pub(crate) cols: Vec<usize>,
    pub(crate) name: String,
    pub(crate) head: Option<usize>,
    /// A same-document read of the round's own rows, not of the previous round's.
    pub(crate) inround: bool,
    /// `rel`'s first node column: a row's node.
    pub(crate) first: Option<usize>,
}

/// The per-document cutoff's setting (decision 0059, step 1): `RERAIL_CROSS_CUTOFF=measure` derives the dependency
/// relations and reports each round's dirty documents, and changes nothing else.
pub(crate) fn cutoff() -> Option<String> {
    std::env::var("RERAIL_CROSS_CUTOFF").ok().filter(|v| !v.is_empty())
}

/// The dependency rule of the across atom `body[i]` read by a rule whose head node is `h` (decision 0059): the body
/// without the atom, cut to the literals joined to `h` through other literals' variables, and the filters those bind.
/// Its head is `(h, key...)`, the reader's node (its document, or a finer unit, is the runner's), the key the atom's columns that the rest binds (constants included). `None`
/// when `h` is bound only through the atom: no reader can be named.
fn dep_rule(body: &[Literal], i: usize, h: &str) -> Option<(Vec<usize>, Vec<Term>, Vec<Literal>)> {
    let Literal::Atom(hop) = &body[i] else { return None };
    let mut reach: BTreeSet<String> = BTreeSet::from([h.to_owned()]);
    let mut bound: BTreeSet<String> = BTreeSet::new();
    let mut kept = vec![false; body.len()];
    loop {
        let mut grew = false;
        for (j, l) in body.iter().enumerate() {
            if j == i || kept[j] {
                continue;
            }
            let take = match l {
                Literal::Atom(a) if !a.neg => {
                    let vs = a.vars();
                    if vs.is_disjoint(&reach) { None } else { Some(vs) }
                }
                Literal::Nearest { n, a, .. } if bound.contains(n) => Some(BTreeSet::from([n.clone(), a.clone()])),
                Literal::Cmp { l: Term::Var(x), op: "=", r } | Literal::Cmp { l: r, op: "=", r: Term::Var(x) } if !bound.contains(x) => {
                    let mut vs = BTreeSet::new();
                    r.vars(&mut vs);
                    if vs.is_subset(&bound) && !vs.is_empty() { vs.insert(x.clone()); Some(vs) } else { None }
                }
                _ => None,
            };
            if let Some(vs) = take {
                kept[j] = true;
                reach.extend(vs.iter().cloned());
                bound.extend(vs);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    if !bound.contains(h) {
        return None;
    }
    // Filters whose variables are all bound: they only narrow what the rule may probe.
    for (j, l) in body.iter().enumerate() {
        if j != i && !kept[j] && !matches!(l, Literal::Agg { .. } | Literal::Ast { .. }) && l.vars().is_subset(&bound) {
            kept[j] = true;
        }
    }
    let (mut cols, mut key) = (Vec::new(), Vec::new());
    for (c, t) in hop.args.iter().enumerate() {
        match t {
            Term::Var(v) if bound.contains(v) => {
                cols.push(c);
                key.push(t.clone());
            }
            Term::Int(_) | Term::Str(_) => {
                cols.push(c);
                key.push(t.clone());
            }
            _ => {}
        }
    }
    let out: Vec<Literal> = body.iter().enumerate().filter(|(j, _)| kept[*j]).map(|(_, l)| l.clone()).collect();
    Some((cols, key, out))
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

/// Adds the dependency rule of `body[i]`, an atom of `rel` read by a rule whose head node is `h` (`dep_rule`), to
/// `dep_rules`, its relation to `deps` and `out`'s declarations; or names `rel` in `deps` by its head column, or in
/// `global`. `inround`: a same-document read of the round's own rows, not a hop.
#[allow(clippy::too_many_arguments)]
fn push_dep(prog: &Program, out: &mut Program, deps: &mut Vec<Dep>, dep_rules: &mut Vec<Rule>, global: &mut BTreeSet<String>, body: &[Literal], i: usize, h: &str, rel: &str, inround: bool) {
    match dep_rule(body, i, h) {
        Some((cols, key, dep_body)) => {
            let name = format!("{DEP}{}{rel}__{}", if inround { "in__" } else { "" }, cols.iter().map(usize::to_string).collect::<Vec<_>>().join("_"));
            if !deps.iter().any(|d| d.name == name) {
                let mut decl = vec![("reader".to_owned(), "node".to_owned())];
                decl.extend(cols.iter().map(|&c| prog.decls[rel][c].clone()));
                out.decls.insert(name.clone(), decl);
                deps.push(Dep { rel: rel.to_owned(), cols, name: name.clone(), head: None, inround, first: first_col(prog, rel) });
            }
            let mut args = vec![Term::Var(h.to_owned())];
            args.extend(key);
            dep_rules.push(Rule { head: Atom { rel: name, args, neg: false, gated: false }, body: dep_body });
        }
        None => match body[i].atom().expect("an atom").args.iter().position(|t| matches!(t, Term::Var(v) if v == h)) {
            Some(c) => {
                if !deps.iter().any(|d| d.rel == rel && d.head == Some(c) && d.inround == inround) {
                    deps.push(Dep { rel: rel.to_owned(), cols: Vec::new(), name: String::new(), head: Some(c), inround, first: first_col(prog, rel) });
                }
            }
            // A same-document read whose reader cannot be named is not counted (the measure's closure is a lower
            // bound for it).
            None if inround => {}
            None => {
                global.insert(rel.to_owned());
            }
        },
    }
}

fn first_col(prog: &Program, rel: &str) -> Option<usize> {
    prog.decls.get(rel)?.iter().position(|(_, t)| t == "node")
}

/// The cross-file part `prog` with its hops reading the previous round. `same_doc` holds the same-document columns of
/// the relations `prog` reads that cross no file (`file_local::Programs::same_doc`).
pub(crate) fn bound(prog: &Program, same_doc: &BTreeMap<String, Vec<usize>>) -> Result<Bounded, front::Error> {
    let (sccs, _) = front::strata(prog)?;
    let comp: BTreeMap<&str, usize> = sccs.iter().enumerate().flat_map(|(i, s)| s.iter().map(move |r| (r.as_str(), i))).collect();
    let mut out = prog.clone();
    out.rules.clear();
    let (mut hops, mut split, mut unbounded) = (BTreeSet::new(), Vec::new(), BTreeSet::new());
    let (measure, mut deps, mut global, mut dep_rules) = (cutoff().is_some(), Vec::<Dep>::new(), BTreeSet::new(), Vec::new());
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
        let mut inround = Vec::new();
        for &i in &recursive {
            let a = rule.body[i].atom().expect("recursive literals are atoms");
            match (&head_node, first_node_var(prog, a)) {
                (Some(h), Some(v)) if *h == v || local_to_head.contains(&v) => {
                    if matches!(rule.body[i], Literal::Atom(_)) && *h != v {
                        inround.push(i);
                    }
                }
                (Some(h), Some(v)) if matches!(rule.body[i], Literal::Atom(_)) => found.push((i, h.clone(), v)),
                _ => {
                    unbounded.insert(rule.head.rel.clone());
                }
            }
        }
        // Same-document reads of the round's own rows (decision 0059, Measured): what a dirty node makes dirty in
        // the same round.
        if measure && let Some(h) = &head_node {
            for &i in &inround {
                let rel = rule.body[i].atom().expect("an atom").rel.clone();
                push_dep(prog, &mut out, &mut deps, &mut dep_rules, &mut global, &rule.body, i, h, &rel, true);
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
            if measure {
                for (bit, (i, h, _)) in found.iter().enumerate() {
                    if choice & (1 << bit) != 0 {
                        let rel = rule.body[*i].atom().expect("an atom").rel.clone();
                        push_dep(prog, &mut out, &mut deps, &mut dep_rules, &mut global, &body, *i, h, &rel, false);
                    }
                }
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
    out.rules.extend(dep_rules);
    Ok(Bounded { program: out, hops, split, unbounded, deps, global })
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
        .map(|a| a.rel.clone())
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
                && reads.contains(&a.rel)
            {
                a.rel = format!("{}{DONE}", a.rel);
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
    // A dependency relation (decision 0059) is read by the loop each round, so it runs in the rounds.
    let mut changing: BTreeSet<&str> = cone.rules.iter().map(|r| r.head.rel.as_str()).filter(|r| r.starts_with(DEP)).collect();
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
        .map(|a| a.rel.clone())
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
                && reads.contains(&a.rel)
            {
                a.rel = format!("{}{PRE}", a.rel);
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

/// The negated atoms of `rounds` over a relation that changes with the rounds (reads a `__prev` row or `unconverged`,
/// directly or through others), by relation: when none, the rounds are monotone (decision 0059, Measured).
pub(crate) fn negated_changing(rounds: &Program) -> BTreeSet<String> {
    let changes_name = |r: &str| r.ends_with(PREV) || r == UNCONVERGED;
    let mut changing: BTreeSet<&str> = BTreeSet::new();
    loop {
        let before = changing.len();
        for r in &rounds.rules {
            if !changing.contains(r.head.rel.as_str()) && r.body.iter().filter_map(Literal::atom).any(|a| changes_name(&a.rel) || changing.contains(a.rel.as_str())) {
                changing.insert(r.head.rel.as_str());
            }
        }
        if changing.len() == before {
            break;
        }
    }
    let mut out = BTreeSet::new();
    for r in &rounds.rules {
        for l in &r.body {
            match l {
                Literal::Atom(a) if a.neg && (changes_name(&a.rel) || changing.contains(a.rel.as_str())) => {
                    out.insert(a.rel.clone());
                }
                Literal::Agg { atom, .. } if changes_name(&atom.rel) || changing.contains(atom.rel.as_str()) => {
                    out.insert(format!("{} (aggregate)", atom.rel));
                }
                _ => {}
            }
        }
    }
    out
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
