//! A version's dependency graph by relation (decision 0173, stage 2 step 2): the derived relations, the edges between
//! them with whether each is negative, and the non-monotone aggregation edges, read from the rules by their heads and
//! the other fields by `About`. Here, under `program_image::analyses` (which re-exports it), so `front`'s checks of a
//! version (`cycles::check`) read the strata by name as `analyses` does.
use crate::code_rule_dsl::logic::front::Error;
use crate::code_rule_dsl::logic::program_image::handles::{AggRef, Lit};
use crate::code_rule_dsl::logic::program_image::{About, RelId, Version};
use std::collections::BTreeSet;

/// Dependency edges, head → dependency, with whether each is negative (an aggregate's is, a gated negation's is
/// not): sorted by head and dependency, each once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edges {
    edges: Vec<(RelId, RelId, bool)>,
}

impl Edges {
    pub fn of(mut edges: Vec<(RelId, RelId, bool)>) -> Edges {
        edges.sort_unstable();
        edges.dedup();
        Edges { edges }
    }
    /// What `head` depends on, and whether negatively.
    pub fn of_head(&self, head: RelId) -> impl Iterator<Item = (RelId, bool)> + '_ {
        let s = self.edges.partition_point(|e| e.0 < head);
        self.edges[s..].iter().take_while(move |e| e.0 == head).map(|e| (e.1, e.2))
    }
    /// The edges, sorted by head and dependency.
    pub fn as_slice(&self) -> &[(RelId, RelId, bool)] {
        &self.edges
    }
    pub fn iter(&self) -> impl Iterator<Item = (RelId, RelId, bool)> + '_ {
        self.edges.iter().copied()
    }
}

/// The derived relations (heads, and what answers produce), the edges, and the non-monotone aggregation edges.
pub fn components(v: &Version, about: &About) -> (Vec<RelId>, Edges, Vec<(RelId, RelId)>) {
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
    guest_copies_after_requests(v, about, &derived, &mut edges);
    aggs.sort_unstable();
    aggs.dedup();
    let derived = (0..n as u32).map(RelId).filter(|r| derived[r.index()]).collect();
    (derived, Edges::of(edges), aggs)
}

/// The prefix of a relation hiding copies for a guest's grammar (`rr_in_<grammar>_<relation>`, decision 0058 §2): its
/// rules read the nodes of that grammar's documents.
pub const GUEST_COPY: &str = "rr_in_";

/// A guest copy runs after each answer the run gives (decision 0597): a request is answered where a stratum reads
/// what it produces (`parsed`), and the answer admits documents (a Vue template's pieces, a string's tape) whose rows
/// the strata above read. A copy made only of base atoms (`decl_site` over `kind` and `field`) depends on no derived
/// relation, so it could be placed below and never see them (vue/code's `tvar` was empty unless another rule happened
/// to place it above). The edge orders only: it is never negative, and it is not added where the answer reaches the
/// copy (a nested guest's ask reads it), which would close a cycle.
fn guest_copies_after_requests(v: &Version, about: &About, derived: &[bool], edges: &mut Vec<(RelId, RelId, bool)>) {
    let mut answers: Vec<RelId> = about.produced().map(|(p, _)| p).collect();
    answers.sort_unstable();
    answers.dedup();
    if answers.is_empty() {
        return;
    }
    let copies: Vec<RelId> = (0..derived.len() as u32).map(RelId).filter(|r| derived[r.index()] && v.rel_name(*r).starts_with(GUEST_COPY)).collect();
    if copies.is_empty() {
        return;
    }
    let mut deps: Vec<Vec<RelId>> = vec![Vec::new(); derived.len()];
    for &(h, w, _) in edges.iter() {
        deps[h.index()].push(w);
    }
    for answer in answers {
        let mut reached = vec![false; derived.len()];
        let mut todo = vec![answer];
        reached[answer.index()] = true;
        while let Some(r) = todo.pop() {
            for &w in &deps[r.index()] {
                if !reached[w.index()] {
                    reached[w.index()] = true;
                    todo.push(w);
                }
            }
        }
        // The next answer's reach counts these edges too: through one, it could reach a copy.
        for &c in copies.iter().filter(|c| !reached[c.index()]) {
            edges.push((c, answer, false));
            deps[c.index()].push(answer);
        }
    }
}

/// The dependency edges by name, as `analyses::strata` gives them: each derived relation's dependencies among the derived
/// relations, with whether each is negative.
pub fn edges_by_name(v: &Version, about: &About) -> crate::code_rule_dsl::logic::front::Edges {
    let (_, edges, _) = components(v, about);
    let mut out = crate::code_rule_dsl::logic::front::Edges::new();
    for (h, w, neg) in edges.iter() {
        out.entry(v.rel_name(h).to_owned()).or_default().insert((v.rel_name(w).to_owned(), neg));
    }
    out
}

/// The components by name, dependencies first (`analyses::strata` without its edges), in the tree's order: the relations numbered in the order of their
/// names, and the tree's error where a negation closes a cycle. For a reader whose output follows the components' order (the
/// composed query lists its definitions in it); `strata` gives them in an order of ids.
pub fn components_by_name(v: &Version, about: &About) -> Result<Vec<BTreeSet<String>>, Error> {
    let (mut by_name, edges, aggs) = components(v, about);
    by_name.sort_unstable_by(|a, b| v.rel_name(*a).cmp(v.rel_name(*b)));
    const NONE: u32 = u32::MAX;
    let mut place = vec![NONE; v.rel_end() as usize];
    for (i, r) in by_name.iter().enumerate() {
        place[r.index()] = i as u32;
    }
    let mut succ: Vec<Vec<(u32, bool)>> = vec![Vec::new(); by_name.len()];
    for (h, w, neg) in edges.iter() {
        let (h, w) = (place[h.index()], place[w.index()]);
        if h != NONE && w != NONE {
            succ[h as usize].push((w, neg));
        }
    }
    for s in &mut succ {
        s.sort_unstable();
    }
    let aggs = aggs.iter().map(|(h, w)| (place[h.index()], place[w.index()])).collect();
    crate::code_rule_dsl::logic::front::components_of_graph(by_name.iter().map(|&r| v.rel_name(r)).collect(), succ, aggs)
}

