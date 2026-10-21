//! Root guards: a two-valued statement's root frame keeps only the roots where the rule can
//! hold, by a capture-free necessary condition of its relations: a relation from the root whose
//! candidates must be of some kinds (`Need::kinds`) needs a node of those kinds among the root's
//! descendants (`has`) or ancestors (`inside`). Widening a walk to every descendant or ancestor,
//! whatever its field or stop, keeps every root the rule holds at; the later operators decide.

use super::{Op, Walk, list};
use crate::code_rule_dsl::sql::plan::Unit;
use std::collections::HashSet;

/// The most operators a guard's search visits.
const BUDGET: usize = 64;

/// A condition on the root node `n` that holds wherever `op` (two-valued) holds at it, if the
/// operator says one.
pub(super) fn root_guard(op: &Op, units: &[Unit]) -> Option<String> {
    let mut budget = BUDGET;
    guard(op, units, &mut HashSet::new(), &mut budget)
}

fn guard(
    op: &Op,
    units: &[Unit],
    visiting: &mut HashSet<usize>,
    budget: &mut usize,
) -> Option<String> {
    *budget = budget.checked_sub(1)?;
    match op {
        // Each term holds where the conjunction does.
        Op::All(items) | Op::Seq(items) => {
            let terms: Vec<String> = items
                .iter()
                .filter_map(|op| guard(op, units, visiting, budget))
                .collect();
            (!terms.is_empty()).then(|| format!("({})", terms.join(" AND ")))
        }
        // Some branch holds where the disjunction does: each needs a guard.
        Op::Any(items) => {
            let branches = items
                .iter()
                .map(|op| guard(op, units, visiting, budget))
                .collect::<Option<Vec<_>>>()?;
            (!branches.is_empty()).then(|| format!("({})", branches.join(" OR ")))
        }
        Op::Relation { walk, need, .. } => {
            let kinds = need.kinds.as_ref()?;
            if kinds.is_empty() {
                return Some("0".to_owned());
            }
            let walked = match walk {
                Walk::Has(_) | Walk::FieldHas(..) => "g.id > n.id AND g.id <= n.\"last\"",
                Walk::Inside(_) | Walk::FieldInside(..) => "g.id < n.id AND g.\"last\" >= n.id",
                _ => return None,
            };
            Some(format!(
                "EXISTS (SELECT 1 FROM node g WHERE g.file = n.file AND {walked} AND g.kind IN ({}))",
                list(kinds)
            ))
        }
        // A unit's answers at the root are its body's verdicts there.
        Op::Recur { unit, .. } => {
            if !visiting.insert(*unit) {
                return None;
            }
            let found = guard(&units[*unit].body, units, visiting, budget);
            visiting.remove(unit);
            found
        }
        Op::Call { body, .. } => guard(body, units, visiting, budget),
        Op::Constrained { rule, .. } => guard(rule, units, visiting, budget),
        Op::Not(inner) => match &**inner {
            Op::Not(inner) => guard(inner, units, visiting, budget),
            _ => None,
        },
        _ => None,
    }
}
