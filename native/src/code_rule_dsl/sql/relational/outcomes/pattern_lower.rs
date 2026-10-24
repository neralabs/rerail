//! Pattern leaves return typed witness columns; composition remains in the outcome plan.
use super::super::{CaptureKind, Op};
use super::Lower;
use crate::code_rule_dsl::sql::{
    plan::SlotKind,
    terminals::{PatternTerm, Terminal},
};
use ast_grep_core::matcher::Pattern;
use std::sync::Arc;

impl Lower {
    pub(super) fn pattern(&mut self, pattern: &Arc<Pattern>) -> Result<(Op, Vec<usize>), String> {
        let mut term = PatternTerm::new(pattern.clone(), self.language);
        let slots = term
            .vars
            .iter()
            .map(|(name, kind)| {
                self.slot(
                    name,
                    match kind {
                        SlotKind::Node => CaptureKind::Node,
                        SlotKind::List => CaptureKind::List,
                        _ => unreachable!("patterns bind nodes or lists"),
                    },
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        term.reads = (0..term.vars.len()).collect();
        let kinds = term.kinds.clone();
        let id = self.pure.program.terminals.len();
        self.pure.program.terminals.push(Terminal::Pattern(term));
        if slots.is_empty() {
            use super::super::super::ir::Set;
            // Keep the native necessary-kind proof even when no captures cross
            // the leaf boundary. Ordered alternatives need every arm's proof.
            let predicate = match kinds {
                Some(kinds) => Set::Intersect(vec![Set::Kinds(kinds), Set::Pattern(id)]),
                None => Set::Pattern(id),
            };
            return Ok((Op::Predicate(predicate), slots));
        }
        Ok((Op::Pattern(id, slots.clone(), kinds), slots))
    }
}
