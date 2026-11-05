//! Preserve diagnostic anchors without retaining complete input captures during SQL execution.
use super::{Plan, Rows};
use crate::code_rule_dsl::violations::{Span, anchored_span};
use std::{collections::HashMap, sync::Arc};

pub(super) type Anchors = HashMap<Arc<str>, HashMap<(usize, usize), (Span, bool)>>;
pub(super) fn canonical(anchors: &mut Anchors, file: &crate::code_rule_dsl::model::FileInput, spans: &[(u32,u32)], work: &std::sync::atomic::AtomicUsize) -> super::Result<()> {
    let Some(anchors) = anchors.get_mut(file.path.as_str()) else { return Ok(()); };
    let Some((_,ignored)) = anchors.get(&(0,file.source.len())).copied() else { return Ok(()); };
    let lines = crate::code_rule_dsl::lines::LineTable::new(&file.source, crate::code_rule_dsl::lines::Terminators::Newlines);
    for &(start,end) in spans {
        super::source::charge(work,1)?;
        anchors.insert((start as usize,end as usize),(lines.span(start as usize,end as usize),ignored));
    }
    Ok(())
}
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
