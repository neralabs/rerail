//! Argument slot matching is atomic; its capture transport is explicit and typed.
use super::super::{CaptureKind, Op, Plan, Slots, effects::Effects};
use super::Lower;
use crate::code_rule_dsl::{
    matcher::{AstMatcher, Semantic},
    sql::{
        plan::SlotKind,
        relational::bound_leaf::OwnedBoundLeaf,
        terminals::{FactTerm, Terminal},
    },
};

impl Lower {
    pub(super) fn arguments(&mut self, matcher: &AstMatcher) -> Result<Plan, String> {
        let AstMatcher::Semantic(Semantic::Arguments(test)) = matcher else {
            unreachable!("argument lowering requires arguments")
        };
        // Native matching retains its binding order. Transport columns only need
        // one entry per capture and are also inputs for existing-binding unification.
        let mut bindings = Vec::new();
        let mut inputs = Vec::new();
        for (name, _) in &test.bind {
            if bindings.iter().any(|(bound, _)| bound == name) {
                continue;
            }
            inputs.push(self.slot(name, CaptureKind::Node)?);
            bindings.push((name.clone(), SlotKind::Node));
        }
        let term = self.pure.program.terminals.len();
        self.pure.program.terminals.push(Terminal::Fact(FactTerm {
            reads: bindings.clone(),
            verdicts: Box::new(OwnedBoundLeaf {
                leaf: matcher.clone(),
                bindings,
                owner: self.scope_owner,
            }),
        }));
        let slots: Slots = inputs.iter().copied().collect();
        let outputs = inputs.clone();
        Ok(Plan {
            op: Op::BoundLeaf {
                term,
                inputs,
                outputs,
                kinds: Some(test.call_kinds.iter().copied().collect()),
            },
            effects: Effects {
                reads_value: true,
                fallible: true,
                cache_fills: true,
                typed_exports: true,
                ..Effects::default()
            },
            mentions: slots.clone(),
            writes: slots.clone(),
            node_writes: slots,
            walk_filter: None,
        })
    }
}
