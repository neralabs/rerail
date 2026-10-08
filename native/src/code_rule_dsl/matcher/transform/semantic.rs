use super::super::value::Reading;
use crate::code_rule_dsl::value::{self, Lang, key};
use ast_grep_core::{Doc, Node};

/// A node's value as the host language converts it to a string: JavaScript's `String(v)` of a
/// primitive or a RegExp (an identifier otherwise reads as its name, escapes decoded), Python's
/// `str(v)` of a scalar. `None` where the value is not known or has no such conversion here.
pub(super) fn value_text<D: Doc>(reading: &Reading, node: &Node<'_, D>) -> Option<String> {
    let known = reading.value(node);
    match reading.lang() {
        Lang::Python => value::python::to_str(&known),
        Lang::Js => value::ecma::ops::to_string(&known)
            .or_else(|| {
                (!known.is_known() && node.kind() != "string")
                    .then(|| key::node_name(node))
                    .flatten()
            })
            .and_then(|units| String::from_utf16(&units).ok()),
        Lang::Php => value::php::to_string(&known),
    }
}
