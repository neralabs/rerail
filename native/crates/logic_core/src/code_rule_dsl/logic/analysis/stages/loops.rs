//! Monotonicity of a component and its auxiliary stages, independent of an executor.
use super::super::Decision;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, serde::Serialize)]
pub struct Stage {
    pub reads: BTreeSet<String>,
    pub scalar_reads: BTreeSet<String>,
    pub aggregate: bool,
    pub total: bool,
}
impl Default for Stage {
    fn default() -> Self {
        Self {
            reads: BTreeSet::new(),
            scalar_reads: BTreeSet::new(),
            aggregate: false,
            total: true,
        }
    }
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct Analysis {
    pub set_outputs: bool,
    pub checked_rules: bool,
    pub component_dependent_stages: BTreeSet<String>,
    pub nonmonotone_stages: BTreeSet<String>,
    pub effect_boundaries: BTreeSet<String>,
    pub stages: BTreeMap<String, Stage>,
    pub decision: Decision,
}
impl Analysis {
    /// Consolidation should reduce active feedback caches, rather than merely
    /// exchange one frontier cache for a growing full-context cache.
    pub fn feedback_cache_savings(&self, recursive_full_rules: usize, delta_variants: usize) -> Decision {
        Decision::new(delta_variants > recursive_full_rules, if delta_variants > recursive_full_rules {
            "Full-rule contexts combine multiple active generated feedback caches"
        } else {
            "Consolidation does not reduce active feedback caches; retain specialized frontier contexts"
        })
    }
    /// A retained positive context already differentiates each recursive input.
    /// A second cache for an emitted delta variant duplicates that work.
    pub fn retained_full_rules(&self, recursive_contexts: bool) -> Decision {
        if !self.decision.proven {
            return self.decision.clone();
        }
        Decision::new(
            recursive_contexts,
            if recursive_contexts {
                "Retained full-rule contexts consume recursive input deltas; generated delta-variant caches are redundant"
            } else {
                "A recursive auxiliary query still requires its specialized delta kernel"
            },
        )
    }
}
pub fn analyze(
    recursive_sources: &BTreeSet<String>,
    set_outputs: bool,
    checked_rules: bool,
    stages: BTreeMap<String, Stage>,
) -> Analysis {
    let mut affected = recursive_sources.clone();
    loop {
        let before = affected.len();
        for (name, stage) in &stages {
            if !stage.reads.is_disjoint(&affected) {
                affected.insert(name.clone());
            }
        }
        if affected.len() == before {
            break;
        }
    }
    let nonmonotone_stages = stages
        .iter()
        .filter(|(_, stage)| {
            !stage.scalar_reads.is_disjoint(&affected)
                || stage.aggregate && !stage.reads.is_disjoint(&affected)
        })
        .map(|(name, _)| name.clone())
        .collect::<BTreeSet<_>>();
    let effect_boundaries = stages
        .iter()
        .filter(|(_, stage)| !stage.total)
        .map(|(name, _)| name.clone())
        .collect::<BTreeSet<_>>();
    let reason = if !set_outputs {
        Some("Lattice merges and widening retain their existing schedule")
    } else if !checked_rules {
        Some("Opaque steps retain their dependency gates")
    } else if !nonmonotone_stages.is_empty() {
        Some(
            "Component-dependent scalars, negative reads, or aggregates require retained cursor semantics",
        )
    } else if !effect_boundaries.is_empty() {
        Some("Checked or opaque expressions retain their evaluation order")
    } else {
        None
    };
    Analysis {set_outputs,checked_rules,component_dependent_stages:affected.into_iter().filter(|name|stages.contains_key(name)).collect(),
        nonmonotone_stages,effect_boundaries,stages,decision:Decision::new(reason.is_none(),reason.unwrap_or("Set component and auxiliary stages are monotone in recursive inputs and contain only total expressions"))}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auxiliary_cycles_propagate_recursive_dependence_and_reject_nonmonotone_consumers() {
        let members = BTreeSet::from(["r".into()]);
        let mut stages = BTreeMap::from([
            (
                "seed".into(),
                Stage {
                    reads: BTreeSet::from(["r".into()]),
                    ..Stage::default()
                },
            ),
            (
                "walk".into(),
                Stage {
                    reads: BTreeSet::from(["seed".into(), "walk".into()]),
                    ..Stage::default()
                },
            ),
            (
                "out".into(),
                Stage {
                    reads: BTreeSet::from(["walk".into()]),
                    ..Stage::default()
                },
            ),
        ]);
        let positive = analyze(&members, true, true, stages.clone());
        assert!(positive.decision.proven);
        assert!(positive.retained_full_rules(true).proven);
        assert!(!positive.retained_full_rules(false).proven);
        assert!(!positive.feedback_cache_savings(1, 1).proven);
        assert!(positive.feedback_cache_savings(1, 2).proven);
        assert_eq!(positive.component_dependent_stages.len(), 3);
        stages
            .get_mut("out")
            .unwrap()
            .scalar_reads
            .insert("walk".into());
        assert_eq!(
            analyze(&members, true, true, stages.clone()).nonmonotone_stages,
            BTreeSet::from(["out".into()])
        );
        stages.get_mut("out").unwrap().scalar_reads.clear();
        stages.get_mut("out").unwrap().aggregate = true;
        assert!(
            !analyze(&members, true, true, stages.clone())
                .decision
                .proven
        );
        stages.get_mut("out").unwrap().reads = BTreeSet::from(["external".into()]);
        assert!(
            analyze(&members, true, true, stages.clone())
                .decision
                .proven
        );
        assert!(
            !analyze(&members, false, true, stages.clone())
                .decision
                .proven
        );
        assert!(!analyze(&members, true, false, stages).decision.proven);
    }
}
