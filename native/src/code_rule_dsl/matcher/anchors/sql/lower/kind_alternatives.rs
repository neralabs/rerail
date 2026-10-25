//! Fold exact finite kind alternatives without constructing their SQL relations.
use super::{AnchorPlan, Condition, Lower};
use std::collections::BTreeSet;

impl Lower {
    pub(super) fn alternative_kinds(&mut self, plans: &[AnchorPlan]) -> Option<BTreeSet<u16>> {
        plans.iter().try_fold(BTreeSet::new(), |mut kinds, plan| {
            kinds.extend(self.finite_kinds(plan)?);
            Some(kinds)
        })
    }

    fn finite_kinds(&mut self, plan: &AnchorPlan) -> Option<BTreeSet<u16>> {
        let key = plan as *const AnchorPlan as usize;
        if let Some(kinds) = self.kind_alternatives.get(&key) {
            return kinds.clone();
        }
        let kinds = if !plan.exact {
            None
        } else {
            let kinds = match &plan.condition {
                Condition::Always => plan.root_kinds.clone(),
                Condition::Any(plans) => self.alternative_kinds(plans),
                Condition::Utility(inner) => self.finite_kinds(inner),
                // A root-kind bound alone does not establish equivalence to a
                // pattern, text predicate, relation, or negated predicate.
                _ => None,
            };
            kinds.map(|kinds| match &plan.root_kinds {
                Some(required) => kinds.intersection(required).copied().collect(),
                None => kinds,
            })
        };
        self.kind_alternatives.insert(key, kinds.clone());
        kinds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_bound_does_not_make_an_approximation_exact() {
        let mut lower = Lower::new(1);
        let plan = AnchorPlan {
            root_kinds: Some([17].into()),
            condition: Condition::Always,
            exact: false,
        };
        assert_eq!(lower.alternative_kinds(&[plan]), None);
    }

    #[test]
    fn enclosing_kind_restrictions_apply_to_shared_utility_definitions() {
        let mut lower = Lower::new(1);
        let plan = AnchorPlan {
            root_kinds: Some([2, 3].into()),
            condition: Condition::Utility(std::sync::Arc::new(AnchorPlan {
                root_kinds: Some([1, 2].into()),
                condition: Condition::Always,
                exact: true,
            })),
            exact: true,
        };
        assert_eq!(lower.alternative_kinds(&[plan]), Some([2].into()));
    }
}
