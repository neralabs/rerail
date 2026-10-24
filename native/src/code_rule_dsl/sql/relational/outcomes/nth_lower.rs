//! Sibling predicates use native An+B arithmetic and SQL-owned pure classifications.
use super::super::{Slots, effects::Effects};
use super::{Lower, Op, Plan};
use crate::code_rule_dsl::matcher::NthChild;

impl Lower {
    pub(super) fn nth_child(&mut self, nth: &NthChild) -> Result<Plan, String> {
        let of_rule = nth
            .of_rule
            .as_ref()
            .map(|rule| self.plan(rule).map(Box::new))
            .transpose()?;
        if of_rule.as_ref().is_some_and(|rule| !metadata(rule)) {
            return Err(
                "sibling index: ofRule requires a capture-free total node-local predicate".into(),
            );
        }
        let identity = std::ptr::from_ref(nth) as usize;
        let next = self.nths.len();
        let term = *self.nths.entry(identity).or_insert(next);
        let op = Op::NthChild {
            step: nth.position.step,
            offset: nth.position.offset,
            reverse: nth.reverse,
            of_rule,
            term,
            memoize: self.parsed_depth == 0,
        };
        Ok(Plan {
            effects: Effects::of(&op),
            op,
            mentions: Slots::new(),
            writes: Slots::new(),
            node_writes: Slots::new(),
            walk_filter: None,
        })
    }
}
fn metadata(plan: &Plan) -> bool {
    if plan.fallible() || !plan.mentions.is_empty() || !plan.writes.is_empty() {
        return false;
    }
    match &plan.op {
        Op::Predicate(_) => true,
        Op::NthChild { of_rule: None, .. } => true,
        Op::Not(child) => metadata(child),
        Op::Shared(child) => metadata(child),
        Op::All(items) | Op::Any(items) => items.iter().all(metadata),
        _ => false,
    }
}
