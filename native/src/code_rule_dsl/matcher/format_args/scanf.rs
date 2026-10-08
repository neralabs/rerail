//! `formatArgs` with style `scanf`: a PHP `sscanf` or `fscanf` call against its format string, as
//! PHP 8.5.11's `ValidateFormat` reads it. With variables passed, PHP throws a `ValueError` where a
//! placeholder numbers a variable past the last passed (a missing field) and where a variable is
//! passed that no placeholder assigns (an unused one, a gap `%2$s` leaves included); with none,
//! the call returns an array and nothing is missing or unused.

use super::printf::{call_values, format_tree};
use super::{Fields, Key, Prepared, Side, key_of, positional_past};
use crate::code_rule_dsl::literal::{Grammar, SubDoc};
use crate::code_rule_dsl::matcher::value::Reading;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{AstGrep, Doc};
use std::collections::BTreeSet;

/// The call's variables and the format's fields: `string` captures the format (an argument, or its
/// expression, read through parentheses, as `printf` reads it), `args` the variables after it (a
/// `$$$` capture, or one capture of a single variable).
pub(super) fn prepare<'t, D: Doc>(
    string: &str,
    args: &str,
    reading: &Reading,
    env: &MetaVarEnv<'t, D>,
) -> Prepared<'t, D> {
    let Some(string) = env.get_match(string) else {
        return Prepared::NoMatch;
    };
    let tree = match format_tree(string.clone(), Grammar::Scanf, reading) {
        Ok(tree) => tree,
        Err(prepared) => return prepared,
    };
    let root = AstGrep::doc(SubDoc { tree: tree.clone() });
    let parsed = root.root();
    // A format PHP refuses throws whatever the variables: none is missing or unused.
    if parsed.children().any(|part| part.kind() == "error") {
        return Prepared::Unknown;
    }
    let fields = Fields {
        fields: parsed
            .children()
            .filter(|part| part.kind() == "placeholder")
            .filter_map(|part| Some((part.node_id(), key_of(tree.value(part.node_id()))?)))
            .collect(),
    };
    let variables = match env.get_match(args) {
        Some(variable) => call_values(vec![variable.clone()]),
        None => call_values(env.get_multiple_matches(args)),
    };
    let (unused, missing) = match variables {
        // With no variable the call returns an array.
        Some(variables) if variables.is_empty() => {
            (Side::known(Vec::new()), Side::known(Vec::new()))
        }
        Some(variables) => {
            let assigned: BTreeSet<usize> = fields
                .fields
                .iter()
                .filter_map(|(_, key)| match key {
                    Key::Index(index) => Some(*index),
                    Key::Name(_) => None,
                })
                .collect();
            let unused = variables
                .iter()
                .enumerate()
                .filter(|(index, _)| !assigned.contains(index))
                .map(|(_, variable)| (variable.clone(), true))
                .collect();
            (
                Side::known(unused),
                positional_past(&fields, variables.len()),
            )
        }
        None => (Side::unknown(), Side::unknown()),
    };
    Prepared::Sides {
        tree,
        unused,
        missing,
    }
}
