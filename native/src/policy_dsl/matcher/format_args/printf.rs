//! `formatArgs` with style `printf`: a PHP printf-family call (`sprintf`, `printf`, `fprintf`,
//! and with an array of values `vsprintf`, `vprintf`, `vfprintf`) against its format string, as
//! PHP 8.5.11's `php_formatted_print` and PHPStan 2.2.16's `PrintfParametersRule` and
//! `PrintfArrayParametersRule` (`argument.sprintf`) read it. A format consumes the values its
//! placeholders and stars number; PHP throws an `ArgumentCountError` (a `ValueError` for an array)
//! where one is not passed, a missing field, and ignores the values past the highest it numbers,
//! the unused ones PHPStan counts. A value a numbered placeholder skips (`%2$s` with two values) is
//! not unused: PHP and PHPStan read the count only.

use super::{Fields, Prepared, Side, key_of, positional_past};
use crate::policy_dsl::literal::{Grammar, Parsed, SubDoc, SubTree};
use crate::policy_dsl::matcher::parse::{parsed_tree, parsed_value_tree};
use crate::policy_dsl::matcher::value::{Reading, spend_step};
use crate::policy_dsl::value::{Context, php};
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{AstGrep, Doc, Node};
use std::sync::Arc;

/// The call's values and the format's fields: `string` captures the format (an argument, or its
/// expression, read through parentheses), `args` the values: a `$$$` capture of the arguments
/// after the format, or one capture of the array a `v*printf` call passes.
pub(super) fn prepare<'t, D: Doc>(
    string: &str,
    args: &str,
    reading: &Reading,
    env: &MetaVarEnv<'t, D>,
) -> Prepared<'t, D> {
    let Some(string) = env.get_match(string) else {
        return Prepared::NoMatch;
    };
    let tree = match format_tree(string.clone(), Grammar::Printf, reading) {
        Ok(tree) => tree,
        Err(prepared) => return prepared,
    };
    let root = AstGrep::doc(SubDoc { tree: tree.clone() });
    let parsed = root.root();
    // A format PHP refuses throws a `ValueError` whatever the values: none is missing or unused.
    if parsed.children().any(|part| part.kind() == "error") {
        return Prepared::Unknown;
    }
    let fields = Fields {
        fields: parsed
            .dfs()
            .filter(|part| matches!(part.kind().as_ref(), "placeholder" | "star"))
            .filter_map(|part| Some((part.node_id(), key_of(tree.value(part.node_id()))?)))
            .collect(),
    };
    let values = match env.get_match(args) {
        Some(array) => array_values(array),
        None => call_values(env.get_multiple_matches(args)),
    };
    let (unused, missing) = match values {
        Some(values) => sides(&values, &fields),
        None => (Side::unknown(), Side::unknown()),
    };
    Prepared::Sides {
        tree,
        unused,
        missing,
    }
}

/// The format's tree under `grammar`: a string literal's, each node mapped to its source, or else
/// the string PHP gives the expression (a concatenation, a cast, a variable through the one binding
/// that reaches it: `value/php.rs` under `reading`, as PHPStan reads a constant string), every node
/// mapped to the whole expression. No match where the format is known and no string; unknown
/// where it is not known.
pub(super) fn format_tree<'t, D: Doc>(
    string: Node<'t, D>,
    grammar: Grammar,
    reading: &Reading,
) -> Result<Arc<SubTree>, Prepared<'t, D>> {
    let format = expression(string);
    let parsed = match parsed_tree(&format, grammar) {
        Some(Parsed::NotString) => parsed_value_tree(&format, reading, grammar),
        parsed => parsed,
    };
    match parsed {
        None | Some(Parsed::NotString) => Err(Prepared::NoMatch),
        Some(Parsed::Unknown) => Err(Prepared::Unknown),
        Some(Parsed::Tree(tree)) => Ok(tree),
    }
}

/// The values past the highest index a field numbers, and the fields numbering a value past the
/// last passed.
fn sides<'t, D: Doc>(values: &[Node<'t, D>], fields: &Fields) -> (Side<Node<'t, D>>, Side<usize>) {
    let required = fields
        .fields
        .iter()
        .filter_map(|(_, key)| match key {
            super::Key::Index(index) => Some(index + 1),
            super::Key::Name(_) => None,
        })
        .max()
        .unwrap_or(0);
    let unused = values
        .iter()
        .skip(required)
        .map(|value| (value.clone(), true))
        .collect();
    (Side::known(unused), positional_past(fields, values.len()))
}

/// The arguments after the format: `None` where their count is not known (an `...` unpacking, a
/// named argument, which PHP refuses for these functions' variadic values).
pub(super) fn call_values<'t, D: Doc>(args: Vec<Node<'t, D>>) -> Option<Vec<Node<'t, D>>> {
    let args: Vec<Node<'t, D>> = args
        .into_iter()
        .filter(|arg| arg.is_named() && arg.kind() != "comment")
        .collect();
    args.iter()
        .all(|arg| {
            arg.kind() == "argument"
                && arg.field("name").is_none()
                && !arg
                    .children()
                    .any(|child| child.kind() == "variadic_unpacking")
        })
        .then_some(args)
}

/// The values a `v*printf` array literal passes, each as its `array_element_initializer`: PHP
/// reads the array's values in order, its keys ignored, so the entries left after PHP coerces and
/// replaces duplicate keys (`[0 => $a, '0' => $b]` passes `$b` alone), as `value/php.rs` reads
/// them. `None` where they are not known (a key not known, a spread, a reference).
fn array_values<'t, D: Doc>(array: &Node<'t, D>) -> Option<Vec<Node<'t, D>>> {
    let array = expression(array.clone());
    let mut spend = spend_step;
    let mut context = Context::new(&mut spend);
    php::array_entries(&array, &mut context)?
        .into_iter()
        .map(|(_, value)| value.parent())
        .collect()
}

/// The expression an argument passes (a pattern's `$A` in a call captures the `argument`), inside
/// PHP's parentheses.
pub(super) fn expression<'t, D: Doc>(mut node: Node<'t, D>) -> Node<'t, D> {
    if node.kind() == "argument"
        && let Some(value) = node.children().filter(|child| child.is_named()).last()
    {
        node = value;
    }
    while node.kind() == "parenthesized_expression" {
        let mut inner = node
            .children()
            .filter(|child| child.is_named() && child.kind() != "comment");
        let (Some(expression), None) = (inner.next(), inner.next()) else {
            break;
        };
        drop(inner);
        node = expression;
    }
    node
}
