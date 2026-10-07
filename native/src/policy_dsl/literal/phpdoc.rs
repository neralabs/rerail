//! `phpDoc`: PHP doc comments (`T_DOC_COMMENT`: `/**` and white space) as PHPStan 2.2.16 reads
//! them, through phpstan/phpdoc-parser 2.3.5 (`PhpDocStringResolver`: the comment's whole text,
//! lexed and parsed); the oracle is that parser (`fixtures/php/literal/generate_phpdoc.php`).
//!
//! The tree is phpdoc-parser's AST. A node's kind is its class without the namespace and the
//! `Node` suffix, in snake case: the `php_doc` root holds `php_doc_text` and `php_doc_tag`
//! children; a tag holds its value (`param_tag_value`, `return_tag_value`, `var_tag_value`,
//! `template_tag_value`, `method_tag_value`, `generic_tag_value`, ...), and the values hold types
//! (`identifier_type`, `union_type`, `generic_type`, `array_shape`, `callable_type`,
//! `conditional_type`, `const_type`, ...) and constant expressions (`const_expr_integer`,
//! `const_fetch`, ...). Each node covers the tokens phpdoc-parser's `startIndex` and `endIndex`
//! attributes name. A tag value phpdoc-parser cannot parse is an `invalid_tag_value` (a type
//! alias's type, an `invalid_type`), an error node valued by the parser's message.
//!
//! phpdoc-parser keeps names, variables and descriptions as strings; the tree adds them where the
//! text spells them, as `name` (a tag's name, a method's, a template's, an alias), `variable` (a
//! parameter name, `$this`) and `description` leaves valued by the string. Children are in text
//! order. A tag's value is its name, a text's its text, an identifier's its name, a constant's
//! its value (a class constant's `Class::NAME`), an array shape's its kind.
//!
//! Nesting past `value::MAX_DEPTH` (in the parse, or in the tree: an array type's `[]` suffixes
//! wrap it without nesting the parse), or a parse that backtracks past a bound proportional to the
//! comment's tokens, is not read: the comment's tree is unknown.

mod lexer;
mod parser;

use super::{Grammar, PNode};
use crate::policy_dsl::value::Value;

pub(crate) const KINDS: &[&str] = &[
    "php_doc",
    "php_doc_text",
    "php_doc_tag",
    "param_tag_value",
    "typeless_param_tag_value",
    "param_immediately_invoked_callable_tag_value",
    "param_later_invoked_callable_tag_value",
    "param_closure_this_tag_value",
    "pure_unless_callable_is_impure_tag_value",
    "pure_unless_parameter_is_passed_tag_value",
    "var_tag_value",
    "return_tag_value",
    "throws_tag_value",
    "mixin_tag_value",
    "require_extends_tag_value",
    "require_implements_tag_value",
    "sealed_tag_value",
    "deprecated_tag_value",
    "property_tag_value",
    "method_tag_value",
    "method_tag_value_parameter",
    "template_tag_value",
    "extends_tag_value",
    "implements_tag_value",
    "uses_tag_value",
    "type_alias_tag_value",
    "type_alias_import_tag_value",
    "assert_tag_value",
    "assert_tag_property_value",
    "assert_tag_method_value",
    "self_out_tag_value",
    "param_out_tag_value",
    "generic_tag_value",
    "invalid_tag_value",
    "doctrine_tag_value",
    "doctrine_annotation",
    "doctrine_argument",
    "doctrine_array",
    "doctrine_array_item",
    "identifier_type",
    "this_type",
    "nullable_type",
    "union_type",
    "intersection_type",
    "generic_type",
    "array_type",
    "array_shape",
    "array_shape_item",
    "array_shape_unsealed_type",
    "object_shape",
    "object_shape_item",
    "callable_type",
    "callable_type_parameter",
    "conditional_type",
    "conditional_type_for_parameter",
    "const_type",
    "offset_access_type",
    "invalid_type",
    "const_expr_integer",
    "const_expr_float",
    "const_expr_string",
    "const_expr_true",
    "const_expr_false",
    "const_expr_null",
    "const_expr_array",
    "const_expr_array_item",
    "const_fetch",
    "doctrine_const_expr_string",
    "name",
    "variable",
    "description",
];

const G: Grammar = Grammar::PhpDoc;

/// Whether PHP lexes a comment's text as a doc comment (`T_DOC_COMMENT`: `/**` and white space).
pub(crate) fn is_doc_comment(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 6
        && bytes.starts_with(b"/**")
        && matches!(bytes[3], b' ' | b'\t' | b'\n' | b'\r')
        && bytes.ends_with(b"*/")
}

/// The tree of a doc comment; `None` past the nesting or backtracking bound.
pub(crate) fn parse(text: &str) -> Option<PNode> {
    let (parser, root) = parser::parse(text);
    if parser.exhausted() {
        return None;
    }
    match root {
        Ok(root) => tree(&parser, root, 0, 0),
        // phpdoc-parser throws for the comment (an unclosed Doctrine annotation tag): one error.
        Err(fatal) => {
            let mut root = PNode::new(G, "php_doc", 0, text.len());
            root.error = true;
            root.value = Some(Value::PhpStr(fatal.message().as_bytes().to_vec()));
            Some(root)
        }
    }
}

fn prop<'p>(node: &'p parser::Node, name: &str) -> Option<&'p [u8]> {
    node.props.iter().find_map(|(key, prop)| match prop {
        parser::Prop::Str(value) if *key == name => Some(value.as_slice()),
        _ => None,
    })
}

/// The tree node of `id` at `depth`, `None` deeper than `value::MAX_DEPTH`; `parent` is where a
/// node of no token starts.
fn tree(parser: &parser::Parser<'_>, id: parser::Id, parent: usize, depth: usize) -> Option<PNode> {
    if depth > crate::policy_dsl::value::MAX_DEPTH {
        return None;
    }
    let node = &parser.nodes[id];
    let (start, end) = match (node.start, node.end) {
        (Some(start), Some(end)) => {
            let start = parser.tokens[start].start;
            (start, parser.tokens[end].end.max(start))
        }
        _ => (parent, parent),
    };
    let mut out = PNode::new(G, node.kind, start, end);
    let value = match node.kind {
        "php_doc_tag" | "identifier_type" | "doctrine_annotation" => prop(node, "name"),
        "php_doc_text" => prop(node, "text"),
        "generic_tag_value"
        | "const_expr_integer"
        | "const_expr_float"
        | "const_expr_string"
        | "doctrine_const_expr_string" => prop(node, "value"),
        "invalid_tag_value" | "invalid_type" => {
            out.error = true;
            prop(node, "message")
        }
        "array_shape" => prop(node, "kind"),
        "const_fetch" => {
            let class = prop(node, "className").unwrap_or_default();
            let name = prop(node, "name").unwrap_or_default();
            let mut value = Vec::with_capacity(class.len() + 2 + name.len());
            if !class.is_empty() {
                value.extend_from_slice(class);
                value.extend_from_slice(b"::");
            }
            value.extend_from_slice(name);
            out.value = Some(Value::PhpStr(value));
            None
        }
        _ => None,
    };
    if let Some(value) = value {
        out.value = Some(Value::PhpStr(value.to_vec()));
    }
    let mut children = node
        .children
        .iter()
        .map(|(_, child)| tree(parser, *child, start, depth + 1))
        .collect::<Option<Vec<PNode>>>()?;
    children.extend(node.leaves.iter().map(|leaf| {
        PNode::new(G, leaf.kind, leaf.span.start, leaf.span.end)
            .with_value(Value::PhpStr(leaf.value.clone()))
    }));
    children.sort_by_key(|child| child.start);
    out.children = children;
    Some(out)
}

#[cfg(test)]
mod tests;
