//! CPython's own facts (`code_rule_dsl::cpython`, native: no Python runs) as base relations, made by a batch only for a
//! program that reads them (`CPython::new`'s `reads`), over each Python document's text:
//!
//! ```text
//! cpython_syntax_error(d, l, msg)    `ast.parse` (CPython 3.13) rejects document d: its SyntaxError's lineno and msg
//! ```
//!
//! core.rr's `syntax_error(d, l, msg)` lowers to `cpython_syntax_error` where a program does not declare a
//! `syntax_error` of its own (`rr::lower::core`); the tape's `syntax_error(d)` (an ERROR or MISSING node) is apart.
//! Rows hold string indexes, as `Batch::build`'s do.

use super::Projected;
use std::collections::BTreeSet;

/// The relation the corpus binds for CPython's verdict (core.rr's `syntax_error(d, l, msg)`).
pub(crate) const SYNTAX_ERROR: &str = "cpython_syntax_error";

#[derive(Default)]
pub(super) struct CPython {
    syntax_error: bool,
    syntax_errors: Vec<[i64; 3]>,
}

impl CPython {
    pub(super) fn new(reads: &BTreeSet<String>) -> Self {
        Self { syntax_error: reads.contains(SYNTAX_ERROR), ..Self::default() }
    }

    /// Document `p`'s rows (`python`: it is Python's).
    pub(super) fn document(&mut self, p: &Projected, python: bool, string: &mut dyn FnMut(&str) -> usize) {
        if !python || !self.syntax_error {
            return;
        }
        if let Some(e) = crate::code_rule_dsl::cpython::syntax::syntax_error(p.facts.source.as_str()) {
            self.syntax_errors.push([i64::from(p.key), e.line as i64, string(&e.msg) as i64]);
        }
    }

    /// The relations read, their strings as atoms (`atom` of a string index).
    pub(super) fn relations(self, atom: &dyn Fn(i64) -> i64) -> Vec<(&'static str, super::Relation)> {
        let mut out = Vec::new();
        if self.syntax_error {
            let mut r = super::Relation::new(&["d", "l", "msg"]);
            self.syntax_errors.iter().for_each(|&[d, l, m]| r.push(&[d, l, atom(m)]));
            out.push((SYNTAX_ERROR, r));
        }
        out
    }
}
