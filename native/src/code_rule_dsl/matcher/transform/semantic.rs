use super::super::value::Reading;
use ast_grep_core::{Doc, Node};
use crate::code_rule_dsl::language::adapter::pack as language_pack;

/// A node's value as the host language converts it to a string: JavaScript's `String(v)` of a
/// primitive or a RegExp (an identifier otherwise reads as its name, escapes decoded), Python's
/// `str(v)` of a scalar. `None` where the value is not known or has no such conversion here.
pub(super) fn value_text<D: Doc>(reading: &Reading, node: &Node<'_, D>) -> Option<String> {
    let known = reading.value(node);
    { let pack = language_pack::pack(reading.lang()); language_pack::value_to_str(pack, &known, node) }
}
