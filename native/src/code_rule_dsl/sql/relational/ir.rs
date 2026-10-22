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
    Intersect(Vec<Set>),
    Union(Vec<Set>),
    Complement(Box<Set>),
    Preimage(Edge, Box<Set>),
    Reference(RelationId),
}

pub(super) struct Relation {
    pub source: String,
    pub expression: Set,
}

#[derive(Default)]
pub(super) struct Program {
    pub relations: Vec<Relation>,
    pub terminals: Vec<Terminal>,
}
