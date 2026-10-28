//! Preserve diagnostic anchors without retaining complete input captures during SQL execution.
use super::{Plan, Rows};
use crate::code_rule_dsl::violations::{Span, anchored_span};
use std::{collections::HashMap, sync::Arc};

pub(super) type Anchors = HashMap<Arc<str>, HashMap<(usize, usize), (Span, bool)>>;
pub(super) fn collect(plan: &Plan, rows: &[Rows]) -> Anchors {
    let mut anchors = Anchors::new();
    for ((_, input), rows) in plan.inputs.iter().zip(rows) {
        for witness in &rows.witnesses {
            let span = anchored_span(&input.anchor, witness).0;
            let key = (span.start_byte, span.end_byte);
            let value = (span, witness.ignored);
            if let Some(by_span) = anchors.get_mut(witness.path.as_str()) {
                by_span.insert(key, value);
            } else {
                anchors.insert(
                    Arc::from(witness.path.as_str()),
                    HashMap::from([(key, value)]),
                );
            }
        }
    }
    anchors
}
