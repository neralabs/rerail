//! Structural equality of PHP expressions and statements (tree-sitter-php 0.24.2), as
//! nikic/PHP-Parser 5.9.0 builds their syntax trees and compares them with every attribute
//! stripped (positions, comments, a string's quotes, a number's base, `array()` against `[]`).
//! A node reads as a sequence of tokens, one per syntax node or leaf, with parentheses, comments,
//! punctuation and keywords the tree's shape records dropped; two nodes are equal when their
//! sequences are. Literals read as PHP-Parser's scalars do: integers and floats by value and type
//! (`1 == 0x1`, `1 != 1.0`; an integer past `PHP_INT_MAX` is a float), strings by their bytes
//! (`'a' == "a" == <<<X\na\nX`); `true`, `false` and `null` are constants, by their name as
//! written. Synonyms PHP-Parser does not tell apart compare equal: `<>` and `!=`, `(integer)` and
//! `(int)`, `exit` and `die`, `new C` and `new C()`, a block and a single statement as a body,
//! `{...}` and `: ... endif;`, `$a` and `{$a}` in a string, `list()` and `[]`. The oracle is
//! `fixtures/php/compare/generate_php.php`.
//!
//! `Hash` is PHP's array key identity: an expression PHP folds to a constant (`value/php.rs`)
//! compares by the key PHP coerces it to (`1 == '1' == 1.5 == true`, `null == ''`); anything else
//! structurally.

use super::php::{self, Key};
use super::structure::{Equality, Tok};
use super::{Context, Value};
use ast_grep_core::{Doc, Node};

/// The node's structure under an equality; `None` where it is not known (a syntax error, a literal
/// that does not decode, an interpolated heredoc, nesting past the depth, a spent budget).
pub(crate) fn structure<D: Doc>(
    node: &Node<'_, D>,
    equality: Equality,
    context: &mut Context<'_>,
) -> Option<Vec<Tok>> {
    let node = unparenthesized(node);
    let mut out = Vec::new();
    match equality {
        Equality::Plain => walk(&node, context, &mut out)?,
        Equality::Hash => match folded_key(&node, context) {
            Some(key) => {
                out.push(Tok::Open("#key".into()));
                out.push(match key {
                    Key::Int(index) => Tok::Leaf("int".into(), index.to_string()),
                    Key::Str(text) => Tok::Bytes(text),
                });
                out.push(Tok::Close);
            }
            None => walk(&node, context, &mut out)?,
        },
        // Refused when the code rule compiles: PHP has no implicit concatenation.
        Equality::Parts => return None,
    }
    Some(out)
}

/// The array key an expression PHP folds to a constant is coerced to; `None` for anything else.
fn folded_key<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Option<Key> {
    if !php::VALUE_KINDS.contains(&node.kind().as_ref()) {
        return None;
    }
    match php::decode(node, context) {
        Value::Unknown | Value::Container { .. } => None,
        value => php::key_of(&value),
    }
}

fn trivia<D: Doc>(node: &Node<'_, D>) -> bool {
    node.kind() == "comment"
}

fn named_children<'r, D: Doc>(node: &Node<'r, D>) -> impl Iterator<Item = Node<'r, D>> {
    node.children()
        .filter(|child| child.is_named() && !trivia(child))
}

/// The expression inside any number of parentheses.
fn unparenthesized<'r, D: Doc>(node: &Node<'r, D>) -> Node<'r, D> {
    let mut node = node.clone();
    while node.kind() == "parenthesized_expression" {
        let children: Vec<_> = named_children(&node).collect();
        match children.as_slice() {
            [inner] => node = inner.clone(),
            _ => break,
        }
    }
    node
}

/// Nodes whose anonymous tokens are their operator: kept, every other anonymous token is
/// punctuation or a keyword the node's kind records.
const OPERATOR_PARENTS: &[&str] = &[
    "binary_expression",
    "unary_op_expression",
    "augmented_assignment_expression",
    "update_expression",
    "yield_expression",
    "namespace_use_declaration",
    "namespace_use_clause",
    "namespace_use_group",
    "declare_directive",
    "list_literal",
];

/// Punctuation, which the tree's shape records.
fn punctuation(kind: &str) -> bool {
    matches!(
        kind,
        "," | ";" | "(" | ")" | "[" | "]" | "{" | "}" | ":" | "list" | "\""
    )
}

/// Modifiers, which PHP-Parser records as flags: in any order and case.
const MODIFIERS: &[&str] = &[
    "visibility_modifier",
    "static_modifier",
    "final_modifier",
    "abstract_modifier",
    "readonly_modifier",
    "var_modifier",
];

/// Keyword-like nodes PHP-Parser records as flags, in any case.
const LOWERED: &[&str] = &[
    "visibility_modifier",
    "static_modifier",
    "final_modifier",
    "abstract_modifier",
    "readonly_modifier",
    "var_modifier",
    "reference_modifier",
];

/// The builtin types PHP-Parser 5.9.0 lower-cases for PHP 8.5 (`PhpVersion::BUILTIN_TYPE_VERSIONS`).
const BUILTIN_TYPES: &[&str] = &[
    "array", "callable", "bool", "int", "float", "string", "iterable", "void", "object", "null",
    "false", "mixed", "never", "true",
];

/// Statement kinds whose `body` is a list of statements in PHP-Parser, however it is written.
const BODIES: &[&str] = &[
    "if_statement",
    "else_if_clause",
    "else_clause",
    "while_statement",
    "for_statement",
    "foreach_statement",
    "do_statement",
    "declare_statement",
];

fn walk<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>, out: &mut Vec<Tok>) -> Option<()> {
    if !context.enter() {
        return None;
    }
    let result = walk_node(node, context, out);
    context.leave();
    if context.exhausted() {
        return None;
    }
    result
}

fn walk_node<D: Doc>(
    node: &Node<'_, D>,
    context: &mut Context<'_>,
    out: &mut Vec<Tok>,
) -> Option<()> {
    if node.is_error() || node.is_missing() {
        return None;
    }
    let kind = node.kind();
    match kind.as_ref() {
        "parenthesized_expression" => {
            let inner = unparenthesized(node);
            if inner.kind() != "parenthesized_expression" {
                return walk(&inner, context, out);
            }
        }
        "integer" | "float" => {
            out.push(match php::decode(node, context) {
                Value::Int(value) => Tok::BigInt(value.to_string()),
                Value::Float(value) => Tok::Float(value.to_bits()),
                _ => return None,
            });
            return Some(());
        }
        "string" | "nowdoc" => return string_value(node, context, out),
        "encapsed_string" | "heredoc" | "shell_command_expression" => {
            return interpolated(node, context, out);
        }
        "cast_type" => {
            let text = node.text().to_ascii_lowercase();
            let canonical = match text.as_str() {
                "integer" => "int",
                "boolean" => "bool",
                "double" | "real" => "float",
                "binary" => "string",
                other => other,
            };
            out.push(Tok::Leaf(kind.into_owned(), canonical.to_owned()));
            return Some(());
        }
        // `use \A\B;` imports `A\B`, as `use A\B;` does.
        "qualified_name" | "relative_name" | "namespace_name" => {
            let text = node.text();
            let imported = node.parent().is_some_and(|parent| {
                matches!(
                    parent.kind().as_ref(),
                    "namespace_use_clause" | "namespace_use_group"
                )
            });
            let text = if imported {
                text.trim_start_matches('\\')
            } else {
                text.as_ref()
            };
            out.push(Tok::Leaf(kind.into_owned(), text.to_owned()));
            return Some(());
        }
        // `self`, `static` and `parent` are names as written, however the grammar reads them.
        "relative_scope" => {
            out.push(Tok::Leaf("name".into(), node.text().into_owned()));
            return Some(());
        }
        // `;` alone is no statement, unless a comment precedes it (PHP-Parser's `Nop`).
        "empty_statement" => {
            if node.prev().is_some_and(|previous| trivia(&previous)) {
                out.push(Tok::Leaf("#nop".into(), String::new()));
            }
            return Some(());
        }
        // A builtin type is a lower-cased identifier however it is written (`INT`), as
        // PHP-Parser's `handleBuiltinTypes` makes it.
        "primitive_type" | "bottom_type" => {
            out.push(Tok::Leaf(
                "primitive_type".into(),
                node.text().to_ascii_lowercase(),
            ));
            return Some(());
        }
        "named_type" => {
            let children: Vec<_> = named_children(node).collect();
            if let [name] = children.as_slice()
                && name.kind() == "name"
            {
                let lower = name.text().to_ascii_lowercase();
                if BUILTIN_TYPES.contains(&lower.as_str()) {
                    out.push(Tok::Leaf("primitive_type".into(), lower));
                    return Some(());
                }
            }
        }
        // An empty argument list is no arguments: `new C` is `new C()`.
        "arguments" if named_children(node).next().is_none() => return Some(()),
        // `exit;` is the expression `exit`, as `die()` and `exit(1)` are.
        "exit_statement" => {
            out.push(Tok::Open("expression_statement".into()));
            let argument: Vec<_> = named_children(node).collect();
            construct("exit", &argument, context, out)?;
            out.push(Tok::Close);
            return Some(());
        }
        // `die` alone is `exit`.
        "name" if exit_name(node) => return construct::<D>("exit", &[], context, out),
        "function_call_expression" => {
            if let Some(function) = node.field("function")
                && function.kind() == "name"
            {
                let lower = function.text().to_ascii_lowercase();
                let construct_name = match lower.as_str() {
                    "exit" | "die" => Some("exit"),
                    "empty" | "eval" | "isset" => Some(lower.as_str()),
                    _ => None,
                };
                if let Some(construct_name) = construct_name {
                    let mut values = Vec::new();
                    if let Some(arguments) = node.field("arguments") {
                        for argument in named_children(&arguments) {
                            let parts: Vec<_> = named_children(&argument).collect();
                            match parts.as_slice() {
                                [value]
                                    if argument.kind() == "argument"
                                        && argument.field("name").is_none()
                                        && value.kind() != "variadic_unpacking" =>
                                {
                                    values.push(value.clone());
                                }
                                // A language construct takes no named, unpacked or
                                // by-reference argument.
                                _ => return None,
                            }
                        }
                    }
                    return construct(construct_name, &values, context, out);
                }
            }
        }
        "list_literal" => return list(node, context, out),
        "for_statement" => {
            out.push(Tok::Open(kind.into_owned()));
            for (field, slot) in [
                ("initialize", node.field("initialize")),
                ("condition", node.field("condition")),
                ("update", node.field("update")),
            ] {
                out.push(Tok::Token(field.into()));
                if let Some(slot) = slot {
                    walk(&slot, context, out)?;
                }
            }
            body(node.field("body"), context, out)?;
            out.push(Tok::Close);
            return Some(());
        }
        _ => {}
    }
    if !node.is_named() {
        let parent = node.parent();
        if parent
            .as_ref()
            .is_some_and(|parent| OPERATOR_PARENTS.contains(&parent.kind().as_ref()))
            && !punctuation(&kind)
        {
            let token = if kind == "<>" { "!=" } else { kind.as_ref() };
            out.push(Tok::Token(token.to_owned()));
        }
        return Some(());
    }
    if LOWERED.contains(&kind.as_ref()) {
        out.push(Tok::Leaf(
            kind.into_owned(),
            node.text().to_ascii_lowercase(),
        ));
        return Some(());
    }
    let children = node
        .children()
        .filter(|child| !trivia(child))
        .collect::<Vec<_>>();
    if children.is_empty() {
        out.push(Tok::Leaf(kind.into_owned(), node.text().into_owned()));
        return Some(());
    }
    let bodies = BODIES.contains(&kind.as_ref());
    let body_range = if bodies {
        node.field("body").map(|body| body.range())
    } else {
        None
    };
    // Modifiers are flags in PHP-Parser: their order is not recorded.
    let mut modifiers: Vec<(String, String)> = children
        .iter()
        .filter(|child| MODIFIERS.contains(&child.kind().as_ref()))
        .map(|child| (child.kind().into_owned(), child.text().to_ascii_lowercase()))
        .collect();
    modifiers.sort();
    let mut modifiers = Some(modifiers);
    let list = STATEMENT_LISTS.contains(&kind.as_ref());
    out.push(Tok::Open(kind.into_owned()));
    for child in &children {
        if MODIFIERS.contains(&child.kind().as_ref()) {
            for (kind, text) in modifiers.take().into_iter().flatten() {
                out.push(Tok::Leaf(kind, text));
            }
        } else if body_range.as_ref() == Some(&child.range()) {
            body(Some(child.clone()), context, out)?;
        } else {
            walk(child, context, out)?;
        }
    }
    if list {
        trailing_comment(node, out);
    }
    out.push(Tok::Close);
    Some(())
}

/// Whether a `name` is `exit` or `die` written as an expression, which PHP reads as `exit`: not a
/// member's, declaration's or argument's name.
fn exit_name<D: Doc>(node: &Node<'_, D>) -> bool {
    if !matches!(node.text().to_ascii_lowercase().as_str(), "exit" | "die") {
        return false;
    }
    let Some(parent) = node.parent() else {
        return false;
    };
    !matches!(
        parent.kind().as_ref(),
        "class_constant_access_expression"
            | "const_element"
            | "named_label_statement"
            | "goto_statement"
            | "named_type"
            | "function_call_expression"
    ) && parent
        .field("name")
        .is_none_or(|name| name.range() != node.range())
}

/// A language construct PHP-Parser has a node of its own for, however it is written (`die(1)`,
/// `exit (1)`, `EMPTY($a)`): its name and its arguments' values.
fn construct<D: Doc>(
    name: &str,
    values: &[Node<'_, D>],
    context: &mut Context<'_>,
    out: &mut Vec<Tok>,
) -> Option<()> {
    out.push(Tok::Open(format!("#{name}")));
    for value in values {
        walk(value, context, out)?;
    }
    out.push(Tok::Close);
    Some(())
}

/// A statement body as PHP-Parser's list of statements: the statements of a block or of an
/// alternative syntax's `: ... end*;`, or the one statement written alone.
fn body<D: Doc>(
    body: Option<Node<'_, D>>,
    context: &mut Context<'_>,
    out: &mut Vec<Tok>,
) -> Option<()> {
    out.push(Tok::Open("#body".into()));
    if let Some(body) = body {
        match body.kind().as_ref() {
            "compound_statement" | "colon_block" => {
                for statement in named_children(&body) {
                    walk(&statement, context, out)?;
                }
                trailing_comment(&body, out);
            }
            _ => walk(&body, context, out)?,
        }
    }
    out.push(Tok::Close);
    Some(())
}

/// Statement lists PHP-Parser closes with a `Nop` where a comment follows their last statement.
const STATEMENT_LISTS: &[&str] = &[
    "compound_statement",
    "colon_block",
    "declaration_list",
    "enum_declaration_list",
];

/// A comment after a statement list's last statement is PHP-Parser's zero-length `Nop`
/// (`maybeCreateZeroLengthNop`): `{ // c }` is not `{ }`.
fn trailing_comment<D: Doc>(list: &Node<'_, D>, out: &mut Vec<Tok>) {
    if list
        .children()
        .filter(|child| child.is_named())
        .last()
        .is_some_and(|child| trivia(&child))
    {
        out.push(Tok::Leaf("#nop".into(), String::new()));
    }
}

/// `list(...)` and `[...]` as an assignment's target: PHP-Parser's items, separated by `,`, a
/// skipped one (`[, $b]`) empty. A trailing comma ends `[...]` without an item (`[$a,]` is
/// `[$a]`, `[$a, ,]` holds one empty item); `list(...)` reads an empty item after it
/// (`list($a,)` is `[$a, ,]`), as PHP-Parser's grammar does.
fn list<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>, out: &mut Vec<Tok>) -> Option<()> {
    let bracket = node
        .children()
        .next()
        .is_some_and(|first| first.kind() == "[");
    out.push(Tok::Open("list_literal".into()));
    let mut filled = false;
    let mut after_comma = false;
    for child in node.children().filter(|child| !trivia(child)) {
        if child.is_named() {
            walk(&child, context, out)?;
            filled = true;
            after_comma = false;
        } else if child.kind() == "," {
            if !filled {
                out.push(Tok::Missing);
            }
            out.push(Tok::Token(",".into()));
            filled = false;
            after_comma = true;
        } else if child.kind() == "=>" {
            out.push(Tok::Token("=>".into()));
        }
    }
    if after_comma {
        if bracket {
            out.pop();
        } else {
            out.push(Tok::Missing);
        }
    }
    out.push(Tok::Close);
    Some(())
}

/// A string without interpolation: its bytes.
fn string_value<D: Doc>(
    node: &Node<'_, D>,
    context: &mut Context<'_>,
    out: &mut Vec<Tok>,
) -> Option<()> {
    match php::decode(node, context) {
        Value::PhpStr(text) => {
            out.push(Tok::Bytes(text));
            Some(())
        }
        _ => None,
    }
}

/// A double-quoted string, heredoc or shell command: without interpolation, a string's bytes
/// (a shell command keeps its kind); with, PHP-Parser's parts, literal runs decoded and each
/// interpolated expression's structure (`$a`, `{$a}` and `${a}` are the same variable).
fn interpolated<D: Doc>(
    node: &Node<'_, D>,
    context: &mut Context<'_>,
    out: &mut Vec<Tok>,
) -> Option<()> {
    let kind = node.kind();
    let heredoc = kind == "heredoc";
    let body = if heredoc {
        node.field("value")
    } else {
        Some(node.clone())
    };
    let parts: Vec<_> = body
        .as_ref()
        .map(|body| named_children(body).collect())
        .unwrap_or_default();
    let literal = parts
        .iter()
        .all(|part| matches!(part.kind().as_ref(), "string_content" | "escape_sequence"));
    if kind != "shell_command_expression" && literal {
        return string_value(node, context, out);
    }
    // An interpolated heredoc's indentation is not read here.
    if heredoc {
        return None;
    }
    let source = node.text();
    let base = node.range().start;
    let children: Vec<_> = node.children().filter(|child| !trivia(child)).collect();
    // The literal text sits between the opening and the closing delimiter.
    let delimiters: Vec<_> = children
        .iter()
        .filter(|child| !child.is_named() && matches!(child.kind().as_ref(), "\"" | "`"))
        .collect();
    let (Some(open), Some(close)) = (delimiters.first(), delimiters.last()) else {
        return None;
    };
    if open.range() == close.range() {
        return None;
    }
    out.push(Tok::Open(if kind == "shell_command_expression" {
        "#shell".into()
    } else {
        "#interpolated".into()
    }));
    let mut literal_start = open.range().end;
    let mut index = 0;
    let push_literal = |from: usize, to: usize, out: &mut Vec<Tok>| -> Option<()> {
        if from < to {
            let raw = source.as_bytes().get(from - base..to - base)?;
            out.push(Tok::Bytes(php::unescape(
                raw,
                kind != "shell_command_expression",
            )?));
        }
        Some(())
    };
    while index < children.len() {
        let child = &children[index];
        if child.range().start < open.range().end || child.range().start >= close.range().start {
            index += 1;
            continue;
        }
        if child.is_named() && matches!(child.kind().as_ref(), "string_content" | "escape_sequence")
        {
            index += 1;
            continue;
        }
        // `{$expr}`: the braces delimit an expression.
        if !child.is_named() && child.kind() == "{" {
            let expression = children.get(index + 1)?;
            let closing = children.get(index + 2)?;
            if !expression.is_named() || closing.kind() != "}" {
                return None;
            }
            push_literal(literal_start, child.range().start, out)?;
            walk(expression, context, out)?;
            literal_start = closing.range().end;
            index += 3;
            continue;
        }
        if !child.is_named() {
            index += 1;
            continue;
        }
        push_literal(literal_start, child.range().start, out)?;
        simple_interpolation(child, context, out)?;
        literal_start = child.range().end;
        index += 1;
    }
    push_literal(literal_start, close.range().start, out)?;
    out.push(Tok::Close);
    Some(())
}

/// An expression interpolated without braces: `$a`, `${a}`, `$a[0]`, `$a[k]`, `$a[$i]`, `$a->b`.
fn simple_interpolation<D: Doc>(
    node: &Node<'_, D>,
    context: &mut Context<'_>,
    out: &mut Vec<Tok>,
) -> Option<()> {
    match node.kind().as_ref() {
        // `${a}` is the variable `$a`.
        "dynamic_variable_name" => {
            let children: Vec<_> = named_children(node).collect();
            match children.as_slice() {
                [name] if name.kind() == "name" => {
                    out.push(Tok::Open("variable_name".into()));
                    out.push(Tok::Leaf("name".into(), name.text().into_owned()));
                    out.push(Tok::Close);
                    Some(())
                }
                _ => walk(node, context, out),
            }
        }
        // `$a[k]` indexes with the string `k`, `$a[0]` with an integer where it is a canonical
        // decimal one (`$a[01]` with the string `01`).
        "subscript_expression" => {
            let children: Vec<_> = named_children(node).collect();
            let [object, index] = children.as_slice() else {
                return None;
            };
            out.push(Tok::Open("subscript_expression".into()));
            walk(object, context, out)?;
            let text = index.text();
            match index.kind().as_ref() {
                "name" => out.push(Tok::Bytes(text.as_bytes().to_vec())),
                "integer" | "unary_op_expression" => {
                    let digits = text.split_whitespace().collect::<String>();
                    match php::canonical_int(digits.as_bytes()) {
                        Some(value) => out.push(Tok::BigInt(value.to_string())),
                        None => out.push(Tok::Bytes(digits.into_bytes())),
                    }
                }
                _ => walk(index, context, out)?,
            }
            out.push(Tok::Close);
            Some(())
        }
        _ => walk(node, context, out),
    }
}

#[cfg(test)]
mod tests;
