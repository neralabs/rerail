//! Normalize exact kind Boolean expressions before constructing SQL row relations.
use super::{AnchorPlan, Condition, Lower, Set, decode::Property};
use std::collections::BTreeSet;
#[derive(Clone)]
pub(super) struct Domain {
    pub positive: bool,
    pub kinds: BTreeSet<u16>,
}
impl Domain {
    fn all() -> Self {
        Self {
            positive: false,
            kinds: BTreeSet::new(),
        }
    }
    fn finite(kinds: BTreeSet<u16>) -> Self {
        Self {
            positive: true,
            kinds,
        }
    }
    fn not(mut self) -> Self {
        self.positive = !self.positive;
        self
    }
    fn intersect(self, other: Self) -> Self {
        let kinds = match (self.positive, other.positive) {
            (true, true) => self.kinds.intersection(&other.kinds).copied().collect(),
            (true, false) => self.kinds.difference(&other.kinds).copied().collect(),
            (false, true) => other.kinds.difference(&self.kinds).copied().collect(),
            (false, false) => self.kinds.union(&other.kinds).copied().collect(),
        };
        Self {
            positive: self.positive || other.positive,
            kinds,
        }
    }
    fn union(self, other: Self) -> Self {
        self.not().intersect(other.not()).not()
    }
}
impl Lower {
    pub(super) fn alternative_kinds(&mut self, plans: &[AnchorPlan]) -> Option<BTreeSet<u16>> {
        let domain = plans
            .iter()
            .try_fold(Domain::finite(BTreeSet::new()), |domain, plan| {
                Some(domain.union(self.kind_domain(plan)?))
            })?;
        domain.positive.then_some(domain.kinds)
    }
    pub(super) fn kind_domain(&mut self, plan: &AnchorPlan) -> Option<Domain> {
        let key = |p: &AnchorPlan| p as *const AnchorPlan as usize;
        // Iterative postorder visits shared utilities once without adding Rust stack depth.
        let mut pending = vec![(plan, false)];
        while let Some((p, ready)) = pending.pop() {
            let id = key(p);
            if self.kind_alternatives.contains_key(&id) {
                continue;
            }
            if !p.exact {
                self.kind_alternatives.insert(id, None);
                continue;
            }
            if !ready {
                pending.push((p, true));
                match &p.condition {
                    Condition::All(plans) | Condition::Any(plans) => {
                        pending.extend(plans.iter().map(|p| (p, false)))
                    }
                    Condition::Not(inner) => pending.push((inner, false)),
                    Condition::Utility(inner) => pending.push((inner, false)),
                    _ => {}
                }
                continue;
            }
            let value = |p: &AnchorPlan| self.kind_alternatives[&key(p)].clone();
            let domain = match &p.condition {
                Condition::Always => Some(Domain::all()),
                Condition::All(plans) => plans
                    .iter()
                    .try_fold(Domain::all(), |d, p| Some(d.intersect(value(p)?))),
                Condition::Any(plans) => plans
                    .iter()
                    .try_fold(Domain::finite(BTreeSet::new()), |d, p| {
                        Some(d.union(value(p)?))
                    }),
                Condition::Not(inner) => value(inner).map(Domain::not),
                Condition::Utility(inner) => value(inner),
                _ => None,
            }
            .map(|d| match &p.root_kinds {
                Some(k) => d.intersect(Domain::finite(k.clone())),
                None => d,
            });
            self.kind_alternatives.insert(id, domain);
        }
        self.kind_alternatives[&key(plan)].clone()
    }
    pub(super) fn exclude_kinds(&mut self, scope: Set, kinds: &BTreeSet<u16>) -> Result<Set, ()> {
        if kinds.is_empty() {
            return Ok(scope);
        }
        if let Some(known) = self.reuse.kinds.get(&scope.name) {
            let allowed = known.difference(kinds).copied().collect();
            return self.restrict_kinds(scope, &allowed);
        }
        if scope.name == "all_nodes" {
            let one_zero = kinds.contains(&0) != kinds.contains(&u16::MAX);
            let mut excluded: BTreeSet<_> =
                kinds.iter().copied().filter(|k| *k != u16::MAX).collect();
            if kinds.contains(&u16::MAX) {
                excluded.insert(0);
            }
            let codes = excluded
                .iter()
                .map(|k| self.parameter(i64::from(*k)))
                .collect::<Vec<_>>()
                .join(",");
            let regular=self.relation(format!("SELECT k.fid,p.pre FROM cf CROSS JOIN file_kind_counts k ON k.fid=cf.fid CROSS JOIN nera_postings(k.postings) p WHERE k.code NOT IN ({codes})"),scope.exact)?;
            self.indexed.insert(regular.name.clone());
            if one_zero {
                let allowed = if kinds.contains(&0) { u16::MAX } else { 0 };
                let zero = self.kinds(&BTreeSet::from([allowed]))?;
                return self.union(&[regular, zero]);
            }
            return Ok(regular);
        }
        let properties = self.decoded(&scope, Property::Node);
        let codes = kinds
            .iter()
            .map(|k| self.parameter(i64::from(*k)))
            .collect::<Vec<_>>()
            .join(",");
        self.relation(
            format!("SELECT p.fid,p.pre FROM {properties} p WHERE p.kind NOT IN ({codes})"),
            scope.exact,
        )
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
