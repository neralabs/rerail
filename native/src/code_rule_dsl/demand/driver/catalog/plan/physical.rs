use super::identity::UtilityDefinitionId;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl::demand::driver::catalog) enum FactFamilyContract {
    Semantic,
    Value,
    Parse,
    Transform,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl::demand::driver::catalog) struct StatementRelations {
    fact_families: BTreeSet<FactFamilyContract>,
    utility_definitions: BTreeSet<UtilityDefinitionId>,
}

impl StatementRelations {
    pub(in crate::code_rule_dsl::demand::driver::catalog) fn from_fact_families(
        families: impl IntoIterator<Item = FactFamilyContract>,
    ) -> Self {
        let mut fact_families = families.into_iter().collect::<BTreeSet<_>>();
        if fact_families.is_empty() {
            fact_families.insert(FactFamilyContract::Semantic);
        }
        Self {
            fact_families,
            utility_definitions: BTreeSet::new(),
        }
    }

    pub(in crate::code_rule_dsl::demand::driver::catalog) fn with_utility_definitions(
        mut self,
        definitions: impl IntoIterator<Item = UtilityDefinitionId>,
    ) -> Self {
        self.utility_definitions.extend(definitions);
        self
    }

    pub(in crate::code_rule_dsl::demand::driver::catalog) fn utility_definitions(
        &self,
    ) -> impl Iterator<Item = UtilityDefinitionId> + '_ {
        self.utility_definitions.iter().copied()
    }

    pub(in crate::code_rule_dsl::demand::driver::catalog) fn fact_families(
        &self,
    ) -> impl Iterator<Item = FactFamilyContract> + '_ {
        self.fact_families.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::{FactFamilyContract, StatementRelations};

    #[test]
    fn statement_relations_are_typed_deduplicated_and_deterministic() {
        let relations = StatementRelations::from_fact_families([
            FactFamilyContract::Transform,
            FactFamilyContract::Value,
            FactFamilyContract::Transform,
        ]);
        assert_eq!(
            relations.fact_families().collect::<Vec<_>>(),
            [FactFamilyContract::Value, FactFamilyContract::Transform]
        );
        assert_eq!(
            StatementRelations::from_fact_families([])
                .fact_families()
                .collect::<Vec<_>>(),
            [FactFamilyContract::Semantic]
        );
    }
}
