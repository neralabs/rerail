//! Read-only atomic predicates receive nullable node captures, never child programs.
use super::super::{CaptureKind, Op, Plan, Slots, effects::Effects};
use super::Lower;
use crate::code_rule_dsl::{
    matcher::{self, AstMatcher, Semantic},
    sql::{
        plan::SlotKind,
        relational::bound_leaf::OwnedBoundLeaf,
        terminals::{FactTerm, Terminal},
    },
};

impl Lower {
    pub(super) fn capture_predicate(&mut self, leaf: &AstMatcher) -> Result<Plan, String> {
        let names: Vec<&str> = match leaf {
            AstMatcher::Semantic(Semantic::Layout(layout)) => layout.reads().collect(),
            AstMatcher::Compare(comparison) => {
                if comparison.transform.is_some() {
                    return Err(
                        "outcome comparison: transformed inputs require a typed text contract"
                            .into(),
                    );
                }
                vec![&comparison.left, &comparison.right]
            }
            _ => {
                return Err(
                    "outcome capture predicate: atomic layout or comparison required".into(),
                );
            }
        };
        let mut bindings = Vec::new();
        let mut inputs = Vec::new();
        for name in names {
            if bindings.iter().any(|(bound, _)| bound == name) {
                continue;
            }
            inputs.push(self.slot(name, CaptureKind::Node)?);
            bindings.push((name.to_owned(), SlotKind::Node));
        }
        let mentions: Slots = inputs.iter().copied().collect();
        let term = self.pure.program.terminals.len();
        self.pure.program.terminals.push(Terminal::Fact(FactTerm {
            reads: bindings.clone(),
            verdicts: Box::new(OwnedBoundLeaf {
                leaf: leaf.clone(),
                bindings,
                owner: self.scope_owner,
            }),
        }));
        Ok(Plan {
            op: Op::BoundLeaf {
                term,
                inputs,
                outputs: vec![],
                kinds: None,
            },
            effects: Effects {
                reads_value: matcher::reads_values(leaf, &mut std::collections::BTreeSet::new()),
                fallible: true,
                cache_fills: true,
                // Predicates export nothing; typed_exports remains false.
                ..Effects::default()
            },
            mentions,
            writes: Slots::new(),
            node_writes: Slots::new(),
            walk_filter: None,
        })
    }
}
