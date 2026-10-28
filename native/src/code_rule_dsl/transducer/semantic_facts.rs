//! Immutable rows produced for semantic predicates before SQL composition.
//!
//! A producer reports only predicate-local facts. It never executes a nested user rule. SQL joins
//! the rows to separately lowered relational operands.

use crate::code_rule_dsl::literal::SubTree;
use crate::code_rule_dsl::scope::ScopeTable;
use ast_grep_core::{Doc, Node};
use std::sync::Arc;

pub(crate) mod slot {
    pub(crate) const GATE: u16 = 0;
    pub(crate) const SITE: u16 = 1;
    pub(crate) const VALUE: u16 = 2;
    pub(crate) const ANNOTATION: u16 = 3;
    pub(crate) const OCCURRENCE: u16 = 4;
    pub(crate) const SHADOWED_SITE: u16 = 5;
    pub(crate) const OTHERS: u16 = 8;
    pub(crate) const OTHER_SITE: u16 = 9;
    pub(crate) const UNUSED: u16 = 12;
    pub(crate) const MISSING: u16 = 13;
    pub(crate) const INCOMPLETE: u16 = 14;
    pub(crate) const ARGUMENT: u16 = 15;
    pub(crate) const CALL_TARGET: u16 = 16;
    pub(crate) const INPUT_BINDING: u16 = 17;
    pub(crate) const OTHER_BITS: u32 = 20;
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BindingRequest {
    pub(crate) site: bool,
    pub(crate) value: bool,
    pub(crate) annotation: bool,
    pub(crate) occurrence: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RedefinesRequest {
    pub(crate) site: bool,
    pub(crate) value: bool,
    pub(crate) annotation: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BindsRequest {
    pub(crate) binding: BindingRequest,
    pub(crate) redefines: Option<RedefinesRequest>,
}

pub(crate) enum Target<'tree, D: Doc> {
    None,
    Node(Node<'tree, D>),
    Tree(Arc<SubTree>, Option<Arc<ScopeTable>>),
    TreeNode(Arc<SubTree>, usize),
}

pub(crate) struct Fact<'tree, D: Doc> {
    pub(crate) slot: u16,
    pub(crate) key: i64,
    pub(crate) pos: i64,
    pub(crate) flag: i64,
    pub(crate) target: Target<'tree, D>,
}

pub(crate) struct Facts<'tree, D: Doc> {
    pub(crate) rows: Vec<Fact<'tree, D>>,
}

impl<'tree, D: Doc> Facts<'tree, D> {
    pub(crate) fn new() -> Self {
        Self { rows: Vec::new() }
    }

    pub(crate) fn push(&mut self, slot: u16, key: i64, flag: i64, target: Target<'tree, D>) -> i64 {
        let pos = self
            .rows
            .iter()
            .filter(|row| row.slot == slot && row.key == key)
            .count() as i64;
        self.rows.push(Fact {
            slot,
            key,
            pos,
            flag,
            target,
        });
        pos
    }

    pub(crate) fn gate(&mut self, flag: i64, target: Target<'tree, D>) -> i64 {
        self.push(slot::GATE, 0, flag, target)
    }
}
