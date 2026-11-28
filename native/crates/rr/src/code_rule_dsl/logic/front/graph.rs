//! A version's dependency graph by relation (decision 0173, stage 2 step 2): the derived relations, the edges between
//! them with whether each is negative, and the non-monotone aggregation edges, read from the rules by their heads and
//! the other fields by `About`. Here, under `program_image::analyses` (which re-exports it), so `front`'s checks of a
//! version (`cycles::check`) read the strata by name as `analyses` does.
use crate::code_rule_dsl::logic::front::Error;
use crate::code_rule_dsl::logic::program_image::handles::{AggRef, Lit};
use crate::code_rule_dsl::logic::program_image::{About, RelId, RuleRef, Version};
use std::collections::{BTreeMap, BTreeSet};

/// Dependency edges, head → dependency, with whether each is negative (an aggregate's is, a gated negation's is
/// not): sorted by head and dependency, each once.
/// A section (decision 0628 §4), shared by a count.
#[derive(Clone)]
pub struct Edges(std::sync::Arc<rerail_section::Section<EdgeList>>);

/// The edges' section: each one as it lies.
#[derive(rerail_section::Archive, Default)]
pub struct EdgeList {
    #[archive(flat)]
    edges: Vec<(RelId, RelId, bool)>,
}

impl Default for Edges {
    fn default() -> Edges {
        Edges::of(Vec::new())
    }
}

impl PartialEq for Edges {
    fn eq(&self, o: &Edges) -> bool {
        self.as_slice() == o.as_slice()
    }
}

impl Eq for Edges {}

impl std::fmt::Debug for Edges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.as_slice()).finish()
    }
}

impl Edges {
    pub fn of(mut edges: Vec<(RelId, RelId, bool)>) -> Edges {
        edges.sort_unstable();
        edges.dedup();
        Edges(std::sync::Arc::new(rerail_section::Section::of(&EdgeList { edges })))
    }
    /// What `head` depends on, and whether negatively.
    pub fn of_head(&self, head: RelId) -> impl Iterator<Item = (RelId, bool)> + '_ {
        let edges = self.as_slice();
        let s = edges.partition_point(|e| e.0 < head);
        edges[s..].iter().take_while(move |e| e.0 == head).map(|e| (e.1, e.2))
    }
    /// The edges, sorted by head and dependency.
    pub fn as_slice(&self) -> &[(RelId, RelId, bool)] {
        &self.0.edges
    }
    pub fn iter(&self) -> impl Iterator<Item = (RelId, RelId, bool)> + '_ {
        self.as_slice().iter().copied()
    }
}

/// The derived relations (heads, and what answers produce), the edges, and the non-monotone aggregation edges.
pub fn components(v: &Version, about: &About) -> (Vec<RelId>, Edges, Vec<(RelId, RelId)>) {
    components_keyed(v, about, None)
}

/// `components`, and where `keys`, each relation with rules' key of the edges they add (`edge_key`).
fn components_keyed(v: &Version, about: &About, mut keys: Option<&mut BTreeMap<String, u64>>) -> (Vec<RelId>, Edges, Vec<(RelId, RelId)>) {
    let n = v.rel_end() as usize;
    let derived = derived_of(v, about);
    let mut edges = Vec::new();
    for (p, req) in about.produced() {
        edges.push((p, req, false));
    }
    for nat in &about.natives {
        if derived[nat.rel.index()] {
            edges.extend(nat.reads.iter().filter(|r| derived[r.index()]).map(|&r| (nat.rel, r, false)));
        }
    }
    let mut aggs = Vec::new();
    let mut per_head: Vec<Vec<(RelId, bool, bool)>> = if keys.is_some() { vec![Vec::new(); n] } else { Vec::new() };
    let mut has_rules = vec![false; if keys.is_some() { n } else { 0 }];
    for r in v.rules() {
        let head = r.head().rel();
        rule_edges(v, about, &derived, r, |w, neg, agg| {
            if agg {
                aggs.push((head, w));
            }
            edges.push((head, w, neg));
            if let Some(e) = per_head.get_mut(head.index()) {
                e.push((w, neg, agg));
            }
        });
        if let Some(h) = has_rules.get_mut(head.index()) {
            *h = true;
        }
    }
    if let Some(keys) = keys.as_deref_mut() {
        for (i, e) in per_head.into_iter().enumerate().filter(|(i, _)| has_rules[*i]) {
            keys.insert(v.rel_name(RelId(i as u32)).to_owned(), edge_key(v, e));
        }
    }
    guest_copies_after_requests(v, about, &derived, &mut edges);
    aggs.sort_unstable();
    aggs.dedup();
    let derived = (0..n as u32).map(RelId).filter(|r| derived[r.index()]).collect();
    (derived, Edges::of(edges), aggs)
}

/// Which relations are derived: the heads of rules, and what answers produce.
pub fn derived_of(v: &Version, about: &About) -> Vec<bool> {
    let mut derived = vec![false; v.rel_end() as usize];
    for r in v.rules() {
        derived[r.head().rel().index()] = true;
    }
    for (p, _) in about.produced() {
        derived[p.index()] = true;
    }
    derived
}

/// The edges rule `r` adds to the graph: per atom reading a derived relation, its relation, whether the edge is negative
/// and whether it is an aggregation not monotone in the head's lattice (negative too). A gated atom's is neither.
pub fn rule_edges(v: &Version, about: &About, derived: &[bool], r: RuleRef<'_>, mut edge: impl FnMut(RelId, bool, bool)) {
    let head = r.head().rel();
    for l in r.body() {
        let Some(a) = l.atom() else { continue };
        if !derived[a.rel().index()] {
            continue;
        }
        let agg = match l.view() {
            Lit::Agg { op, .. } => !about.lattice_of(head).is_some_and(|lat| {
                matches!((op, v.text(lat)), (AggRef::Count | AggRef::Max, "max") | (AggRef::Min, "min"))
            }),
            _ => false,
        };
        edge(a.rel(), (a.neg() || agg) && !a.gated(), agg && !a.gated());
    }
}

/// A key of the edges a relation's rules add (`rule_edges`), by the names they read: two versions whose relations
/// with rules are the same with the same keys have one graph.
fn edge_key(v: &Version, edges: Vec<(RelId, bool, bool)>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut e: Vec<(&str, bool, bool)> = edges.into_iter().map(|(w, neg, agg)| (v.rel_name(w), neg, agg)).collect();
    e.sort_unstable();
    e.dedup();
    let mut h = rustc_hash::FxHasher::default();
    e.hash(&mut h);
    h.finish()
}

/// `rel`'s key of its rules' edges (`edge_key`), `None` where it has no rule: what the components are kept by
/// (`Components::edge_keys`).
pub fn head_edge_key(v: &Version, about: &About, derived: &[bool], rel: RelId) -> Option<u64> {
    let mut e = Vec::new();
    let mut any = false;
    for r in v.heads(rel) {
        any = true;
        rule_edges(v, about, derived, r, |w, neg, agg| e.push((w, neg, agg)));
    }
    any.then(|| edge_key(v, e))
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
pub fn components_by_name(v: &Version, about: &About) -> Result<crate::code_rule_dsl::logic::front::Components, Error> {
    components_by_name_keyed(v, about, None)
}

/// `components_by_name`, with each relation with rules' key of its edges (`head_edge_key`): the analysis's, which
/// keeps them over an edit that changes no edge.
pub fn components_by_name_with_keys(v: &Version, about: &About) -> Result<crate::code_rule_dsl::logic::front::Components, Error> {
    let mut keys = BTreeMap::new();
    let c = components_by_name_keyed(v, about, Some(&mut keys))?;
    Ok(c.keyed(keys))
}

fn components_by_name_keyed(v: &Version, about: &About, keys: Option<&mut BTreeMap<String, u64>>) -> Result<crate::code_rule_dsl::logic::front::Components, Error> {
    let (mut by_name, edges, aggs) = components_keyed(v, about, keys);
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

