//! Structural equality of Python expressions and statements, as Ruff 0.16.9 compares them
//! (`ruff_python_ast::comparable`). A node reads as a sequence of tokens, one per AST node or leaf,
//! with parentheses, comments, commas and other trivia dropped; two nodes are equal when their
//! sequences are. Literals read as values of their own type (`1 == 0x1`, `'a' == "a"`, but
//! `1 != 1.0 != True`), strings with escapes decoded and implicit concatenation joined.
//!
//! Three equalities, one per compare text mode:
//! - `Plain` is `ComparableExpr` (PIE796, SIM101) and `ComparableStmt` (SIM114).
//! - `Hash` is `HashableExpr` (F601, B033): numbers fold by value as Python hashes them
//!   (`1 == 1.0 == True`, `1 + 0j == 1`), tuples of constants element by element, and anything
//!   else structurally.
//! - `Parts` is `ComparableLiteral` (SIM116): a string or bytes literal compares its implicitly
//!   concatenated parts one by one (`"a" "b" != "ab" ""`); anything else structurally.

use super::Context;
use super::python::{Piece, Prefix, decode_body, prefix};
use crate::code_rule_dsl::identifier::python_name;
use ast_grep_core::{Doc, Node};

/// Which of Ruff's equalities a comparison applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Equality {
    Plain,
    Hash,
    Parts,
}

/// One token of a node's structure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Tok {
    Open(String),
    Close,
    /// A named leaf (an identifier, NFKC-normalized as Ruff's AST holds it; a keyword argument's
    /// name): its kind and text.
    Leaf(String, String),
    /// An anonymous token (an operator, a keyword): its kind.
    Token(String),
    /// An empty slot of a slice (`x[:1]` has no lower bound).
    Missing,
    /// An `int` below 2**64.
    Int(u64),
    /// An `int` of 2**64 or more: Ruff keeps its token, underscores dropped for a decimal one.
    BigInt(String),
    Float(u64),
    Complex(u64, u64),
    Str(String),
    Bytes(Vec<u8>),
    /// A number folded as `HashableExpr` folds it: real and imaginary parts.
    Number(Real, Real),
    /// Debug text of an f-string replacement field (`{x = }`).
    Debug(String),
}

/// A real part of a folded number: an integer when the value is one below 2**64, else its bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Real {
    Integer(i128),
    Float(u64),
}

impl Real {
    fn from_float(value: f64) -> Self {
        const BOUND: f64 = 18_446_744_073_709_551_616.0;
        if value.is_finite() && value.abs() < BOUND {
            let integer = value as i128;
            if integer as f64 == value {
                return Self::Integer(integer);
            }
        }
        Self::Float(value.to_bits())
    }

    fn into_float(self) -> Self {
        match self {
            Self::Integer(integer) => Self::from_float(integer as f64),
            Self::Float(_) => self,
        }
    }

    fn negate(self) -> Self {
        match self {
            Self::Integer(integer) => Self::Integer(-integer),
            Self::Float(bits) => Self::Float(bits ^ (1 << 63)),
        }
    }
}

/// The node's structure under an equality; `None` where it is not known (a syntax error, a string
/// that does not decode, nesting past the decoder's depth, a spent budget).
pub(crate) fn structure<D: Doc>(
    node: &Node<'_, D>,
    equality: Equality,
    context: &mut Context<'_>,
) -> Option<Vec<Tok>> {
    let node = unparenthesized(node);
    let mut out = Vec::new();
    let known = match equality {
        Equality::Plain => plain(&node, context, &mut out),
        Equality::Hash => hashed(&node, context, &mut out),
        Equality::Parts => match literal_parts(&node) {
            Some(parts) => {
                out.push(Tok::Open("parts".into()));
                out.extend(parts);
                out.push(Tok::Close);
                Some(())
            }
            None => plain(&node, context, &mut out),
        },
    };
    known.map(|()| out)
}

/// Named children other than comments and line continuations.
fn operands<'r, D: Doc>(node: &Node<'r, D>) -> Vec<Node<'r, D>> {
    node.children()
        .filter(|child| child.is_named() && !trivia(child))
        .collect()
}

fn trivia<D: Doc>(node: &Node<'_, D>) -> bool {
    matches!(node.kind().as_ref(), "comment" | "line_continuation")
}

/// The expression inside any number of parentheses.
fn unparenthesized<'r, D: Doc>(node: &Node<'r, D>) -> Node<'r, D> {
    let mut node = node.clone();
    while node.kind() == "parenthesized_expression" {
        match operands(&node)[..] {
            [ref inner] => node = inner.clone(),
            _ => break,
        }
    }
    node
}

fn plain<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>, out: &mut Vec<Tok>) -> Option<()> {
    if !context.enter() {
        return None;
    }
    let result = plain_node(node, context, out);
    context.leave();
    if context.exhausted() {
        return None;
    }
    result
}

fn plain_node<D: Doc>(
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
                return plain(&inner, context, out);
            }
        }
        "integer" | "float" => {
            out.push(number(&node.text())?);
            return Some(());
        }
        "string" | "concatenated_string" => return string(node, context, out),
        // A bare tuple and the target lists of assignments and loops are tuples; a bracketed
        // target list is a list.
        "expression_list" | "pattern_list" | "tuple_pattern" => {
            return sequence("tuple", &operands(node), context, out);
        }
        "list_pattern" => return sequence("list", &operands(node), context, out),
        "expression_statement" => {
            let children = operands(node);
            if children.len() > 1 {
                out.push(Tok::Open("expression_statement".into()));
                sequence("tuple", &children, context, out)?;
                out.push(Tok::Close);
                return Some(());
            }
        }
        "boolean_operator" => return boolean(node, context, out),
        "subscript" => return subscript(node, context, out),
        "slice" => return slice(node, context, out),
        "call" => return call(node, context, out),
        _ => {}
    }
    if !node.is_named() {
        if !punctuation(&kind) {
            out.push(Tok::Token(kind.into_owned()));
        }
        return Some(());
    }
    let children = node
        .children()
        .filter(|child| !trivia(child))
        .collect::<Vec<_>>();
    if children.is_empty() {
        let text = node.text();
        let text = if kind == "identifier" {
            python_name(text)
        } else {
            text
        };
        out.push(Tok::Leaf(kind.into_owned(), text.into_owned()));
        return Some(());
    }
    out.push(Tok::Open(kind.into_owned()));
    for child in &children {
        plain(child, context, out)?;
    }
    out.push(Tok::Close);
    Some(())
}

/// Tokens the tree's shape already records: separators and brackets.
fn punctuation(kind: &str) -> bool {
    matches!(kind, "," | ";" | "(" | ")" | "[" | "]" | "{" | "}" | ":")
}

fn sequence<D: Doc>(
    kind: &str,
    items: &[Node<'_, D>],
    context: &mut Context<'_>,
    out: &mut Vec<Tok>,
) -> Option<()> {
    out.push(Tok::Open(kind.into()));
    for item in items {
        plain(item, context, out)?;
    }
    out.push(Tok::Close);
    Some(())
}

/// `a and b and c` is one operation over three operands, as Ruff parses it; a parenthesized
/// operand stays an operation of its own.
fn boolean<D: Doc>(
    node: &Node<'_, D>,
    context: &mut Context<'_>,
    out: &mut Vec<Tok>,
) -> Option<()> {
    let operator = node.field("operator")?.kind().into_owned();
    let mut operands = Vec::new();
    let mut current = node.clone();
    loop {
        let (left, right) = (current.field("left")?, current.field("right")?);
        operands.push(right);
        let same = left.kind() == "boolean_operator"
            && left
                .field("operator")
                .is_some_and(|op| op.kind() == operator.as_str());
        if same {
            current = left;
        } else {
            operands.push(left);
            break;
        }
    }
    operands.reverse();
    out.push(Tok::Open("boolean_operator".into()));
    out.push(Tok::Token(operator));
    for operand in &operands {
        plain(operand, context, out)?;
    }
    out.push(Tok::Close);
    Some(())
}

/// `x[1, 2]` and `x[1,]` index with a tuple, as `x[(1, 2)]` does.
fn subscript<D: Doc>(
    node: &Node<'_, D>,
    context: &mut Context<'_>,
    out: &mut Vec<Tok>,
) -> Option<()> {
    let value = node.field("value")?;
    let indices = node.field_children("subscript").collect::<Vec<_>>();
    let comma = node.children().any(|child| child.kind() == ",");
    out.push(Tok::Open("subscript".into()));
    plain(&value, context, out)?;
    match &indices[..] {
        [index] if !comma => plain(index, context, out)?,
        _ => sequence("tuple", &indices, context, out)?,
    }
    out.push(Tok::Close);
    Some(())
}

/// A slice's three slots, each an expression or empty (`x[1:]`, `x[:1]`, `x[::2]`).
fn slice<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>, out: &mut Vec<Tok>) -> Option<()> {
    let mut slots: [Option<Node<'_, D>>; 3] = [None, None, None];
    let mut slot = 0;
    for child in node.children().filter(|child| !trivia(child)) {
        if child.kind() == ":" {
            slot += 1;
        } else if child.is_named() && slot < 3 {
            slots[slot] = Some(child);
        }
    }
    out.push(Tok::Open("slice".into()));
    for slot in &slots {
        match slot {
            Some(bound) => plain(bound, context, out)?,
            None => out.push(Tok::Missing),
        }
    }
    out.push(Tok::Close);
    Some(())
}

/// A call's positional arguments, then its keyword arguments (`f(k=1, *a)` is `f(*a, k=1)`); a
/// bare generator argument is an argument.
fn call<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>, out: &mut Vec<Tok>) -> Option<()> {
    let (function, arguments) = (node.field("function")?, node.field("arguments")?);
    out.push(Tok::Open("call".into()));
    plain(&function, context, out)?;
    out.push(Tok::Open("argument_list".into()));
    if arguments.kind() == "argument_list" {
        let items = operands(&arguments);
        let keyword = |item: &Node<'_, D>| {
            matches!(
                item.kind().as_ref(),
                "keyword_argument" | "dictionary_splat"
            )
        };
        for item in items.iter().filter(|item| !keyword(item)) {
            plain(item, context, out)?;
        }
        for item in items.iter().filter(|item| keyword(item)) {
            plain(item, context, out)?;
        }
    } else {
        plain(&arguments, context, out)?;
    }
    out.push(Tok::Close);
    out.push(Tok::Close);
    Some(())
}

/// An `int`, `float` or imaginary literal as Ruff's parser reads it.
fn number(text: &str) -> Option<Tok> {
    let digits = text.replace('_', "");
    if let Some(imaginary) = digits.strip_suffix(['j', 'J']) {
        return Some(Tok::Complex(
            0.0f64.to_bits(),
            imaginary.parse::<f64>().ok()?.to_bits(),
        ));
    }
    let lower = digits.to_ascii_lowercase();
    let radix = if lower.starts_with("0x") {
        16
    } else if lower.starts_with("0o") {
        8
    } else if lower.starts_with("0b") {
        2
    } else if lower.contains(['.', 'e']) {
        return Some(Tok::Float(digits.parse::<f64>().ok()?.to_bits()));
    } else {
        10
    };
    let body = if radix == 10 {
        &digits[..]
    } else {
        &digits[2..]
    };
    match u64::from_str_radix(body, radix) {
        Ok(value) => Some(Tok::Int(value)),
        // Ruff keeps a radix literal's whole token and a decimal one's digits.
        Err(_) if !body.is_empty() && body.chars().all(|c| c.is_digit(radix)) => {
            Some(Tok::BigInt(if radix == 10 {
                digits
            } else {
                text.to_owned()
            }))
        }
        Err(_) => None,
    }
}

/// One part of a string: its prefix and body.
fn split_part(text: &str) -> Option<(Prefix, &str)> {
    let quote_at = text.find(['\'', '"'])?;
    let prefix = prefix(&text[..quote_at])?;
    let rest = &text[quote_at..];
    let quote = if rest.starts_with("'''") || rest.starts_with("\"\"\"") {
        3
    } else {
        1
    };
    (rest.len() >= 2 * quote).then(|| (prefix, &rest[quote..rest.len() - quote]))
}

fn parts<'r, D: Doc>(node: &Node<'r, D>) -> Vec<Node<'r, D>> {
    if node.kind() == "concatenated_string" {
        node.children()
            .filter(|child| child.kind() == "string")
            .collect()
    } else {
        vec![node.clone()]
    }
}

/// A plain string or bytes literal's parts, each decoded (`ComparableLiteral`); `None` for an f- or
/// t-string or anything else.
fn literal_parts<D: Doc>(node: &Node<'_, D>) -> Option<Vec<Tok>> {
    if !matches!(node.kind().as_ref(), "string" | "concatenated_string") {
        return None;
    }
    let mut out = Vec::new();
    for part in parts(node) {
        let text = part.text();
        let (prefix, body) = split_part(&text)?;
        if prefix.format || prefix.template {
            return None;
        }
        out.push(match decode_body(body, &prefix)? {
            Piece::Str(value) => Tok::Str(value),
            Piece::Bytes(value) => Tok::Bytes(value),
        });
    }
    Some(out)
}

/// A string: a plain literal joined into one value; an f-string (or t-string) as its literal text,
/// joined across parts, and its replacement fields.
fn string<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>, out: &mut Vec<Tok>) -> Option<()> {
    let parts = parts(node);
    let mut prefixes = Vec::with_capacity(parts.len());
    for part in &parts {
        let text = part.text();
        prefixes.push(split_part(&text)?.0);
    }
    let template = prefixes.iter().any(|prefix| prefix.template);
    let format = prefixes.iter().any(|prefix| prefix.format);
    if !template && !format {
        let mut joined: Option<Piece> = None;
        for part in &parts {
            let text = part.text();
            let (prefix, body) = split_part(&text)?;
            joined = Some(match (joined, decode_body(body, &prefix)?) {
                (None, next) => next,
                (Some(Piece::Str(mut left)), Piece::Str(right)) => {
                    left.push_str(&right);
                    Piece::Str(left)
                }
                (Some(Piece::Bytes(mut left)), Piece::Bytes(right)) => {
                    left.extend(right);
                    Piece::Bytes(left)
                }
                _ => return None,
            });
        }
        out.push(match joined? {
            Piece::Str(value) => Tok::Str(value),
            Piece::Bytes(value) => Tok::Bytes(value),
        });
        return Some(());
    }
    // Ruff joins adjacent literal text into one element; a plain part is an element even when it
    // is empty. A template records its (possibly empty) text before, between and after fields.
    out.push(Tok::Open(
        if template { "tstring" } else { "fstring" }.into(),
    ));
    let mut text: Option<String> = template.then(String::new);
    for (part, prefix) in parts.iter().zip(&prefixes) {
        if !prefix.format && !prefix.template {
            let source = part.text();
            let (_, body) = split_part(&source)?;
            match decode_body(body, prefix)? {
                Piece::Str(value) => text.get_or_insert_with(String::new).push_str(&value),
                Piece::Bytes(_) => return None,
            }
            continue;
        }
        for child in part.children() {
            match child.kind().as_ref() {
                "string_content" => match decode_body(&child.text(), prefix)? {
                    Piece::Str(value) => text.get_or_insert_with(String::new).push_str(&value),
                    Piece::Bytes(_) => return None,
                },
                "interpolation" => {
                    if let Some(literal) = text.take() {
                        out.push(Tok::Str(literal));
                    }
                    field(&child, context, out)?;
                    if template {
                        text = Some(String::new());
                    }
                }
                "string_start" | "string_end" => {}
                _ => return None,
            }
        }
    }
    if let Some(literal) = text {
        out.push(Tok::Str(literal));
    }
    out.push(Tok::Close);
    Some(())
}

/// A replacement field: its expression, debug text (`=`, with the white space around it), conversion
/// and format spec (literal text and nested fields).
fn field<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>, out: &mut Vec<Tok>) -> Option<()> {
    out.push(Tok::Open("field".into()));
    let expression = node.field("expression")?;
    plain(&expression, context, out)?;
    let children = node.children().collect::<Vec<_>>();
    if let Some(equals) = children.iter().position(|child| child.kind() == "=") {
        // From after `{` to the next part of the field, newlines as Python reads them.
        let start = children.first()?.range().end;
        let end = children.get(equals + 1)?.range().start;
        let text = node.text();
        let offset = node.range().start;
        let debug = text.get(start - offset..end - offset)?;
        out.push(Tok::Debug(debug.replace("\r\n", "\n").replace('\r', "\n")));
    }
    for child in &children {
        match child.kind().as_ref() {
            "type_conversion" => out.push(Tok::Token(child.text().into_owned())),
            "format_specifier" => {
                out.push(Tok::Open("spec".into()));
                let text = child.text();
                let base = child.range().start;
                let mut at = child
                    .children()
                    .next()
                    .map_or(0, |colon| colon.range().end - base);
                for nested in child
                    .children()
                    .filter(|nested| nested.kind() == "format_expression")
                {
                    let range = nested.range();
                    let literal = text.get(at..range.start - base)?;
                    if !literal.is_empty() {
                        out.push(Tok::Str(literal.replace("{{", "{").replace("}}", "}")));
                    }
                    field(&nested, context, out)?;
                    at = range.end - base;
                }
                let literal = text.get(at..)?;
                if !literal.is_empty() {
                    out.push(Tok::Str(literal.replace("{{", "{").replace("}}", "}")));
                }
                out.push(Tok::Close);
            }
            _ => {}
        }
    }
    out.push(Tok::Close);
    Some(())
}

/// `HashableExpr`: a constant folded by value, a named expression by its target and folded value,
/// anything else structurally.
fn hashed<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>, out: &mut Vec<Tok>) -> Option<()> {
    let node = unparenthesized(node);
    if let Some(constant) = constant(&node, context) {
        out.extend(constant?);
        return Some(());
    }
    if node.kind() == "named_expression" {
        out.push(Tok::Open("named".into()));
        plain(&node.field("name")?, context, out)?;
        hashed(&node.field("value")?, context, out)?;
        out.push(Tok::Close);
        return Some(());
    }
    plain(&node, context, out)
}

/// A constant's tokens: `None` when the node is no constant, `Some(None)` when it is one whose
/// structure is not known.
fn constant<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Option<Option<Vec<Tok>>> {
    let node = unparenthesized(node);
    if let Some((real, imag)) = folded(&node) {
        return Some(Some(vec![Tok::Number(real, imag)]));
    }
    match node.kind().as_ref() {
        "tuple" | "expression_list" => {
            let mut out = vec![Tok::Open("constant-tuple".into())];
            for item in operands(&node) {
                match constant(&item, context)? {
                    Some(tokens) => out.extend(tokens),
                    None => return Some(None),
                }
            }
            out.push(Tok::Close);
            Some(Some(out))
        }
        // Ruff's other literals: strings, bytes, ints past 2**64, `None`, `...`; not f-strings.
        "string" | "concatenated_string" if literal_parts(&node).is_some() => {
            let mut out = Vec::new();
            Some(plain(&node, context, &mut out).map(|()| out))
        }
        "integer" | "none" | "ellipsis" => {
            let mut out = Vec::new();
            Some(plain(&node, context, &mut out).map(|()| out))
        }
        _ => None,
    }
}

/// A number `HashableExpr` folds: a boolean, an `int` below 2**64, a `float`, an imaginary literal,
/// a signed one, or a real one plus or minus an imaginary literal.
fn folded<D: Doc>(node: &Node<'_, D>) -> Option<(Real, Real)> {
    let node = unparenthesized(node);
    let zero = Real::Integer(0);
    match node.kind().as_ref() {
        "true" => Some((Real::Integer(1), zero)),
        "false" => Some((zero, zero)),
        "integer" | "float" => match number(&node.text())? {
            Tok::Int(value) => Some((Real::Integer(i128::from(value)), zero)),
            Tok::Float(bits) => Some((Real::from_float(f64::from_bits(bits)), zero)),
            Tok::Complex(real, imag) => Some((
                Real::from_float(f64::from_bits(real)),
                Real::from_float(f64::from_bits(imag)),
            )),
            _ => None,
        },
        "unary_operator" => {
            let (real, imag) = folded(&node.field("argument")?)?;
            match node.field("operator")?.kind().as_ref() {
                "+" => Some((real, imag)),
                "-" => Some((real.negate(), imag.negate())),
                _ => None,
            }
        }
        "binary_operator" => {
            let operator = node.field("operator")?.kind().into_owned();
            if !matches!(operator.as_ref(), "+" | "-") {
                return None;
            }
            let (real, imag) = folded(&node.field("left")?)?;
            if imag != zero {
                return None;
            }
            let right = unparenthesized(&node.field("right")?);
            if !matches!(right.kind().as_ref(), "integer" | "float") {
                return None;
            }
            let Tok::Complex(right_real, right_imag) = number(&right.text())? else {
                return None;
            };
            if Real::from_float(f64::from_bits(right_real)) != zero {
                return None;
            }
            let imag = Real::from_float(f64::from_bits(right_imag));
            let imag = if operator == "-" { imag.negate() } else { imag };
            Some((real.into_float(), imag))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
