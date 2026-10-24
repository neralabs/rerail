//! Pattern leaves return typed witness columns; composition remains in the outcome plan.
use super::super::Op;
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
        if term.vars.iter().any(|(_, kind)| *kind != SlotKind::Node) {
            return Err("outcome pattern: list captures require typed sequence columns".into());
        }
        let slots = term
            .vars
            .iter()
            .map(|(name, _)| self.slot(name, false))
            .collect::<Result<Vec<_>, _>>()?;
        term.reads = (0..term.vars.len()).collect();
        let kinds = term.kinds.clone();
        let id = self.pure.program.terminals.len();
        self.pure.program.terminals.push(Terminal::Pattern(term));
        if slots.is_empty() {
            return Ok((
                Op::Predicate(super::super::super::ir::Set::Pattern(id)),
                slots,
            ));
        }
        Ok((Op::Pattern(id, slots.clone(), kinds), slots))
    }
}
