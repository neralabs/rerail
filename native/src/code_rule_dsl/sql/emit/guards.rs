//! Proven structural restrictions applied before allocating capture-bearing root frames.

use super::{Op, Walk, list};
use crate::code_rule_dsl::sql::plan::Unit;
use std::collections::HashSet;

pub(super) fn root_guard(op: &Op, units: &[Unit]) -> Option<String> {
    fn guard(
        op: &Op,
        units: &[Unit],
        visiting: &mut HashSet<usize>,
        budget: &mut usize,
    ) -> Option<String> {
        *budget = budget.checked_sub(1)?;
        match op {
            Op::All(items) | Op::Seq(items) => {
                let predicates: Vec<_> = items
                    .iter()
                    .filter_map(|op| guard(op, units, visiting, budget))
                    .collect();
                (!predicates.is_empty()).then(|| format!("({})", predicates.join(" AND ")))
            }
            Op::Any(items) => {
                let predicates = items
                    .iter()
                    .map(|op| guard(op, units, visiting, budget))
                    .collect::<Option<Vec<_>>>()?;
                (!predicates.is_empty()).then(|| format!("({})", predicates.join(" OR ")))
            }
            Op::Relation { walk, need, .. } => {
                let kinds = need.kinds.as_ref()?;
                if kinds.is_empty() {
                    return Some("0".into());
                }
                let bounds = match walk {
                    Walk::Inside(_) | Walk::FieldInside(..) => "g.id < n.id AND g.\"last\" >= n.id",
                    Walk::Has(_) | Walk::FieldHas(..) => "g.id > n.id AND g.id <= n.\"last\"",
                    _ => return None,
                };
                // Ignoring field and stop restrictions widens the candidate set, so every
                // actual match passes. `node` recognizes the ancestor interval as a walk.
                Some(format!(
                    "EXISTS (SELECT 1 FROM node g WHERE g.file = n.file AND {bounds} AND g.kind IN ({}))",
                    list(kinds)
                ))
            }
            Op::Recur { unit, .. } => {
                if !visiting.insert(*unit) {
                    return None;
                }
                let predicate = guard(&units[*unit].body, units, visiting, budget);
                visiting.remove(unit);
                predicate
            }
            Op::Call { body, .. } => guard(body, units, visiting, budget),
            Op::Constrained { rule, .. } => guard(rule, units, visiting, budget),
            Op::Not(inner) => match &**inner {
                Op::Not(inner) => guard(inner, units, visiting, budget),
                _ => None,
            },
            // Unknown results, capture-dependent tests, and arbitrary terminal effects do not
            // prove a capture-free restriction. This pass runs only on two-valued clauses.
            _ => None,
        }
    }
    guard(op, units, &mut HashSet::new(), &mut 64)
}
