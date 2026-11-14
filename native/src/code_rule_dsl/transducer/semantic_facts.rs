//! Immutable rows produced for semantic predicates before SQL composition.
//!
//! A producer reports only predicate-local facts. It never executes a nested user rule. SQL joins
//! the rows to separately lowered relational operands.

use crate::code_rule_dsl::literal::SubTree;
use crate::code_rule_dsl::scope::ScopeTable;
use ast_grep_core::{Doc, Node};
use std::sync::Arc;

pub mod slot {
    pub const GATE: u16 = 0;
    pub const SITE: u16 = 1;
    pub const VALUE: u16 = 2;
    pub const ANNOTATION: u16 = 3;
    pub const OCCURRENCE: u16 = 4;
    pub const SHADOWED_SITE: u16 = 5;
    pub(crate) const OTHERS: u16 = 8;
    pub const OTHER_SITE: u16 = 9;
    pub const UNUSED: u16 = 12;
    pub const MISSING: u16 = 13;
    pub(crate) const INCOMPLETE: u16 = 14;
    pub const ARGUMENT: u16 = 15;
    pub const CALL_TARGET: u16 = 16;
    pub const INPUT_BINDING: u16 = 17;
    pub(crate) const OTHER_BITS: u32 = 20;
}

#[derive(Clone, Copy, Debug)]
pub struct BindingRequest {
    pub site: bool,
    pub value: bool,
    pub annotation: bool,
    pub occurrence: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct RedefinesRequest {
    pub site: bool,
    pub value: bool,
    pub annotation: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct BindsRequest {
    pub binding: BindingRequest,
    pub redefines: Option<RedefinesRequest>,
}

pub enum Target<'tree, D: Doc> {
    None,
    Node(Node<'tree, D>),
    Tree(Arc<SubTree>, Option<Arc<ScopeTable>>),
    TreeNode(Arc<SubTree>, usize),
}

pub struct Fact<'tree, D: Doc> {
    pub slot: u16,
    pub key: i64,
    pub pos: i64,
    pub flag: i64,
    pub target: Target<'tree, D>,
}

pub struct Facts<'tree, D: Doc> {
    pub rows: Vec<Fact<'tree, D>>,
}

impl<'tree, D: Doc> Facts<'tree, D> {
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }

    pub fn push(&mut self, slot: u16, key: i64, flag: i64, target: Target<'tree, D>) -> i64 {
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

    pub fn gate(&mut self, flag: i64, target: Target<'tree, D>) -> i64 {
        self.push(slot::GATE, 0, flag, target)
    }
}
