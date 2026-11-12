//! ECMAScript's conversions between numbers and strings, exactly: `Number::toString`,
//! StringToNumber, StringToBigInt, ToInt32 and ToUint32; and ESLint's static property names.

use crate::code_rule_dsl::tree::Tree;
use crate::code_rule_dsl::language::Family;

use super::ecma::{cooked, number, operand, ops, string_node};
use super::{BigInt, Value};
use ast_grep_core::{Doc, Node};
use num_traits::Zero;

/// ESLint's `getStaticPropertyName` (`lib/rules/utils/ast-utils.js`), as UTF-16: the name of a
/// property, method, field, signature or member access, or of a key node itself. A plain key is
/// its name (identifier escapes decoded); a literal or computed literal key is `String(value)`;
/// a template without substitutions its cooked string. A computed key's expression, read by
/// itself, is what its property is named (`getStaticStringValue`: `` `a` `` is `a`, `/a/g` is
/// `/a/g`, `0x1` is `1`). Anything else, a computed expression or a private name included, is not
/// known (`None`).
pub(crate) fn static_property_name<D: Doc>(node: &Node<'_, D>) -> Option<Vec<u16>> {
    let (key, computed) = match node.kind().as_ref() {
        "pair" | "pair_pattern" => (node.field("key")?, false),
        "method_definition"
        | "public_field_definition"
        | "method_signature"
        | "property_signature" => (node.field("name")?, false),
        "field_definition" => (node.field("property")?, false),
        "member_expression" => (node.field("property")?, false),
        "subscript_expression" => (node.field("index")?, true),
        // ESTree has no parentheses: `(('m'))` is the literal.
        _ => (unparenthesized(node.clone()), false),
    };
    match key.kind().as_ref() {
        "property_identifier"
        | "shorthand_property_identifier"
        | "shorthand_property_identifier_pattern"
            if !computed =>
        {
            identifier_name(&key.text())
        }
        "computed_property_name" => static_string(&unparenthesized(operand(&key)?)),
        _ => static_string(&unparenthesized(key)),
    }
}

/// The name ESTree gives a node, as UTF-16: an identifier's (`Identifier.name`, escapes decoded; a
/// private name's `PrivateIdentifier.name`, without its `#`) or a string literal's value
/// (`Literal.value`). ESLint rules compare these across the two (func-name-matching's string key
/// and function name, no-useless-rename's key and local name). `None` for any other node.
pub(crate) fn node_name<D: Doc>(node: &Node<'_, D>) -> Option<Vec<u16>> {
    match node.kind().as_ref() {
        "identifier"
        | "property_identifier"
        | "shorthand_property_identifier"
        | "shorthand_property_identifier_pattern"
        | "type_identifier"
        | "statement_identifier"
        | "undefined" => identifier_name(&node.text()),
        "private_property_identifier" => identifier_name(node.text().strip_prefix('#')?),
        "string" => string_node(node),
        _ => None,
    }
}

/// ESLint's `isSameReference` (`lib/rules/utils/ast-utils.js`) over two expressions as ESTree has
/// them: parentheses and optional chains aside, `this` and `super` each the same, identifiers and
/// private names by name, literals by `equalLiteralValue` (a regex by pattern and flags as written,
/// a BigInt by value, anything else by `===`), and member accesses by object and property. With
/// `static_keys` (the default there), a member access with a static name is the same as one of the
/// same name on the same object (`a.b` and `a['b']`); without, computed and static accesses
/// differ. Any other expression is never the same. `None` where a literal does not decode.
pub(crate) fn same_reference<D: Doc>(
    left: &Node<'_, D>,
    right: &Node<'_, D>,
    static_keys: bool,
) -> Option<bool> {
    let (left, right) = (
        unparenthesized(left.clone()),
        unparenthesized(right.clone()),
    );
    let kind = |node: &Node<'_, D>| -> &'static str {
        match node.kind().as_ref() {
            "this" => "this",
            "super" => "super",
            "identifier" | "undefined" | "property_identifier" => "identifier",
            "private_property_identifier" => "private",
            "string" | "number" | "true" | "false" | "null" | "regex" => "literal",
            "member_expression" | "subscript_expression" => "member",
            _ => "other",
        }
    };
    let (left_kind, right_kind) = (kind(&left), kind(&right));
    if left_kind != right_kind {
        return Some(false);
    }
    match left_kind {
        "this" | "super" => Some(true),
        "identifier" | "private" => Some(node_name(&left)? == node_name(&right)?),
        "literal" => equal_literal_value(&left, &right),
        "member" => {
            if static_keys && let Some(name) = static_property_name(&left) {
                let objects =
                    same_reference(&left.field("object")?, &right.field("object")?, true)?;
                return Some(
                    objects && static_property_name(&right).is_some_and(|other| other == name),
                );
            }
            fn property<'r, D: Doc>(node: &Node<'r, D>) -> Option<Node<'r, D>> {
                node.field(if node.kind() == "subscript_expression" {
                    "index"
                } else {
                    "property"
                })
            }
            Some(
                left.kind() == right.kind()
                    && same_reference(
                        &left.field("object")?,
                        &right.field("object")?,
                        static_keys,
                    )?
                    && same_reference(&property(&left)?, &property(&right)?, static_keys)?,
            )
        }
        _ => Some(false),
    }
}

/// ESLint's `equalLiteralValue`: regexes by pattern and flags as written, BigInts by value
/// (typescript-estree's `bigint`), anything else by `===` on the value.
fn equal_literal_value<D: Doc>(left: &Node<'_, D>, right: &Node<'_, D>) -> Option<bool> {
    let (left_kind, right_kind) = (left.kind(), right.kind());
    if left_kind == "regex" || right_kind == "regex" {
        return Some(left_kind == right_kind && left.text() == right.text());
    }
    let bigint = |node: &Node<'_, D>| -> Option<Option<BigInt>> {
        let text = node.text();
        if node.kind() != "number" || !text.ends_with('n') {
            return Some(None);
        }
        match number(&text) {
            Value::BigInt(value) => Some(Some(BigInt::from(value))),
            Value::Unknown => None,
            _ => Some(None),
        }
    };
    match (bigint(left)?, bigint(right)?) {
        (Some(left), Some(right)) => return Some(left == right),
        (None, None) => {}
        _ => return Some(false),
    }
    let value = |node: &Node<'_, D>| -> Option<Value> {
        Some(match node.kind().as_ref() {
            "string" => Value::Utf16(string_node(node)?),
            "number" => match number(&node.text()) {
                Value::Unknown => return None,
                value => value,
            },
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => Value::None,
        })
    };
    let (left, right) = (value(left)?, value(right)?);
    Some(match (&left, &right) {
        (Value::Float(left), Value::Float(right)) => left == right,
        _ => left.equals(&right, Family::Ecma)?,
    })
}

/// typescript-eslint's `getStaticMemberAccessValue` (`util/misc.ts`), as UTF-16: the name of a
/// non-computed identifier or private key (`#a` is `a`), and otherwise `String(v)` of what
/// `value` folds the key to (eslint-utils' `getStaticValue` in the node's scope: `['f' + '']` is
/// `f`). It reads a property, method, field, signature or member access, or a key node itself.
/// `None` where the key's value is not known.
pub(crate) fn member_access_name<'r, D: Doc>(
    node: &Node<'r, D>,
    value: &mut dyn FnMut(&Node<'r, D>) -> Value,
) -> Option<Vec<u16>> {
    let (key, computed) = match node.kind().as_ref() {
        "pair" | "pair_pattern" => (node.field("key")?, false),
        "method_definition"
        | "public_field_definition"
        | "method_signature"
        | "property_signature" => (node.field("name")?, false),
        "field_definition" => (node.field("property")?, false),
        "member_expression" => (node.field("property")?, false),
        "subscript_expression" => (node.field("index")?, true),
        _ => (node.clone(), false),
    };
    match key.kind().as_ref() {
        "property_identifier"
        | "shorthand_property_identifier"
        | "shorthand_property_identifier_pattern"
            if !computed =>
        {
            identifier_name(&key.text())
        }
        "private_property_identifier" if !computed => {
            identifier_name(key.text().strip_prefix('#')?)
        }
        "computed_property_name" => ops::to_string(&value(&operand(&key)?)),
        _ => ops::to_string(&value(&key)),
    }
}

#[cfg(any(test, feature = "test-hooks"))]
/// The key node that ESLint's `key` or `property` is: a computed name without its brackets and
/// parentheses, since ESTree has neither.
pub(crate) fn key_node<'r, D: Doc>(node: &Node<'r, D>) -> Option<Node<'r, D>> {
    let key = match node.kind().as_ref() {
        "pair" | "pair_pattern" => node.field("key")?,
        "method_definition"
        | "public_field_definition"
        | "method_signature"
        | "property_signature" => node.field("name")?,
        "field_definition" => node.field("property")?,
        "member_expression" => node.field("property")?,
        "subscript_expression" => node.field("index")?,
        "shorthand_property_identifier" | "shorthand_property_identifier_pattern" => node.clone(),
        _ => return None,
    };
    Some(if key.kind() == "computed_property_name" {
        unparenthesized(operand(&key)?)
    } else {
        unparenthesized(key)
    })
}

fn unparenthesized<'r, D: Doc>(mut node: Node<'r, D>) -> Node<'r, D> {
    while node.kind() == "parenthesized_expression" {
        match operand(&node) {
            Some(inner) => node = inner,
            None => break,
        }
    }
    node
}

/// ESLint's `getStaticStringValue`: a literal's `String(value)`, a regex's `/pattern/flags`, a
/// template without substitutions' cooked string.
fn static_string<D: Doc>(node: &Node<'_, D>) -> Option<Vec<u16>> {
    let utf16 = |text: &str| Some(text.encode_utf16().collect());
    match node.kind().as_ref() {
        "string" => string_node(node),
        "number" => match number(&node.text()) {
            Value::Float(value) => utf16(&number_to_string(value)),
            Value::BigInt(value) => utf16(&value.to_string()),
            _ => None,
        },
        "true" | "false" | "null" => utf16(&node.text()),
        "regex" => utf16(&node.text()),
        "template_string" => {
            if node
                .children()
                .any(|child| child.kind() == "template_substitution")
            {
                return None;
            }
            let text = node.text();
            cooked(text.strip_prefix('`')?.strip_suffix('`')?)
        }
        _ => None,
    }
}

/// An identifier's name, `\uXXXX` and `\u{...}` escapes decoded.
pub(crate) fn identifier_name(text: &str) -> Option<Vec<u16>> {
    let mut out = Vec::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut buffer = [0u16; 2];
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.extend_from_slice(c.encode_utf16(&mut buffer));
            continue;
        }
        if chars.next()? != 'u' {
            return None;
        }
        let digits = if chars.peek() == Some(&'{') {
            chars.next();
            chars.by_ref().take_while(|c| *c != '}').collect::<String>()
        } else {
            chars.by_ref().take(4).collect::<String>()
        };
        let code = u32::from_str_radix(&digits, 16).ok()?;
        out.extend_from_slice(char::from_u32(code)?.encode_utf16(&mut buffer));
    }
    Some(out)
}

// The DSL's number text, which is ECMAScript's (`matcher::transform`, the engine's).
pub use crate::code_rule_dsl::matcher::transform::number_to_string;

/// `StrWhiteSpaceChar`: ECMAScript white space and line terminators.
fn is_str_white_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\u{b}'
            | '\u{c}'
            | ' '
            | '\u{a0}'
            | '\u{feff}'
            | '\n'
            | '\r'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

fn trimmed(units: &[u16]) -> Option<String> {
    let text = String::from_utf16(units).ok()?;
    Some(text.trim_matches(is_str_white_space).to_owned())
}

/// A `0x`, `0o` or `0b` literal without separators or sign.
fn radix_integer(text: &str) -> Option<(u32, &str)> {
    let lower = text.get(..2)?.to_ascii_lowercase();
    let radix = match lower.as_str() {
        "0x" => 16,
        "0o" => 8,
        "0b" => 2,
        _ => return None,
    };
    let digits = &text[2..];
    (!digits.is_empty() && digits.chars().all(|c| c.is_digit(radix))).then_some((radix, digits))
}

/// StringToNumber: white space trimmed; empty is 0; `Infinity`, decimals and `0x`/`0o`/`0b`
/// integers read; anything else is NaN. `None` only for a radix integer past 128 bits.
pub(crate) fn string_to_number(units: &[u16]) -> Option<f64> {
    let Some(text) = trimmed(units) else {
        return Some(f64::NAN);
    };
    if text.is_empty() {
        return Some(0.0);
    }
    if let Some((radix, digits)) = radix_integer(&text) {
        return u128::from_str_radix(digits, radix)
            .ok()
            .map(|value| value as f64);
    }
    let (sign, unsigned) = match text.as_bytes()[0] {
        b'+' => (1.0, &text[1..]),
        b'-' => (-1.0, &text[1..]),
        _ => (1.0, text.as_str()),
    };
    if unsigned == "Infinity" {
        return Some(sign * f64::INFINITY);
    }
    // StrUnsignedDecimalLiteral: digits, an optional fraction, an optional exponent.
    let bytes = unsigned.as_bytes();
    let mut index = 0;
    let digits = |index: &mut usize| {
        let start = *index;
        while *index < bytes.len() && bytes[*index].is_ascii_digit() {
            *index += 1;
        }
        *index - start
    };
    let mut count = digits(&mut index);
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        count += digits(&mut index);
    }
    if count == 0 {
        return Some(f64::NAN);
    }
    if index < bytes.len() && matches!(bytes[index], b'e' | b'E') {
        index += 1;
        if index < bytes.len() && matches!(bytes[index], b'+' | b'-') {
            index += 1;
        }
        if digits(&mut index) == 0 {
            return Some(f64::NAN);
        }
    }
    if index != bytes.len() {
        return Some(f64::NAN);
    }
    Some(sign * unsigned.parse::<f64>().unwrap_or(f64::NAN))
}

/// StringToBigInt, exactly: `None` where it is undefined (not an integer literal).
pub(crate) fn string_to_bigint(units: &[u16]) -> Option<BigInt> {
    let text = trimmed(units)?;
    if text.is_empty() {
        return Some(BigInt::zero());
    }
    if let Some((radix, digits)) = radix_integer(&text) {
        return BigInt::parse_bytes(digits.as_bytes(), radix);
    }
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(&text);
    if unsigned.is_empty() || !unsigned.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let magnitude = BigInt::parse_bytes(unsigned.as_bytes(), 10)?;
    Some(if text.starts_with('-') {
        -magnitude
    } else {
        magnitude
    })
}

/// ToUint32: the number truncated, modulo 2^32; NaN and infinities are 0.
pub(crate) fn to_uint32(value: f64) -> u32 {
    if !value.is_finite() {
        return 0;
    }
    value.trunc().rem_euclid(4_294_967_296.0) as u32
}

/// ToInt32: ToUint32 read as two's complement.
pub(crate) fn to_int32(value: f64) -> i32 {
    to_uint32(value) as i32
}

#[cfg(test)]
pub mod tests;
