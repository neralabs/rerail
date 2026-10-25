//! Immutable atomic batch statistics. Estimates select physical plans, never admission proofs.
use super::super::{exec::Batch, nodes::NONE};
use super::physical::{Arm, Predicate};
use crate::code_rule_dsl::language::GrammarDialect;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub(in crate::code_rule_dsl::sql) struct PlannerStats {
    kinds: BTreeMap<u16, u64>,
    edges: BTreeMap<(u16, u16), u64>,
    complete: bool,
    dialect: Option<GrammarDialect>,
    #[cfg(test)]
    force_materialization: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AnchorDecision {
    pub variable: usize,
    pub estimated_rows: u64,
    pub reason: &'static str,
}

impl PlannerStats {
    /// One atomic metadata pass; no matcher, source-text decoding, or SQL execution.
    /// Counts cover exactly this batch. New parsed subfiles are not covered by this snapshot.
    pub(super) fn from_batch(batch: &Batch, dialect: GrammarDialect) -> Self {
        let mut stats = Self {
            complete: true,
            dialect: Some(dialect),
            ..Self::default()
        };
        for file in batch.files.iter().filter(|file| file.dialect == dialect) {
            for id in 0..file.facts.len() {
                let kind = file.facts.kind(id);
                *stats.kinds.entry(kind).or_default() += 1;
                let parent = file.facts.parent(id);
                if parent != NONE {
                    *stats
                        .edges
                        .entry((file.facts.kind(parent), kind))
                        .or_default() += 1;
                }
            }
        }
        stats
    }

    pub(super) fn for_dialect(&self, dialect: GrammarDialect) -> Option<&Self> {
        (self.complete && self.dialect == Some(dialect)).then_some(self)
    }

    pub(super) fn known(&self) -> bool {
        self.complete
    }

    pub(super) fn kind_rows(&self, kinds: &[u16]) -> Option<u64> {
        self.complete.then(|| {
            kinds
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .iter()
                .map(|k| self.kinds.get(k).copied().unwrap_or(0))
                .sum()
        })
    }

    /// Observed immediate child count, not a recursive or conditional selectivity promise.
    pub(super) fn direct_edges(&self, parent: u16, child: u16) -> Option<u64> {
        self.complete
            .then(|| self.edges.get(&(parent, child)).copied().unwrap_or(0))
    }

    pub(super) fn anchor(&self, arm: &Arm) -> Option<AnchorDecision> {
        arm.predicates
            .iter()
            .filter_map(|p| match p {
                Predicate::Kinds(variable, kinds) => {
                    self.kind_rows(kinds).map(|estimated_rows| AnchorDecision {
                        variable: *variable,
                        estimated_rows,
                        reason: "minimum observed batch kind postings",
                    })
                }
                _ => None,
            })
            .min_by_key(|d| (d.estimated_rows, d.variable))
    }
}

#[cfg(test)]
#[path = "planner_stats_tests.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StorageDecision {
    pub relation: usize,
    pub consumers: usize,
    pub materialized: bool,
    pub reason: &'static str,
}

/// Count IR edges, including nested predicates. Never infer reuse from SQL text.
/// Only an ordinary single-reference join arm proves an uncorrelated one-shot use.
pub(super) fn materialization_decisions(
    program: &super::ir::Program,
    plans: &[super::physical::RelationPlan],
    stats: &PlannerStats,
) -> Vec<StorageDecision> {
    use super::{ir, physical::RelationPlan};
    let mut counts = vec![0usize; program.relations.len()];
    let mut uncorrelated = vec![false; counts.len()];
    for (owner, relation) in program.relations.iter().enumerate() {
        let mut refs = Vec::new();
        ir::references(&relation.expression, &mut refs);
        refs.extend(relation.domain);
        for reference in refs {
            counts[reference] += 1;
        }
        if relation.domain.is_none()
            && relation.keys.is_none()
            && let RelationPlan::Joins(arms) = &plans[owner]
        {
            for arm in arms {
                if let [(_, reference)] = arm.references.as_slice()
                    && *reference != owner
                    && !arm
                        .predicates
                        .iter()
                        .any(|p| matches!(p, Predicate::Region(..) | Predicate::Keys(..)))
                {
                    uncorrelated[*reference] = true;
                }
            }
        }
    }
    plans.iter().enumerate().map(|(relation, plan)| {
        let ordinary = matches!(plan, RelationPlan::Joins(arms) if arms.iter().all(|arm| !arm.references.iter().any(|(_,r)| *r==relation)))
            && program.relations[relation].domain.is_none() && program.relations[relation].keys.is_none();
        let inline = stats.known() && relation != 0 && ordinary && counts[relation] == 1 && uncorrelated[relation];
        StorageDecision { relation, consumers: counts[relation], materialized: {
                #[cfg(test)]
                { stats.force_materialization.unwrap_or(!inline) }
                #[cfg(not(test))]
                { !inline }
            },
            reason: if inline { "single uncorrelated IR consumer" } else if counts[relation]>1 { "multiple IR consumers" } else { "default or correlated/recursive boundary" } }
    }).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct EdgeDecision {
    pub edge_index: usize,
    pub from: usize,
    pub to: usize,
    pub known: usize,
    pub target: usize,
    pub observed_edges: u64,
    pub known_rows: u64,
}

impl PlannerStats {
    /// All candidates must have exact local kind metadata. Otherwise retain source order.
    /// The ratio estimates qualifying candidates, not utility work or semantic traversal order.
    pub(super) fn incident_edge(&self, arm: &Arm, known: &[bool]) -> Option<EdgeDecision> {
        use super::ir::Edge;
        fn bound(arm: &Arm, node: usize) -> Option<std::collections::BTreeSet<u16>> {
            arm.predicates
                .iter()
                .filter_map(|p| match p {
                    Predicate::Kinds(at, kinds) if *at == node => Some(
                        kinds
                            .iter()
                            .copied()
                            .collect::<std::collections::BTreeSet<_>>(),
                    ),
                    _ => None,
                })
                .reduce(|a, b| a.intersection(&b).copied().collect())
        }
        let mut candidates = Vec::new();
        for (edge_index, &(from, edge, to)) in arm
            .edges
            .iter()
            .enumerate()
            .filter(|(_, (from, _, to))| known[*from] != known[*to])
        {
            let (parent, child) = match edge {
                Edge::Child => (from, to),
                Edge::Parent => (to, from),
                _ => return None,
            };
            let parents = bound(arm, parent)?;
            let children = bound(arm, child)?;
            let mut observed_edges = 0u64;
            for p in &parents {
                for c in &children {
                    observed_edges = observed_edges.saturating_add(self.direct_edges(*p, *c)?);
                }
            }
            let (old, new) = if known[from] { (from, to) } else { (to, from) };
            let kinds = if old == parent { &parents } else { &children };
            let known_rows = self.kind_rows(&kinds.iter().copied().collect::<Vec<_>>())?;
            candidates.push(EdgeDecision {
                edge_index,
                from,
                to,
                known: old,
                target: new,
                observed_edges,
                known_rows,
            });
        }
        candidates.into_iter().min_by(|a, b| {
            (u128::from(a.observed_edges) * u128::from(b.known_rows.max(1)))
                .cmp(&(u128::from(b.observed_edges) * u128::from(a.known_rows.max(1))))
                .then(a.edge_index.cmp(&b.edge_index))
        })
    }
}
