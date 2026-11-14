//! The whole-program analyses over a version (decision 0173, stage 2 step 2), each relation by its id: the strata
//! (`front::strata`), what the outputs need (`front::needed`), the projections (`plan::projections`), and whether
//! the composed path accepts the program (`compose::accepts`). Each reads the rules by their heads and the other
//! fields by `About`, and is checked against its tree form on the pack programs (`tests`).
//!
//! The strata come in an order of ids, where the tree form's come in an order of names: each is dependencies first.
use super::about::{About, NativeKind};
use super::handles::{AggRef, Lit, TermRef};
use super::version::Version;
use super::RelId;
use crate::code_rule_dsl::logic::front::Error;
use std::collections::BTreeSet;

/// Dependency edges, head → dependency, with whether each is negative (an aggregate's is, a gated negation's is
/// not): sorted by head and dependency, each once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edges {
    edges: Vec<(RelId, RelId, bool)>,
}

impl Edges {
    fn of(mut edges: Vec<(RelId, RelId, bool)>) -> Edges {
        edges.sort_unstable();
        edges.dedup();
        Edges { edges }
    }
    /// What `head` depends on, and whether negatively.
    pub fn of_head(&self, head: RelId) -> impl Iterator<Item = (RelId, bool)> + '_ {
        let s = self.edges.partition_point(|e| e.0 < head);
        self.edges[s..].iter().take_while(move |e| e.0 == head).map(|e| (e.1, e.2))
    }
    pub fn iter(&self) -> impl Iterator<Item = (RelId, RelId, bool)> + '_ {
        self.edges.iter().copied()
    }
}

/// The program's components, dependencies first, each sorted by id; and its dependency edges.
#[derive(Clone, Debug, Default)]
pub struct Strata {
    pub sccs: Vec<Vec<RelId>>,
    pub edges: Edges,
}

/// The derived relations (heads, and what answers produce), the edges, and the non-monotone aggregation edges.
fn components(v: &Version, about: &About) -> (Vec<RelId>, Edges, Vec<(RelId, RelId)>) {
    let n = v.rel_end() as usize;
    let mut derived = vec![false; n];
    for r in v.rules() {
        derived[r.head().rel().index()] = true;
    }
    let mut edges = Vec::new();
    for (p, req) in about.produced() {
        derived[p.index()] = true;
        edges.push((p, req, false));
    }
    for nat in &about.natives {
        if derived[nat.rel.index()] {
            edges.extend(nat.reads.iter().filter(|r| derived[r.index()]).map(|&r| (nat.rel, r, false)));
        }
    }
    let mut aggs = Vec::new();
    for r in v.rules() {
        let head = r.head().rel();
        for l in r.body() {
            let Some(a) = l.atom() else { continue };
            if !derived[a.rel().index()] {
                continue;
            }
            // Aggregation that is not monotone in the head's lattice is negative.
            let agg = match l.view() {
                Lit::Agg { op, .. } => !about.lattice_of(head).is_some_and(|lat| {
                    matches!((op, v.text(lat)), (AggRef::Count | AggRef::Max, "max") | (AggRef::Min, "min"))
                }),
                _ => false,
            };
            if agg && !a.gated() {
                aggs.push((head, a.rel()));
            }
            edges.push((head, a.rel(), (a.neg() || agg) && !a.gated()));
        }
    }
    aggs.sort_unstable();
    aggs.dedup();
    let derived = (0..n as u32).map(RelId).filter(|r| derived[r.index()]).collect();
    (derived, Edges::of(edges), aggs)
}

/// The strata (`front::strata`): an error if a negation or a non-monotone aggregate closes a cycle.
pub fn strata(v: &Version, about: &About) -> Result<Strata, Error> {
    let (derived, edges, aggs) = components(v, about);
    let sccs = tarjan(v.rel_end() as usize, &derived, &edges);
    let mut at = vec![usize::MAX; v.rel_end() as usize];
    for (i, scc) in sccs.iter().enumerate() {
        for r in scc {
            at[r.index()] = i;
        }
    }
    for (i, scc) in sccs.iter().enumerate() {
        for &h in scc {
            for (w, neg) in edges.of_head(h) {
                if !neg || at[w.index()] != i {
                    continue;
                }
                let names = |s: &[RelId]| s.iter().map(|&r| v.rel_name(r).to_owned()).collect::<BTreeSet<_>>();
                let (head, dep) = (v.rel_name(h).to_owned(), v.rel_name(w).to_owned());
                return Err(if aggs.binary_search(&(h, w)).is_ok() {
                    Error::AggregationInRecursion { head, aggregated: dep, component: names(scc) }
                } else {
                    Error::NotStratifiable {
                        message: format!("not stratifiable: {head} depends negatively on {dep} in its own component"),
                        component: names(scc),
                        negated: dep,
                    }
                });
            }
        }
    }
    Ok(Strata { sccs, edges })
}

/// Strongly connected components of `nodes` (sorted) under `edges`, dependencies first, each sorted. Iterative.
fn tarjan(n: usize, nodes: &[RelId], edges: &Edges) -> Vec<Vec<RelId>> {
    const NONE: u32 = u32::MAX;
    let mut node = vec![false; n];
    for r in nodes {
        node[r.index()] = true;
    }
    let (mut index, mut low, mut on) = (vec![NONE; n], vec![0u32; n], vec![false; n]);
    let (mut stack, mut sccs, mut counter) = (Vec::<RelId>::new(), Vec::new(), 0u32);
    // Frames: a node and the position of its next edge in `edges`.
    let mut frames: Vec<(RelId, usize)> = Vec::new();
    let first = |r: RelId| edges.edges.partition_point(|e| e.0 < r);
    for &root in nodes {
        if index[root.index()] != NONE {
            continue;
        }
        index[root.index()] = counter;
        low[root.index()] = counter;
        counter += 1;
        stack.push(root);
        on[root.index()] = true;
        frames.push((root, first(root)));
        while let Some(&mut (v, ref mut at)) = frames.last_mut() {
            if let Some(&(h, w, _)) = edges.edges.get(*at)
                && h == v
            {
                *at += 1;
                if !node[w.index()] {
                    continue;
                }
                if index[w.index()] == NONE {
                    index[w.index()] = counter;
                    low[w.index()] = counter;
                    counter += 1;
                    stack.push(w);
                    on[w.index()] = true;
                    frames.push((w, first(w)));
                } else if on[w.index()] {
                    low[v.index()] = low[v.index()].min(index[w.index()]);
                }
                continue;
            }
            frames.pop();
            if let Some(&(parent, _)) = frames.last() {
                low[parent.index()] = low[parent.index()].min(low[v.index()]);
            }
            if low[v.index()] == index[v.index()] {
                let mut scc = Vec::new();
                loop {
                    let w = stack.pop().expect("v is on the stack");
                    on[w.index()] = false;
                    scc.push(w);
                    if w == v {
                        break;
                    }
                }
                scc.sort_unstable();
                sccs.push(scc);
            }
        }
    }
    sccs
}

/// The relations `roots` read, directly or through other rules or natives, the roots included (`front::needed`).
/// Indexed by relation id.
pub fn needed(v: &Version, about: &About, roots: impl IntoIterator<Item = RelId>) -> Vec<bool> {
    let mut need = vec![false; v.rel_end() as usize];
    let mut todo: Vec<RelId> = roots.into_iter().collect();
    while let Some(r) = todo.pop() {
        if std::mem::replace(&mut need[r.index()], true) {
            continue;
        }
        if let Some(nat) = about.native(r) {
            todo.extend(&nat.reads);
        }
        for rule in v.heads(r) {
            todo.extend(rule.reads().map(|a| a.rel()));
        }
    }
    need
}

/// The relations whose one rule's body is a single positive atom of distinct variables, wildcards and integers,
/// with the integers' columns (`plan::projections`). Sorted by relation.
pub fn projections(v: &Version) -> Vec<(RelId, Vec<(usize, i64)>)> {
    let mut seen = vec![false; v.rel_end() as usize];
    let mut out = Vec::new();
    for r in v.rules() {
        let rel = r.head().rel();
        if std::mem::replace(&mut seen[rel.index()], true) {
            continue;
        }
        let mut heads = v.heads(rel);
        let (Some(only), None) = (heads.next(), heads.next()) else { continue };
        let mut body = only.body();
        let (Some(l), None) = (body.next(), body.next()) else { continue };
        let Lit::Atom(a) = l.view() else { continue };
        if a.neg() {
            continue;
        }
        let (mut vars, mut eqs, mut plain) = (Vec::new(), Vec::new(), true);
        for (c, t) in a.args().enumerate() {
            match t {
                TermRef::Var(s) => vars.push(s),
                TermRef::Wild => {}
                TermRef::Int(i) => eqs.push((c, i)),
                _ => plain = false,
            }
        }
        vars.sort_unstable();
        if plain && vars.windows(2).all(|w| w[0] != w[1]) {
            out.push((rel, eqs));
        }
    }
    out.sort_unstable_by_key(|p| p.0);
    out
}

/// Whether the composed path takes the program (`compose::accepts`): no lattices, answers, gates or natives but
/// Python's, no `finding` output, and every component one relation that reads itself at most once, linearly.
pub fn accepts(v: &Version, about: &About) -> Result<bool, Error> {
    if !about.lattice.is_empty()
        || !about.answers.is_empty()
        || about.gates != 0
        || about.natives.iter().any(|n| !matches!(n.kind, NativeKind::Python | NativeKind::PythonCandidates))
        || v.outputs().any(|o| v.text(o) == "finding")
    {
        return Ok(false);
    }
    for scc in strata(v, about)?.sccs {
        let [rel] = scc[..] else { return Ok(false) };
        for rule in v.heads(rel) {
            let reads = rule.reads().filter(|a| a.rel() == rel).count();
            if reads > 1
                || (reads == 1
                    && (about.native(rel).is_some()
                        || rule.body().any(|l| matches!(l.view(), Lit::Agg { .. } | Lit::Nearest { .. } | Lit::Ast { .. }))))
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
