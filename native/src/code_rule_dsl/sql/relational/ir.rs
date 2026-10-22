//! Set relations with typed `(file, node)` identity. No SQL text is stored in this IR.
use super::super::terminals::Terminal;

pub(super) type RelationId = usize;

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(super) enum ColumnType {
    FileId,
    NodeId,
}

pub(super) const SCHEMA: [(&str, ColumnType); 2] =
    [("file", ColumnType::FileId), ("node", ColumnType::NodeId)];

/// A directed edge from the candidate root to a related candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Edge {
    Parent,
    Child,
    Previous,
    Next,
}

/// Every operator returns a set of node keys. Witness order is unobservable in this admitted subset.
#[derive(Clone, Debug)]
pub(super) enum Set {
    AllNodes,
    Empty,
    Kinds(Vec<u16>),
    Regex(usize),
    Pattern(usize),
    Intersect(Vec<Set>),
    Union(Vec<Set>),
    Complement(Box<Set>),
    Preimage(Edge, Box<Set>),
    Reference(RelationId),
    /// Strict descendants of roots. The emitter collapses nested root intervals.
    Within(RelationId),
    /// Inverse of capture-free `has`: ancestors of matching descendants, with inclusive stops.
    Ancestors {
        seeds: RelationId,
        stop: Option<RelationId>,
    },
}

pub(super) struct Relation {
    pub source: String,
    pub expression: Set,
    /// A proved closed subtree demand. It restricts seed output keys, never metadata probes.
    pub domain: Option<RelationId>,
    pub keys: Option<KeyDemand>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(super) struct KeyDemand {
    pub root_kinds: Vec<u16>,
    pub parent_kinds: Vec<u16>,
}

#[derive(Default)]
pub(super) struct Program {
    pub relations: Vec<Relation>,
    pub terminals: Vec<Terminal>,
    pub demand_decisions: Vec<String>,
}

pub(super) fn references(set: &Set, out: &mut Vec<RelationId>) {
    match set {
        Set::Reference(id) | Set::Within(id) => out.push(*id),
        Set::Ancestors { seeds, stop } => {
            out.push(*seeds);
            out.extend(stop);
        }
        Set::Union(sets) | Set::Intersect(sets) => {
            for set in sets {
                references(set, out);
            }
        }
        Set::Complement(set) | Set::Preimage(_, set) => references(set, out),
        _ => {}
    }
}

pub(super) fn reachable(program: &Program) -> std::collections::BTreeSet<RelationId> {
    let mut found = std::collections::BTreeSet::new();
    let mut todo = vec![0];
    while let Some(id) = todo.pop() {
        if !found.insert(id) {
            continue;
        }
        references(&program.relations[id].expression, &mut todo);
        todo.extend(program.relations[id].domain);
    }
    found
}
