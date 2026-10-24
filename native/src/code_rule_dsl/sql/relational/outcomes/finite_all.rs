//! Remove only conjunction boundaries that cannot hide Unknown or withheld inputs.
use super::Lower;
use crate::code_rule_dsl::matcher::{self, AstMatcher, CaptureAnalysis};
use std::collections::BTreeSet;
impl Lower {
    pub(super) fn finite_all_items<'a>(&mut self, items: &'a [AstMatcher]) -> Vec<&'a AstMatcher> {
        if !self.finite_mode {
            return items.iter().collect();
        }
        fn append<'a>(
            items: &'a [AstMatcher],
            held: &mut BTreeSet<String>,
            analysis: &mut CaptureAnalysis,
            out: &mut Vec<&'a AstMatcher>,
            removed: &mut usize,
        ) {
            for child in items {
                let mentions = analysis.mentions(child);
                let skipped = !mentions.is_disjoint(held);
                let unknown = matcher::reads_values(child, &mut BTreeSet::new());
                if let AstMatcher::All(nested) = child
                    && !unknown
                    && !skipped
                {
                    *removed += 1;
                    append(nested, held, analysis, out, removed);
                } else {
                    out.push(child);
                }
                // Skipped total terms propagate withheld captures too. Without
                // this, flattening could reach a utility native never enters.
                if unknown || skipped {
                    held.extend(analysis.binds(child));
                }
            }
        }
        let mut out = vec![];
        let mut removed = 0;
        append(
            items,
            &mut BTreeSet::new(),
            &mut CaptureAnalysis::default(),
            &mut out,
            &mut removed,
        );
        self.finite_flattened += removed;
        out
    }
}
