//! Atomic literal decoding for SQL: this never evaluates the parsed tree's rule.
use super::{ParseTest, THREE_VALUED, UNKNOWN, UTIL_EXCEEDED, UTIL_STEPS_LEFT, UtilLimit};
use crate::code_rule_dsl::{literal::Parsed, scope::ScopeTable};
use ast_grep_core::{Doc, Node};
use std::sync::Arc;

type ParsedAt = Option<(Parsed, Option<Arc<ScopeTable>>)>;

pub(in crate::code_rule_dsl) fn sql_parse_budget<D: Doc>(
    test: &ParseTest,
    node: &Node<'_, D>,
    work: usize,
) -> (Result<ParsedAt, String>, usize) {
    struct Reset(bool, bool, usize, Option<UtilLimit>);
    impl Drop for Reset {
        fn drop(&mut self) {
            THREE_VALUED.set(self.0);
            UNKNOWN.set(self.1);
            UTIL_STEPS_LEFT.set(self.2);
            UTIL_EXCEEDED.set(self.3);
        }
    }
    let _reset = Reset(
        THREE_VALUED.replace(true),
        UNKNOWN.replace(false),
        UTIL_STEPS_LEFT.replace(work),
        UTIL_EXCEEDED.replace(None),
    );
    let parsed = test.tree_at(node);
    // tree_at's None can represent an exhausted decode; check the limit before
    // interpreting absence as a host that does not hold a literal.
    let result = match UTIL_EXCEEDED.get() {
        Some(limit) => Err(format!("{limit:?}")),
        None => Ok(parsed),
    };
    (result, UTIL_STEPS_LEFT.get())
}
