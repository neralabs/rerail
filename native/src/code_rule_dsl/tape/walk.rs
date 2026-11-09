//! The transducer's walk over a named-only document, tokens included (RER-131 option B, t30 §3).
//!
//! A named-only document's records hold its named nodes and the tokens the fold kept. A rule may
//! still walk to a token: it may match the `except` keyword, or step to the `:` after a node. So
//! a walk numbers the other tokens after the records: record `r` is node `r`, and token `t` of
//! the [`TokenColumn`] is node `records + t`.
//!
//! The column lists tokens by parent, in the records' preorder. So the tokens below a record,
//! its own included, are one range: a record's descendants are its records' range and this one.
//!
//! [`TapeDoc::walking_tokens`] gives the document a walk sees. Every other reader of the
//! document's `transducer_*` ids reads records only.

use super::TapeDoc;
use super::tokens::Item;
use std::ops::Range;

/// A token the records leave out. 16 bytes: the column is what a named-only document pays
/// for the tokens a walk needs, against a 40-byte record each in a full document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct TokenRow {
    pub(in crate::code_rule_dsl) start: u32,
    pub(in crate::code_rule_dsl) end: u32,
    /// Its place among its parent's children, tokens included.
    pub(in crate::code_rule_dsl) slot: u32,
    pub(in crate::code_rule_dsl) kind: u16,
    pub(in crate::code_rule_dsl) field: u16,
}

/// The tokens the records leave out, by parent: record `r`'s are `rows[offsets[r]..offsets[r + 1]]`.
#[derive(Debug, Default)]
pub(in crate::code_rule_dsl) struct TokenColumn {
    offsets: Vec<u32>,
    rows: Vec<TokenRow>,
}

impl TokenColumn {
    /// The column of `doc`, recognized from its source (`tokens::TokenGrammar::sequence`).
    pub(super) fn build(doc: &TapeDoc) -> Self {
        let records = doc.data.len();
        let mut column = Self { offsets: Vec::with_capacity(records + 1), rows: Vec::new() };
        for record in 0..records as u32 {
            column.offsets.push(column.rows.len() as u32);
            for (slot, item) in doc.transducer_parts(record).into_iter().enumerate() {
                if let Item::Token { kind, field, start, end } = item {
                    column.rows.push(TokenRow { start, end, slot: slot as u32, kind, field });
                }
            }
        }
        column.offsets.push(column.rows.len() as u32);
        column
    }

    pub(in crate::code_rule_dsl) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(in crate::code_rule_dsl) fn row(&self, token: u32) -> &TokenRow {
        &self.rows[token as usize]
    }

    /// Record `record`'s own tokens.
    pub(in crate::code_rule_dsl) fn of(&self, record: u32) -> Range<u32> {
        self.offsets[record as usize]..self.offsets[record as usize + 1]
    }

    /// The tokens of records `records` (a subtree's range), their own included.
    pub(in crate::code_rule_dsl) fn below(&self, records: Range<u32>) -> Range<u32> {
        self.offsets[records.start as usize]..self.offsets[records.end as usize]
    }

    /// The record a token belongs to.
    pub(in crate::code_rule_dsl) fn parent(&self, token: u32) -> u32 {
        (self.offsets.partition_point(|&offset| offset <= token) - 1) as u32
    }

    /// Bytes held.
    pub(in crate::code_rule_dsl) fn bytes(&self) -> usize {
        self.offsets.len() * 4 + self.rows.len() * std::mem::size_of::<TokenRow>()
    }
}

/// A node's children ([`TapeDoc::transducer_children`]): the records' sibling links, or a walk's
/// merged list.
pub(in crate::code_rule_dsl) enum Children<'d> {
    Linked { doc: &'d TapeDoc, next: u32 },
    Listed(std::vec::IntoIter<u32>),
}

impl Iterator for Children<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        match self {
            Self::Linked { doc, next } => {
                let current = *next;
                if current == super::NONE {
                    return None;
                }
                *next = doc.data[current as usize].next;
                Some(current)
            }
            Self::Listed(list) => list.next(),
        }
    }
}

/// A walk's node: a record, or a token of the column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum At<'c> {
    Record(u32),
    Token(u32, &'c TokenRow),
}

impl TapeDoc {
    /// This document as a transducer walk sees it: on a named-only document, its `transducer_*`
    /// accessors number the column's tokens after the records ([`TokenColumn`]). A full
    /// document is itself. A clone: the records, the source and the column are shared.
    pub(in crate::code_rule_dsl) fn walking_tokens(&self) -> Self {
        if self.tokens.is_none() || self.walking {
            return self.clone();
        }
        let _ = self.token_column.get_or_init(|| TokenColumn::build(self));
        Self { walking: true, ..self.clone() }
    }

    /// How many records this document holds: a walk's first token is node `record_count()`.
    pub(in crate::code_rule_dsl) fn record_count(&self) -> usize {
        self.data.len()
    }

    /// Whether this document's walk numbers tokens after its records.
    pub(in crate::code_rule_dsl) fn walks_tokens(&self) -> bool {
        self.walking
    }

    /// The column a walk reads, where it reads one.
    pub(super) fn walk_column(&self) -> Option<&TokenColumn> {
        if self.walking { self.token_column.get() } else { None }
    }

    /// What walk node `node` is.
    pub(super) fn at(&self, node: u32) -> At<'_> {
        let records = self.data.len() as u32;
        match self.walk_column() {
            Some(column) if node >= records => At::Token(node - records, column.row(node - records)),
            _ => At::Record(node),
        }
    }

    /// `record`'s children in a walk, tokens included, in source order.
    pub(super) fn walk_children(&self, record: u32, column: &TokenColumn) -> Vec<u32> {
        let records = self.data.len() as u32;
        let tokens = column.of(record);
        let mut out = Vec::with_capacity(tokens.len());
        let mut kids = self.record_children(record).peekable();
        let mut token = tokens.start;
        while kids.peek().is_some() || token < tokens.end {
            if token < tokens.end && column.row(token).slot as usize == out.len() || kids.peek().is_none() {
                out.push(records + token);
                token += 1;
            } else {
                out.push(kids.next().unwrap());
            }
        }
        out
    }
}
