//! Benchmark-only first-candidate probes. Capture-dependent conditions keep their original order
//! and batched terminal evaluation.

use super::{Emitter, Live, Op, Reads, Walk};
use crate::code_rule_dsl::sql::plan::Stop;

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
