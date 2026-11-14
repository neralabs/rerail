use super::identity::{ClausePlanId, DocumentMapGenerationId, EffectProfileId, ScopeInstanceId};
use super::scope::CandidateScope;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum FailureIsolationMode {
    PerFile,
    #[cfg(test)]
    AtomicGroup,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ClauseScope {
    clause: ClausePlanId,
    scope: CandidateScope,
    effect_profile: EffectProfileId,
    document_map_generation: DocumentMapGenerationId,
    failure_isolation: FailureIsolationMode,
}

impl ClauseScope {
    pub(super) fn new(
        clause: ClausePlanId,
        scope: CandidateScope,
        effect_profile: EffectProfileId,
        document_map_generation: DocumentMapGenerationId,
        failure_isolation: FailureIsolationMode,
    ) -> Self {
        Self {
            clause,
            scope,
            effect_profile,
            document_map_generation,
            failure_isolation,
        }
    }

    #[cfg(test)]
    pub(super) fn clause(&self) -> ClausePlanId {
        self.clause
    }

    #[cfg(test)]
    pub(super) fn effect_profile(&self) -> EffectProfileId {
        self.effect_profile
    }

    #[cfg(test)]
    pub(super) fn document_map_generation(&self) -> DocumentMapGenerationId {
        self.document_map_generation
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ExactGroupKey {
    scope_id: ScopeInstanceId,
    scope: CandidateScope,
    effect_profile: EffectProfileId,
    document_map_generation: DocumentMapGenerationId,
    failure_isolation: FailureIsolationMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ExecutionGroup {
    key: ExactGroupKey,
    clauses: Vec<ClausePlanId>,
}

impl ExecutionGroup {
    #[cfg(test)]
    pub(super) fn scope(&self) -> &CandidateScope {
        &self.key.scope
    }

    pub(super) fn clauses(&self) -> &[ClausePlanId] {
        &self.clauses
    }
}

pub(super) fn group_exact_scopes(assignments: Vec<ClauseScope>) -> Vec<ExecutionGroup> {
    let mut grouped = BTreeMap::<ExactGroupKey, Vec<ClausePlanId>>::new();
    for assignment in assignments {
        let key = ExactGroupKey {
            scope_id: assignment.scope.id(),
            scope: assignment.scope,
            effect_profile: assignment.effect_profile,
            document_map_generation: assignment.document_map_generation,
            failure_isolation: assignment.failure_isolation,
        };
        grouped.entry(key).or_default().push(assignment.clause);
    }
    grouped
        .into_iter()
        .map(|(key, mut clauses)| {
            clauses.sort_unstable();
            ExecutionGroup { key, clauses }
        })
        .collect()
}
