//! Benchmark-only compiler experiments: conservative root restrictions and bounded first-candidate
//! probes. Capture-dependent conditions keep their original order and batched terminal evaluation.

use super::{Emitter, Live, Op, Reads, Walk, list};
use crate::code_rule_dsl::sql::plan::{Stop, Unit};
use std::collections::HashSet;

impl Emitter<'_> {
    /// Test one candidate per root in a batch, then run the original search only for roots
    /// whose probe failed. Pattern and regex terminals still receive a complete input set.
    pub(super) fn try_first_probe(&mut self, op: &Op, input: &str, live: &Live) -> Option<String> {
        fn simple(op: &Op) -> bool {
            match op {
                Op::Kinds(_) | Op::Regex(_) | Op::Pattern { .. } | Op::Never => true,
                Op::All(items) | Op::Any(items) => items.iter().all(simple),
                Op::Not(inner) => simple(inner),
                _ => false,
            }
        }
        let (walk, sub, strict) = match op {
            Op::Relation { walk, sub, .. } => (*walk, sub, true),
            Op::Search {
                walk, sub, strict, ..
            } => (*walk, sub, *strict),
            _ => return None,
        };
        if self.first_probe_active || !super::super::diagnostics::first_probe() {
            return None;
        }
        if !matches!(
            walk,
            Walk::Has(Stop::End)
                | Walk::Inside(Stop::End)
                | Walk::Precedes(Stop::End)
                | Walk::Follows(Stop::End)
        ) {
            return None;
        }
        if !super::super::diagnostics::first_probe_all()
            && (self.three || !simple(sub) || !matches!(walk, Walk::Has(Stop::End)))
        {
            return None;
        }
        self.first_probe_active = true;
        self.first_candidate = true;
        let probed = self.op(op, input, live);
        debug_assert!(!self.first_candidate);
        // A non-strict unknown does not end a search. Its root falls back to the original
        // complete search, which includes that unknown candidate and preserves its verdict.
        let probe = if self.three && !strict {
            self.frame(
                "x",
                &format!(
                    "SELECT {} FROM {probed} f WHERE f.unk = 0",
                    self.verdict("f", &[], "f.unk")
                ),
                Reads::join(&[&probed]),
            )
        } else {
            probed
        };
        let held = self.live_before(op, live);
        let rest = self.within(&held, |this| {
            this.frame(
                "y",
                &format!(
                    "SELECT {} FROM {input} f WHERE f.k NOT IN (SELECT k FROM {probe})",
                    this.cols("f")
                ),
                Reads::anti(input, &[&probe]),
            )
        });
        self.inherit(input, &rest, &Default::default());
        self.same_space(input, &rest);
        let fallback = self.op(op, &rest, live);
        self.first_probe_active = false;
        let selected = if self.three {
            self.verdict("f", &[], "f.unk")
        } else {
            self.cols("f")
        };
        Some(self.frame(
            "x",
            &format!(
                "SELECT {} FROM {probe} f UNION ALL SELECT {} FROM {fallback} f",
                selected, selected
            ),
            Reads::union(&[&probe, &fallback]),
        ))
    }
}

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
