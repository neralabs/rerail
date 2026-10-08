//! PHP values (tree-sitter-php 0.24.2, PHP-only grammar): PHP 8.5's constant expressions, valued
//! as PHP evaluates them. An expression is known only where `const X = <expr>;` accepts it and
//! evaluates it without error; anything PHP refuses there (variables, calls, `new`, interpolated
//! strings, `match`) or fails to evaluate (`1 / 0`, `"abc" + 1`, `1 << -1`) is `Unknown`. Under
//! `value.follow` a variable is its binding's value, and a constant this file declares once (a
//! `const`, a top-level `define`, a class constant of a class-like declared here, as
//! `PhpNames::constant_value` finds it) is its declaration's value. Other named constants
//! (`PHP_EOL`, `__LINE__`, a constant another file may declare) are `Unknown`. Integers are
//! 64-bit, as PHP's are where Rerail runs; strings are bytes; arrays are ordered maps whose keys
//! PHP coerces. The oracle is PHP 8.5.11 (`fixtures/php/value/generate_php.php`); PHPStan
//! 2.2.16 reads the constants followed (`fixtures/php/format_args/phpstan/constants.php`).

use super::{BigInt, ContainerKind, Context, Value};
use crate::code_rule_dsl::value::NodeRef;
use ast_grep_core::{Doc, Node};
use num_traits::ToPrimitive;
use std::cmp::Ordering;

/// Expressions: the subtypes of the grammar's `expression`, `primary_expression` and `literal`
/// supertypes (tree-sitter-php 0.24.2 `node-types.json`). Only an expression has a value; on any
/// other node every value predicate fails.
pub(crate) const EXPRESSION_KINDS: &[&str] = &[
    "anonymous_function",
    "array_creation_expression",
    "arrow_function",
    "assignment_expression",
    "augmented_assignment_expression",
    "binary_expression",
    "boolean",
    "cast_expression",
    "class_constant_access_expression",
    "clone_expression",
    "conditional_expression",
    "dynamic_variable_name",
    "encapsed_string",
    "error_suppression_expression",
    "float",
    "function_call_expression",
    "heredoc",
    "include_expression",
    "include_once_expression",
    "integer",
    "match_expression",
    "member_access_expression",
    "member_call_expression",
    "name",
    "nowdoc",
    "null",
    "nullsafe_member_access_expression",
    "nullsafe_member_call_expression",
    "object_creation_expression",
    "parenthesized_expression",
    "print_intrinsic",
    "qualified_name",
    "reference_assignment_expression",
    "relative_name",
    "require_expression",
    "require_once_expression",
    "scoped_call_expression",
    "scoped_property_access_expression",
    "shell_command_expression",
    "string",
    "subscript_expression",
    "throw_expression",
    "unary_op_expression",
    "update_expression",
    "variable_name",
    "yield_expression",
];

/// Kinds whose value `decode` can know; any other expression is `Unknown`.
pub(crate) const VALUE_KINDS: &[&str] = &[
    "array_creation_expression",
    "binary_expression",
    "boolean",
    "cast_expression",
    "conditional_expression",
    "encapsed_string",
    "float",
    "heredoc",
    "integer",
    "nowdoc",
    "null",
    "parenthesized_expression",
    "string",
    "subscript_expression",
    "unary_op_expression",
];

/// A PHP array key: an integer, or a string that is not a canonical decimal integer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    Int(i64),
    Str(Vec<u8>),
}

/// A PHP number.
#[derive(Clone, Copy, Debug)]
enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    fn float(self) -> f64 {
        match self {
            Self::Int(value) => value as f64,
            Self::Float(value) => value,
        }
    }
}

/// Decodes `node` to its value; `Unknown` for anything that is not a constant expression this
/// knows.
pub(crate) fn decode<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    if !context.enter() {
        return Value::Unknown;
    }
    let value = if node.is_missing() {
        None
    } else {
        eval(node, context)
    };
    context.leave();
    match value {
        Some(value) if !context.exhausted() => value,
        _ => Value::Unknown,
    }
}

/// A value, or `None` where PHP refuses the expression or fails to evaluate it.
fn eval<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Option<Value> {
    match node.kind().as_ref() {
        "integer" => integer(&node.text()),
        "float" => float(&node.text()),
        "boolean" => Some(Value::Bool(node.text().eq_ignore_ascii_case("true"))),
        "null" => Some(Value::None),
        "string" => single_quoted(&node.text()).map(Value::PhpStr),
        "encapsed_string" => {
            if !literal_parts(node) {
                return Some(Value::Unknown);
            }
            double_quoted(&node.text()).map(Value::PhpStr)
        }
        "heredoc" | "nowdoc" => heredoc(node),
        "parenthesized_expression" => Some(decode(&only_child(node)?, context)),
        "unary_op_expression" => unary(node, context),
        "binary_expression" => binary(node, context),
        "conditional_expression" => conditional(node, context),
        "cast_expression" => unary(node, context),
        "array_creation_expression" => array(node, context),
        "subscript_expression" => subscript(node, context),
        "variable_name" => Some(follow(node, context)),
        "name" | "qualified_name" | "relative_name" | "class_constant_access_expression" => {
            Some(constant(node, context))
        }
        _ => Some(Value::Unknown),
    }
}

/// A variable's value through its binding (`value.follow`): the one binding every path reaches
/// the read with, where it assigns a value and the variable is no reference (`ScopeTable::binding`).
/// An array changed after it was assigned (`$a[] = 1`) is a binding of its own that assigns none.
fn follow<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    let Some(followed) = context.follow(node.range(), &node.text()) else {
        return Value::Unknown;
    };
    match followed.value.find_near(node.root().root(), node) {
        Some(target) => decode(&target, context),
        None => Value::Unknown,
    }
}

/// A constant's value through its declaration in this file (`value.follow`).
fn constant<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    let Some(value) = context.constant(NodeRef::of(node)) else {
        return Value::Unknown;
    };
    match value.find_near(node.root().root(), node) {
        Some(target) => decode(&target, context),
        None => Value::Unknown,
    }
}

/// Whether PHP knows the operand: `Some(value)` only for a whole value; an array with an unknown
/// element stops an operation as an unknown value does.
fn known(value: Value) -> Option<Value> {
    match value {
        Value::Unknown | Value::Container { .. } => None,
        value => Some(value),
    }
}

/// The array key PHP coerces the expression at `node` to (a destructuring's written key), where
/// its value is known and a valid key.
pub(crate) fn key<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Option<Key> {
    key_of(&known(decode(node, context))?)
}

fn named<'a, D: Doc>(node: &Node<'a, D>) -> impl Iterator<Item = Node<'a, D>> {
    node.children()
        .filter(|child| child.is_named() && child.kind() != "comment")
}

fn only_child<'a, D: Doc>(node: &Node<'a, D>) -> Option<Node<'a, D>> {
    let mut children = named(node);
    let child = children.next()?;
    children.next().is_none().then_some(child)
}

// ---------------------------------------------------------------------------------------------
// Literals
// ---------------------------------------------------------------------------------------------

/// The digits of a numeric literal without its `_` separators, which PHP allows only between two
/// digits.
fn digits(text: &str, digit: impl Fn(u8) -> bool) -> Option<String> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(bytes.len());
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == b'_' {
            let before = index.checked_sub(1).map(|i| bytes[i]);
            let after = bytes.get(index + 1).copied();
            if !before.is_some_and(&digit) || !after.is_some_and(&digit) {
                return None;
            }
        } else if digit(byte) {
            out.push(char::from(byte));
        } else {
            return None;
        }
    }
    Some(out)
}

/// An integer literal: decimal, `0x`, `0b`, `0o` or legacy-octal `0` digits with `_` separators
/// (legacy octal also after its leading `0`: `0_17`). One past `PHP_INT_MAX` is a float: decimal
/// ones read as `strtod` reads them, the others accumulated digit by digit in a float, as PHP's
/// lexer does.
fn integer(text: &str) -> Option<Value> {
    let (radix, body) = match text.as_bytes() {
        [b'0', b'x' | b'X', ..] => (16, &text[2..]),
        [b'0', b'b' | b'B', ..] => (2, &text[2..]),
        [b'0', b'o' | b'O', ..] => (8, &text[2..]),
        [b'0', b'_', ..] => (8, &text[2..]),
        [b'0', _, ..] => (8, &text[1..]),
        _ => (10, text),
    };
    let clean = digits(body, |byte| (byte as char).is_digit(radix))?;
    if let Ok(value) = i64::from_str_radix(&clean, radix) {
        return Some(Value::Int(BigInt::from(value)));
    }
    Some(Value::Float(match radix {
        10 => clean.parse::<f64>().ok()?,
        // `zend_hex_strtod` accumulates `value * 16 + digit` in doubles.
        16 => clean.bytes().fold(0.0, |value, digit| {
            value * 16.0 + f64::from((digit as char).to_digit(16).unwrap_or(0))
        }),
        // `zend_oct_strtod` and `zend_bin_strtod` accumulate `value * base + c - '0'`, the
        // character code included, and round as that does.
        _ => clean.bytes().fold(0.0, |value, digit| {
            value * f64::from(radix) + f64::from(digit) - f64::from(b'0')
        }),
    }))
}

/// A float literal (`1.`, `.5`, `1e5`), with `_` separators only between two digits.
fn float(text: &str) -> Option<Value> {
    let bytes = text.as_bytes();
    let mut clean = String::with_capacity(bytes.len());
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == b'_' {
            let before = index.checked_sub(1).map(|i| bytes[i]);
            if !before.is_some_and(|b| b.is_ascii_digit())
                || !bytes.get(index + 1).is_some_and(u8::is_ascii_digit)
            {
                return None;
            }
        } else {
            clean.push(char::from(byte));
        }
    }
    clean.parse::<f64>().ok().map(Value::Float)
}

/// The body of a quoted literal with an optional `b` prefix, between its quotes.
fn quoted(text: &str, quote: char) -> Option<&str> {
    let text = text
        .strip_prefix('b')
        .or_else(|| text.strip_prefix('B'))
        .unwrap_or(text);
    text.strip_prefix(quote)?.strip_suffix(quote)
}

/// A single-quoted string: `\\` and `\'` are escapes, any other backslash is itself.
fn single_quoted(text: &str) -> Option<Vec<u8>> {
    let body = quoted(text, '\'')?.as_bytes();
    let mut out = Vec::with_capacity(body.len());
    let mut index = 0;
    while index < body.len() {
        if body[index] == b'\\' && matches!(body.get(index + 1), Some(b'\\' | b'\'')) {
            out.push(body[index + 1]);
            index += 2;
        } else {
            out.push(body[index]);
            index += 1;
        }
    }
    Some(out)
}

/// Whether a double-quoted string or heredoc body holds only literal text and escapes, not an
/// interpolation, which a constant expression refuses.
fn literal_parts<D: Doc>(node: &Node<'_, D>) -> bool {
    named(node).all(|child| matches!(child.kind().as_ref(), "string_content" | "escape_sequence"))
}

fn double_quoted(text: &str) -> Option<Vec<u8>> {
    unescape(quoted(text, '"')?.as_bytes(), true)
}

/// PHP's double-quoted escapes, as its lexer reads them: `\n \t \r \v \e \f \\ \$`, `\"` in a
/// double-quoted string (not in a heredoc), one to three octal digits (wrapping past `\377`), `\x`
/// with one or two hex digits, and `\u{...}` as UTF-8 (past U+10FFFF, or empty, it does not
/// compile). Any other backslash is itself.
pub(crate) fn unescape(body: &[u8], double_quote: bool) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(body.len());
    let mut index = 0;
    while index < body.len() {
        let byte = body[index];
        if byte != b'\\' || index + 1 == body.len() {
            out.push(byte);
            index += 1;
            continue;
        }
        let next = body[index + 1];
        index += 2;
        match next {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'v' => out.push(0x0b),
            b'e' => out.push(0x1b),
            b'f' => out.push(0x0c),
            b'\\' => out.push(b'\\'),
            b'$' => out.push(b'$'),
            b'"' if double_quote => out.push(b'"'),
            b'0'..=b'7' => {
                let mut value = u32::from(next - b'0');
                let mut taken = 0;
                while taken < 2 && matches!(body.get(index), Some(b'0'..=b'7')) {
                    value = value * 8 + u32::from(body[index] - b'0');
                    index += 1;
                    taken += 1;
                }
                out.push((value & 0xff) as u8);
            }
            b'x' if body.get(index).is_some_and(u8::is_ascii_hexdigit) => {
                let mut value = (body[index] as char).to_digit(16)?;
                index += 1;
                if let Some(digit) = body
                    .get(index)
                    .and_then(|byte| (*byte as char).to_digit(16))
                {
                    value = value * 16 + digit;
                    index += 1;
                }
                out.push(value as u8);
            }
            b'u' if body.get(index) == Some(&b'{') => {
                let close = body[index..].iter().position(|byte| *byte == b'}')? + index;
                let hex = std::str::from_utf8(&body[index + 1..close]).ok()?;
                if hex.is_empty() || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return None;
                }
                let code = u32::from_str_radix(hex, 16).ok()?;
                if code > 0x10ffff {
                    return None;
                }
                push_utf8(&mut out, code);
                index = close + 1;
            }
            _ => {
                out.push(b'\\');
                out.push(next);
            }
        }
    }
    Some(out)
}

/// A code point as UTF-8, surrogates included, as PHP encodes `\u{D800}`.
fn push_utf8(out: &mut Vec<u8>, code: u32) {
    match code {
        0..=0x7f => out.push(code as u8),
        0x80..=0x7ff => out.extend([0xc0 | (code >> 6) as u8, 0x80 | (code & 0x3f) as u8]),
        0x800..=0xffff => out.extend([
            0xe0 | (code >> 12) as u8,
            0x80 | ((code >> 6) & 0x3f) as u8,
            0x80 | (code & 0x3f) as u8,
        ]),
        _ => out.extend([
            0xf0 | (code >> 18) as u8,
            0x80 | ((code >> 12) & 0x3f) as u8,
            0x80 | ((code >> 6) & 0x3f) as u8,
            0x80 | (code & 0x3f) as u8,
        ]),
    }
}

/// A heredoc or nowdoc: the lines between its opening line and its closing marker, each without
/// the closing marker's indentation (a body line indented less does not compile, nor does
/// indentation mixing tabs and spaces), joined by their own line breaks; a heredoc's escapes are
/// read as a double-quoted string's, but `\"` stays.
fn heredoc<D: Doc>(node: &Node<'_, D>) -> Option<Value> {
    let nowdoc = node.kind() == "nowdoc";
    if let Some(body) = node.field("value")
        && !nowdoc
        && !literal_parts(&body)
    {
        return Some(Value::Unknown);
    }
    let text = node.text();
    let start = node.range().start;
    let open = node.field("identifier")?.range().end - start;
    let close = node.field("end_tag")?.range().start - start;
    let source = text.as_bytes();
    // The opening line ends at the line break after the identifier (and its quotes).
    let line_break = open
        + source
            .get(open..)?
            .iter()
            .position(|b| matches!(b, b'\n' | b'\r'))?;
    let body_start = match &source[line_break..] {
        [b'\r', b'\n', ..] => line_break + 2,
        _ => line_break + 1,
    };
    let line_start = source[..close]
        .iter()
        .rposition(|byte| matches!(byte, b'\n' | b'\r'))
        .map_or(0, |i| i + 1);
    let indent = &source[line_start..close];
    if !indent.iter().all(|byte| matches!(byte, b' ' | b'\t'))
        || (indent.contains(&b' ') && indent.contains(&b'\t'))
    {
        return None;
    }
    let mut out = Vec::new();
    if line_start > body_start {
        // The body ends before the line break that precedes the closing line.
        let body_end = if &source[line_start - 2.min(line_start)..line_start] == b"\r\n" {
            line_start - 2
        } else {
            line_start - 1
        };
        let mut rest = source.get(body_start..body_end.max(body_start))?;
        loop {
            let end = rest
                .iter()
                .position(|byte| matches!(byte, b'\n' | b'\r'))
                .unwrap_or(rest.len());
            let (line, after) = rest.split_at(end);
            let brk = match after {
                [b'\r', b'\n', ..] => 2,
                [b'\n' | b'\r', ..] => 1,
                _ => 0,
            };
            let removable = line
                .iter()
                .take(indent.len())
                .take_while(|byte| matches!(byte, b' ' | b'\t'))
                .count();
            // A line indented less than the closing marker is an error unless it is only white
            // space, which is dropped.
            if (removable < indent.len() && removable < line.len())
                || line[..removable].iter().any(|byte| !indent.contains(byte))
            {
                return None;
            }
            out.extend_from_slice(&line[removable..]);
            out.extend_from_slice(&after[..brk]);
            if brk == 0 {
                break;
            }
            rest = &after[brk..];
        }
    }
    if nowdoc {
        Some(Value::PhpStr(out))
    } else {
        unescape(&out, false).map(Value::PhpStr)
    }
}

// ---------------------------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------------------------

const WHITESPACE: &[u8] = b" \t\n\r\x0b\x0c";

/// What PHP's `_is_numeric_string_ex` reads in a string: its number, and whether text other than
/// white space follows it (a leading-numeric string such as `"5 apples"`); `None` when it does
/// not start with a number. An integer past 64 bits is a float.
fn numeric_prefix(text: &[u8]) -> Option<(Num, bool)> {
    let start = text
        .iter()
        .position(|byte| !WHITESPACE.contains(byte))
        .unwrap_or(text.len());
    let mut index = start;
    if matches!(text.get(index), Some(b'+' | b'-')) {
        index += 1;
    }
    let digits_start = index;
    while text.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    let int_digits = index - digits_start;
    let mut is_float = false;
    if text.get(index) == Some(&b'.') {
        let fraction_start = index + 1;
        let mut end = fraction_start;
        while text.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if int_digits > 0 || end > fraction_start {
            is_float = true;
            index = end;
        }
    }
    if int_digits == 0 && !is_float {
        return None;
    }
    if matches!(text.get(index), Some(b'e' | b'E')) {
        let mut end = index + 1;
        if matches!(text.get(end), Some(b'+' | b'-')) {
            end += 1;
        }
        if text.get(end).is_some_and(u8::is_ascii_digit) {
            while text.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
            is_float = true;
            index = end;
        }
    }
    let number = std::str::from_utf8(&text[start..index]).ok()?;
    let mut rest = index;
    while text.get(rest).is_some_and(|byte| WHITESPACE.contains(byte)) {
        rest += 1;
    }
    let trailing = rest < text.len();
    let num = if is_float {
        Num::Float(number.parse().ok()?)
    } else {
        match number.parse::<i64>() {
            Ok(value) => Num::Int(value),
            Err(_) => Num::Float(number.parse().ok()?),
        }
    };
    Some((num, trailing))
}

/// Whether a numeric string is an integer past 64 bits, which PHP reads as a float, and its sign:
/// PHP's `oflow`.
fn overflow(text: &[u8]) -> i8 {
    let trimmed = trim(text);
    let negative = trimmed.first() == Some(&b'-');
    let digits = trimmed
        .strip_prefix(b"-")
        .or_else(|| trimmed.strip_prefix(b"+"))
        .unwrap_or(trimmed);
    if !digits.is_empty()
        && digits.iter().all(u8::is_ascii_digit)
        && matches!(numeric(text), Some(Num::Float(_)))
    {
        if negative { -1 } else { 1 }
    } else {
        0
    }
}

fn trim(text: &[u8]) -> &[u8] {
    let start = text
        .iter()
        .position(|byte| !WHITESPACE.contains(byte))
        .unwrap_or(text.len());
    let end = text
        .iter()
        .rposition(|byte| !WHITESPACE.contains(byte))
        .map_or(start, |end| end + 1);
    &text[start..end.max(start)]
}

/// A whole numeric string (white space around allowed): what `==` and `<=>` read as a number.
fn numeric(text: &[u8]) -> Option<Num> {
    match numeric_prefix(text)? {
        (num, false) => Some(num),
        (_, true) => None,
    }
}

/// An operand of arithmetic, as `zendi_try_convert_scalar_to_number` reads it: `null` is `0`, a
/// bool `0` or `1`, a string its number (a leading-numeric one with a warning); `None` where PHP
/// throws a `TypeError` (a non-numeric string, an array).
fn to_number(value: &Value) -> Option<Num> {
    match value {
        Value::None => Some(Num::Int(0)),
        Value::Bool(value) => Some(Num::Int(i64::from(*value))),
        Value::Int(value) => value.to_i64().map(Num::Int),
        Value::Float(value) => Some(Num::Float(*value)),
        Value::PhpStr(text) => numeric_prefix(text).map(|(num, _)| num),
        _ => None,
    }
}

/// 2⁶³, the first float past `PHP_INT_MAX`.
const TWO_63: f64 = 9_223_372_036_854_775_808.0;

/// A float as an integer, as `zend_dval_to_lval` converts it: modulo 2⁶⁴ past 64 bits, and `0`
/// for infinities and NaN.
fn float_to_int(value: f64) -> i64 {
    if !value.is_finite() {
        return 0;
    }
    if (-TWO_63..TWO_63).contains(&value) {
        return value as i64;
    }
    let two_64 = 2.0 * TWO_63;
    let mut modulo = value.trunc() % two_64;
    if modulo < 0.0 {
        modulo += two_64;
    }
    if modulo >= TWO_63 {
        modulo -= two_64;
    }
    modulo as i64
}

/// A float as an integer, saturating past 64 bits: `zend_dval_to_lval_cap`, which PHP applies to
/// the float in a numeric string.
fn float_to_int_cap(value: f64) -> i64 {
    if value.is_nan() {
        0
    } else if value >= TWO_63 {
        i64::MAX
    } else if value < -TWO_63 {
        i64::MIN
    } else {
        value as i64
    }
}

/// An operand of `%`, bitwise operators and shifts, as `zendi_try_get_long` reads it.
fn to_int_operand(value: &Value) -> Option<i64> {
    match value {
        Value::None => Some(0),
        Value::Bool(value) => Some(i64::from(*value)),
        Value::Int(value) => value.to_i64(),
        Value::Float(value) => {
            if !value.is_finite() {
                return None;
            }
            Some(float_to_int(*value))
        }
        Value::PhpStr(text) => match numeric_prefix(text)? {
            (Num::Int(value), _) => Some(value),
            (Num::Float(value), _) => {
                if !value.is_finite() {
                    return None;
                }
                Some(float_to_int_cap(value))
            }
        },
        _ => None,
    }
}

/// `(int)`: a string reads its leading number (none is `0`); an array is `0` when empty, else `1`.
fn to_int_cast(value: &Value) -> Option<i64> {
    match value {
        Value::None => Some(0),
        Value::Bool(value) => Some(i64::from(*value)),
        Value::Int(value) => value.to_i64(),
        Value::Float(value) => Some(float_to_int(*value)),
        Value::PhpStr(text) => Some(match numeric_prefix(text) {
            Some((Num::Int(value), _)) => value,
            Some((Num::Float(value), _)) => float_to_int_cap(value),
            None => 0,
        }),
        Value::PhpArray(items) => Some(i64::from(!items.is_empty())),
        _ => None,
    }
}

/// `(float)`: a string reads its leading number with `strtod` (none is `0`).
fn to_float_cast(value: &Value) -> Option<f64> {
    match value {
        Value::None => Some(0.0),
        Value::Bool(value) => Some(f64::from(u8::from(*value))),
        Value::Int(value) => value.to_i64().map(|value| value as f64),
        Value::Float(value) => Some(*value),
        Value::PhpStr(text) => Some(numeric_prefix(text).map_or(0.0, |(num, _)| num.float())),
        Value::PhpArray(items) => Some(f64::from(u8::from(!items.is_empty()))),
        _ => None,
    }
}

/// A value as a string, as PHP converts it: `null` and `false` are empty, `true` is `1`, floats
/// as `precision=14` prints them, an array is `Array` (with a warning).
pub(crate) fn to_bytes(value: &Value) -> Option<Vec<u8>> {
    Some(match value {
        Value::None | Value::Bool(false) => Vec::new(),
        Value::Bool(true) => b"1".to_vec(),
        Value::Int(value) => value.to_string().into_bytes(),
        Value::Float(value) => float_string(*value).into_bytes(),
        Value::PhpStr(text) => text.clone(),
        Value::PhpArray(_) => b"Array".to_vec(),
        _ => return None,
    })
}

/// A value as text, where its string is UTF-8.
pub(crate) fn to_string(value: &Value) -> Option<String> {
    String::from_utf8(to_bytes(value)?).ok()
}

/// A float as PHP prints it with `precision=14`: `zend_gcvt`, fourteen significant digits without
/// trailing zeros, in `1.0E+25` form when the exponent is below -4 or at least 15; `INF`, `-INF`,
/// `NAN`, and `-0` for negative zero.
fn float_string(value: f64) -> String {
    if value.is_nan() {
        return "NAN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "INF" } else { "-INF" }.into();
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if value == 0.0 {
        return format!("{sign}0");
    }
    let formatted = format!("{:.13e}", value.abs());
    let (mantissa, exponent) = formatted.split_once('e').unwrap_or((&formatted, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let decpt = exponent + 1;
    let mut out = String::from(sign);
    if if decpt < 0 { decpt < -3 } else { decpt > 14 } {
        let (first, rest) = digits.split_at(1);
        out.push_str(first);
        out.push('.');
        out.push_str(if rest.is_empty() { "0" } else { rest });
        let exp = decpt - 1;
        out.push('E');
        out.push(if exp < 0 { '-' } else { '+' });
        out.push_str(&exp.abs().to_string());
    } else if decpt <= 0 {
        out.push_str("0.");
        for _ in decpt..0 {
            out.push('0');
        }
        out.push_str(digits);
    } else {
        let decpt = decpt as usize;
        if digits.len() <= decpt {
            out.push_str(digits);
            for _ in digits.len()..decpt {
                out.push('0');
            }
        } else {
            out.push_str(&digits[..decpt]);
            out.push('.');
            out.push_str(&digits[decpt..]);
        }
    }
    out
}

/// PHP's `(bool)`.
pub(crate) fn truthy(value: &Value) -> Option<bool> {
    Some(match value {
        Value::None => false,
        Value::Bool(value) => *value,
        Value::Int(value) => value.to_i64()? != 0,
        // NaN is truthy in PHP.
        Value::Float(value) => *value != 0.0,
        Value::PhpStr(text) => !(text.is_empty() || text.as_slice() == b"0"),
        Value::PhpArray(items) => !items.is_empty(),
        Value::Container {
            kind: ContainerKind::PhpArray,
            nonempty,
            ..
        } => return *nonempty,
        _ => return None,
    })
}

/// PHP's `get_debug_type` for the values a constant expression has.
pub(crate) fn type_name(value: &Value) -> Option<&'static str> {
    Some(match value {
        Value::None => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::PhpStr(_) => "string",
        Value::PhpArray(_)
        | Value::Container {
            kind: ContainerKind::PhpArray,
            ..
        } => "array",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------------------------

/// `===`: the same type and value; floats by value (`0.0 === -0.0`, NaN is not NaN); arrays with
/// the same keys in the same order and identical values.
pub(crate) fn identical(left: &Value, right: &Value) -> Option<bool> {
    Some(match (left, right) {
        (Value::None, Value::None) => true,
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Int(left), Value::Int(right)) => left == right,
        (Value::Float(left), Value::Float(right)) => left == right,
        (Value::PhpStr(left), Value::PhpStr(right)) => left == right,
        (Value::PhpArray(left), Value::PhpArray(right)) => {
            if left.len() != right.len() {
                return Some(false);
            }
            let mut all = Some(true);
            for ((left_key, left), (right_key, right)) in left.iter().zip(right) {
                if left_key != right_key {
                    return Some(false);
                }
                match identical(left, right) {
                    Some(true) => {}
                    Some(false) => return Some(false),
                    None => all = None,
                }
            }
            return all;
        }
        (Value::Unknown, _) | (_, Value::Unknown) => return None,
        // An array with an unknown key or element is identical to another array only as far as
        // their lengths allow, and to nothing else.
        (Value::Container { .. }, _) | (_, Value::Container { .. }) => {
            return match (array_len(left), array_len(right)) {
                (Some(Some(first)), Some(Some(second))) if first != second => Some(false),
                (Some(_), Some(_)) => None,
                _ => Some(false),
            };
        }
        _ => false,
    })
}

/// An array's length: `Some(None)` for an array whose length is not known, `None` for anything
/// that is not an array.
fn array_len(value: &Value) -> Option<Option<usize>> {
    match value {
        Value::PhpArray(items) => Some(Some(items.len())),
        Value::Container {
            kind: ContainerKind::PhpArray,
            len,
            ..
        } => Some(*len),
        _ => None,
    }
}

/// `<=>` with PHP's order: `Some(None)` where each side is greater than the other (NaN, arrays with
/// keys the other lacks), which no comparison operator but `<=>` and `!=` holds for.
pub(crate) fn order(left: &Value, right: &Value) -> Option<Option<Ordering>> {
    let forward = compare(left, right)?;
    let backward = compare(right, left)?;
    Some(
        if forward == Ordering::Greater && backward == Ordering::Greater {
            None
        } else {
            Some(forward)
        },
    )
}

fn sign(value: f64) -> Ordering {
    if value == 0.0 {
        Ordering::Equal
    } else if value < 0.0 {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

/// Doubles as `ZEND_THREEWAY_COMPARE` orders them: NaN is greater than everything, both ways.
fn compare_floats(left: f64, right: f64) -> Ordering {
    if left == right {
        Ordering::Equal
    } else if left < right {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

fn compare_nums(left: Num, right: Num) -> Ordering {
    match (left, right) {
        (Num::Int(left), Num::Int(right)) => left.cmp(&right),
        (left, right) => compare_floats(left.float(), right.float()),
    }
}

/// Bytes as `zend_binary_strcmp` orders them.
fn compare_bytes(left: &[u8], right: &[u8]) -> Ordering {
    left.cmp(right)
}

/// Two strings as `zendi_smart_strcmp` orders them: numerically when both are numeric strings
/// (with PHP's handling of integers past 64 bits), else byte by byte. Two integers past 64 bits
/// on the same side whose doubles coincide compare as bytes, as do two equal infinite doubles.
fn smart_strcmp(left: &[u8], right: &[u8]) -> Ordering {
    if let (Some(first), Some(second)) = (numeric(left), numeric(right)) {
        let (oflow1, oflow2) = (overflow(left), overflow(right));
        match (first, second) {
            (Num::Float(first), Num::Float(second))
                if oflow1 != 0 && oflow1 == oflow2 && first - second == 0.0 => {}
            (Num::Int(first), Num::Int(second)) => return first.cmp(&second),
            (Num::Int(first), Num::Float(second)) => {
                if oflow2 != 0 {
                    return if oflow2 > 0 {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    };
                }
                return sign(first as f64 - second);
            }
            (Num::Float(first), Num::Int(second)) => {
                if oflow1 != 0 {
                    return if oflow1 > 0 {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    };
                }
                return sign(first - second as f64);
            }
            (Num::Float(first), Num::Float(second)) => {
                if first != second || first.is_finite() {
                    return sign(first - second);
                }
            }
        }
    }
    compare_bytes(left, right)
}

/// An integer against a string: numerically when the string is numeric, else the integer's
/// decimal text against the string.
fn compare_int_string(int: i64, text: &[u8]) -> Ordering {
    match numeric(text) {
        Some(num) => compare_nums(Num::Int(int), num),
        None => compare_bytes(int.to_string().as_bytes(), text),
    }
}

fn compare_float_string(float: f64, text: &[u8]) -> Ordering {
    if float.is_nan() {
        return Ordering::Greater;
    }
    match numeric(text) {
        Some(num) => compare_floats(float, num.float()),
        None => compare_bytes(float_string(float).as_bytes(), text),
    }
}

/// PHP's relational operators: `a < b` and `a <= b` compare `a` with `b`, `a > b` and `a >= b`
/// compare `b` with `a`, so over arrays both `a < b` and `a > b` may hold.
pub(crate) fn relation(operator: &str, left: &Value, right: &Value) -> Option<bool> {
    Some(match operator {
        "<" => compare(left, right)? == Ordering::Less,
        "<=" => compare(left, right)? != Ordering::Greater,
        ">" => compare(right, left)? == Ordering::Less,
        ">=" => compare(right, left)? != Ordering::Greater,
        _ => return None,
    })
}

/// PHP 8's `zend_compare`: the order `<=>`, `==` and the relational operators read. It is not
/// antisymmetric: arrays compare by count, then by each of the left side's keys in the right,
/// and a key the right lacks makes the left greater.
fn compare(left: &Value, right: &Value) -> Option<Ordering> {
    use Value::*;
    let bool_of = |value: &Value| truthy(value);
    Some(match (left, right) {
        (Int(_) | Float(_), Int(_) | Float(_)) => compare_nums(to_number(left)?, to_number(right)?),
        (PhpArray(left), PhpArray(right)) => {
            if left.len() != right.len() {
                return Some(left.len().cmp(&right.len()));
            }
            for (key, value) in left {
                let Some((_, other)) = right.iter().find(|(other, _)| other == key) else {
                    return Some(Ordering::Greater);
                };
                match compare(value, other)? {
                    Ordering::Equal => {}
                    order => return Some(order),
                }
            }
            Ordering::Equal
        }
        (None, None) => Ordering::Equal,
        (None, Bool(value)) => (false).cmp(value),
        (Bool(value), None) => value.cmp(&false),
        (Bool(left), Bool(right)) => left.cmp(right),
        (PhpStr(left), PhpStr(right)) => smart_strcmp(left, right),
        (None, PhpStr(text)) => {
            if text.is_empty() {
                Ordering::Equal
            } else {
                Ordering::Less
            }
        }
        (PhpStr(text), None) => {
            if text.is_empty() {
                Ordering::Equal
            } else {
                Ordering::Greater
            }
        }
        (Int(int), PhpStr(text)) => compare_int_string(int.to_i64()?, text),
        (PhpStr(text), Int(int)) => compare_int_string(int.to_i64()?, text).reverse(),
        (Float(float), PhpStr(text)) => compare_float_string(*float, text),
        (PhpStr(text), Float(float)) => {
            if float.is_nan() {
                Ordering::Greater
            } else {
                compare_float_string(*float, text).reverse()
            }
        }
        (None | Bool(false), _) => {
            if bool_of(right)? {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        }
        (Bool(true), _) => {
            if bool_of(right)? {
                Ordering::Equal
            } else {
                Ordering::Greater
            }
        }
        (_, None | Bool(false)) => {
            if bool_of(left)? {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }
        (_, Bool(true)) => {
            if bool_of(left)? {
                Ordering::Equal
            } else {
                Ordering::Less
            }
        }
        (PhpArray(_), Int(_) | Float(_) | PhpStr(_)) => Ordering::Greater,
        (Int(_) | Float(_) | PhpStr(_), PhpArray(_)) => Ordering::Less,
        _ => return Option::None,
    })
}

// ---------------------------------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------------------------------

fn int_value(value: i64) -> Value {
    Value::Int(BigInt::from(value))
}

/// A prefix operator or cast applied to an operand: what `-x`, `(int) x` and the like read.
enum Prefix {
    Operator(String),
    Cast(String),
}

impl Prefix {
    /// The prefix of a unary operation or cast node, and its operand.
    fn of<'a, D: Doc>(node: &Node<'a, D>) -> Option<(Self, Node<'a, D>)> {
        match node.kind().as_ref() {
            "unary_op_expression" => Some((
                Self::Operator(node.field("operator")?.text().to_string()),
                node.field("argument").or_else(|| named(node).last())?,
            )),
            "cast_expression" => Some((
                Self::Cast(node.field("type")?.text().to_ascii_lowercase()),
                node.field("value")?,
            )),
            _ => None,
        }
    }

    fn apply(&self, value: Value) -> Value {
        match self {
            Self::Operator(operator) => apply_unary(operator, value),
            Self::Cast(kind) => apply_cast(kind, value),
        }
    }
}

fn unary<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Option<Value> {
    let (prefix, operand) = Prefix::of(node)?;
    Some(prefix.apply(decode(&operand, context)))
}

fn apply_unary(operator: &str, value: Value) -> Value {
    if operator == "!" {
        return truthy(&value).map_or(Value::Unknown, |value| Value::Bool(!value));
    }
    let Some(value) = known(value) else {
        return Value::Unknown;
    };
    match operator {
        // PHP compiles `-x` to `x * -1` and `+x` to `x * 1`.
        "-" => arithmetic("*", &value, &int_value(-1)).unwrap_or(Value::Unknown),
        "+" => arithmetic("*", &value, &int_value(1)).unwrap_or(Value::Unknown),
        "~" => match value {
            Value::Int(value) => value
                .to_i64()
                .map_or(Value::Unknown, |value| int_value(!value)),
            Value::Float(value) if value.is_finite() => int_value(!float_to_int(value)),
            Value::PhpStr(text) => Value::PhpStr(text.iter().map(|byte| !byte).collect()),
            _ => Value::Unknown,
        },
        _ => Value::Unknown,
    }
}

/// `a ** b` where `a` is written with prefix operators or casts: PHP binds `**` tighter than
/// them (`-2 ** 2` is `-(2 ** 2)`), where tree-sitter-php 0.24.2 reads `(-2) ** 2`.
fn power<D: Doc>(
    left: &Node<'_, D>,
    right: &Node<'_, D>,
    context: &mut Context<'_>,
) -> Option<Value> {
    let mut prefixes = Vec::new();
    let mut base = left.clone();
    while let Some((prefix, operand)) = Prefix::of(&base) {
        prefixes.push(prefix);
        base = operand;
    }
    let (Some(base), Some(exponent)) =
        (known(decode(&base, context)), known(decode(right, context)))
    else {
        return Some(Value::Unknown);
    };
    let mut value = arithmetic("**", &base, &exponent).unwrap_or(Value::Unknown);
    for prefix in prefixes.iter().rev() {
        value = prefix.apply(value);
    }
    Some(value)
}

/// `+ - * / % **` over PHP numbers (and `+` over two arrays, their union); `None` where PHP
/// throws.
fn arithmetic(operator: &str, left: &Value, right: &Value) -> Option<Value> {
    if operator == "+"
        && let (Value::PhpArray(left), Value::PhpArray(right)) = (left, right)
    {
        let mut union = left.clone();
        for (key, value) in right {
            if !union.iter().any(|(own, _)| own == key) {
                union.push((key.clone(), value.clone()));
            }
        }
        return Some(Value::PhpArray(union));
    }
    if operator == "%" {
        let divisor = to_int_operand(right)?;
        let dividend = to_int_operand(left)?;
        return match divisor {
            0 => None,
            -1 => Some(int_value(0)),
            divisor => Some(int_value(dividend % divisor)),
        };
    }
    let (left, right) = (to_number(left)?, to_number(right)?);
    Some(match operator {
        "+" | "-" | "*" => match (left, right) {
            (Num::Int(a), Num::Int(b)) => {
                let exact = match operator {
                    "+" => a.checked_add(b),
                    "-" => a.checked_sub(b),
                    _ => a.checked_mul(b),
                };
                match exact {
                    Some(value) => int_value(value),
                    None => Value::Float(match operator {
                        "+" => a as f64 + b as f64,
                        "-" => a as f64 - b as f64,
                        _ => a as f64 * b as f64,
                    }),
                }
            }
            (a, b) => Value::Float(match operator {
                "+" => a.float() + b.float(),
                "-" => a.float() - b.float(),
                _ => a.float() * b.float(),
            }),
        },
        "/" => {
            if right.float() == 0.0 {
                return None;
            }
            match (left, right) {
                (Num::Int(a), Num::Int(b)) if b != -1 || a != i64::MIN => {
                    if a % b == 0 {
                        int_value(a / b)
                    } else {
                        Value::Float(a as f64 / b as f64)
                    }
                }
                (a, b) => Value::Float(a.float() / b.float()),
            }
        }
        "**" => match (left, right) {
            (Num::Int(base), Num::Int(exponent)) if exponent >= 0 => int_pow(base, exponent),
            (base, exponent) => Value::Float(base.float().powf(exponent.float())),
        },
        _ => return None,
    })
}

/// `pow_function_base` over two integers, the exponent not negative: exponentiation by squaring
/// over 64-bit integers, which at the first product past 64 bits takes the product as a double
/// and finishes with `pow` (`1 ** 4294967296` is `1`, `2 ** 63` is a float).
fn int_pow(base: i64, exponent: i64) -> Value {
    if exponent == 0 {
        return int_value(1);
    }
    if base == 0 {
        return int_value(0);
    }
    let (mut result, mut square, mut remaining) = (1_i64, base, exponent);
    while remaining >= 1 {
        if remaining % 2 == 1 {
            remaining -= 1;
            match result.checked_mul(square) {
                Some(product) => result = product,
                None => {
                    let product = result as f64 * square as f64;
                    return Value::Float(product * (square as f64).powf(remaining as f64));
                }
            }
        } else {
            remaining /= 2;
            match square.checked_mul(square) {
                Some(product) => square = product,
                None => {
                    let product = square as f64 * square as f64;
                    return Value::Float(result as f64 * product.powf(remaining as f64));
                }
            }
        }
    }
    int_value(result)
}

/// `& | ^` (bytewise over two strings), `<< >>`.
fn bitwise(operator: &str, left: &Value, right: &Value) -> Option<Value> {
    if let (Value::PhpStr(left), Value::PhpStr(right)) = (left, right)
        && matches!(operator, "&" | "|" | "^")
    {
        let (short, long) = if left.len() <= right.len() {
            (left, right)
        } else {
            (right, left)
        };
        return Some(Value::PhpStr(match operator {
            "|" => long
                .iter()
                .enumerate()
                .map(|(index, byte)| byte | short.get(index).copied().unwrap_or(0))
                .collect(),
            "&" => short.iter().zip(long).map(|(a, b)| a & b).collect(),
            _ => short.iter().zip(long).map(|(a, b)| a ^ b).collect(),
        }));
    }
    let (left, right) = (to_int_operand(left)?, to_int_operand(right)?);
    Some(int_value(match operator {
        "&" => left & right,
        "|" => left | right,
        "^" => left ^ right,
        "<<" => {
            if right < 0 {
                return None;
            }
            if right >= 64 { 0 } else { left << right }
        }
        ">>" => {
            if right < 0 {
                return None;
            }
            if right >= 64 {
                if left < 0 { -1 } else { 0 }
            } else {
                left >> right
            }
        }
        _ => return None,
    }))
}

fn binary<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Option<Value> {
    let operator = node.field("operator")?.text().to_ascii_lowercase();
    let left_node = node.field("left")?;
    let right_node = node.field("right")?;
    if operator == "**" {
        return power(&left_node, &right_node, context);
    }
    let left = decode(&left_node, context);
    // Short-circuiting operators decide on the left where they can.
    match operator.as_str() {
        "&&" | "and" | "||" | "or" => {
            let and = matches!(operator.as_str(), "&&" | "and");
            let Some(left) = truthy(&left) else {
                return Some(Value::Unknown);
            };
            if left != and {
                return Some(Value::Bool(left));
            }
            let right = decode(&right_node, context);
            return Some(truthy(&right).map_or(Value::Unknown, Value::Bool));
        }
        "??" => {
            return Some(match left {
                Value::None => decode(&right_node, context),
                Value::Unknown => Value::Unknown,
                value => value,
            });
        }
        _ => {}
    }
    let right = decode(&right_node, context);
    if operator == "xor" {
        return Some(match (truthy(&left), truthy(&right)) {
            (Some(left), Some(right)) => Value::Bool(left != right),
            _ => Value::Unknown,
        });
    }
    if matches!(operator.as_str(), "===" | "!==") {
        return Some(identical(&left, &right).map_or(Value::Unknown, |same| {
            Value::Bool(same == (operator == "==="))
        }));
    }
    let (Some(left), Some(right)) = (known(left), known(right)) else {
        return Some(Value::Unknown);
    };
    let result = match operator.as_str() {
        "+" | "-" | "*" | "/" | "%" | "**" => arithmetic(&operator, &left, &right),
        "&" | "|" | "^" | "<<" | ">>" => bitwise(&operator, &left, &right),
        "." => {
            // `.` directly followed by a digit is a float to PHP's lexer (`0x1.5` does not
            // compile), not the concatenation tree-sitter-php reads.
            if node.field("operator")?.range().end == right_node.range().start
                && right_node.text().starts_with(|c: char| c.is_ascii_digit())
            {
                return Some(Value::Unknown);
            }
            let mut text = to_bytes(&left)?;
            text.extend(to_bytes(&right)?);
            (text.len() <= super::MAX_STRING_BYTES).then_some(Value::PhpStr(text))
        }
        "==" => compare(&left, &right).map(|order| Value::Bool(order == Ordering::Equal)),
        "!=" | "<>" => compare(&left, &right).map(|order| Value::Bool(order != Ordering::Equal)),
        "<" | "<=" | ">" | ">=" => relation(&operator, &left, &right).map(Value::Bool),
        "<=>" => compare(&left, &right).map(|order| int_value(order as i64)),
        _ => None,
    };
    Some(result.unwrap_or(Value::Unknown))
}

fn conditional<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Option<Value> {
    let condition = decode(&node.field("condition")?, context);
    let alternative = node.field("alternative")?;
    Some(match truthy(&condition) {
        None => Value::Unknown,
        Some(true) => match node.field("body") {
            Some(body) => decode(&body, context),
            // `a ?: b` is `a` when `a` is truthy.
            None => condition,
        },
        Some(false) => decode(&alternative, context),
    })
}

/// `(int)`, `(float)`, `(string)`, `(bool)`, `(array)` and their deprecated spellings; an
/// `(object)` cast is an object, which Rerail does not represent.
fn apply_cast(kind: &str, value: Value) -> Value {
    if !value.is_constant() {
        return match kind {
            "bool" | "boolean" => truthy(&value).map_or(Value::Unknown, Value::Bool),
            _ => Value::Unknown,
        };
    }
    match kind {
        "int" | "integer" => to_int_cast(&value).map_or(Value::Unknown, int_value),
        "float" | "double" => to_float_cast(&value).map_or(Value::Unknown, Value::Float),
        "string" | "binary" => to_bytes(&value).map_or(Value::Unknown, Value::PhpStr),
        "bool" | "boolean" => truthy(&value).map_or(Value::Unknown, Value::Bool),
        "array" => match value {
            Value::None => Value::PhpArray(Vec::new()),
            Value::PhpArray(items) => Value::PhpArray(items),
            value => Value::PhpArray(vec![(Key::Int(0), value)]),
        },
        _ => Value::Unknown,
    }
}

// ---------------------------------------------------------------------------------------------
// Arrays
// ---------------------------------------------------------------------------------------------

/// A value as an array key, as PHP coerces it: a canonical decimal integer string is that
/// integer, a float is truncated, a bool is `0` or `1`, `null` is `""`; `None` for an array.
pub(crate) fn key_of(value: &Value) -> Option<Key> {
    Some(match value {
        Value::None => Key::Str(Vec::new()),
        Value::Bool(value) => Key::Int(i64::from(*value)),
        Value::Int(value) => Key::Int(value.to_i64()?),
        Value::Float(value) => Key::Int(float_to_int(*value)),
        Value::PhpStr(text) => canonical_int(text).map_or_else(|| Key::Str(text.clone()), Key::Int),
        _ => return None,
    })
}

/// A string that is a canonical decimal integer (`"5"`, `"-5"`, not `"05"`, `"-0"` or `"+5"`)
/// within 64 bits.
pub(crate) fn canonical_int(text: &[u8]) -> Option<i64> {
    let digits = text.strip_prefix(b"-").unwrap_or(text);
    if digits.is_empty()
        || !digits.iter().all(u8::is_ascii_digit)
        || (digits[0] == b'0' && (digits.len() > 1 || text.len() > digits.len()))
    {
        return None;
    }
    std::str::from_utf8(text).ok()?.parse().ok()
}

/// An array literal under construction: its entries, and PHP's next free integer key (`None`
/// until an integer key is used; after a negative key it is that key plus one, as of PHP 8.3).
struct Builder<T> {
    entries: Vec<(Key, T)>,
    next: Option<i64>,
}

impl<T> Builder<T> {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            next: None,
        }
    }

    fn insert(&mut self, key: Key, value: T) {
        if let Key::Int(index) = key
            && self.next.is_none_or(|next| index >= next)
        {
            self.next = Some(index.saturating_add(1));
        }
        match self.entries.iter_mut().find(|(own, _)| *own == key) {
            Some(entry) => entry.1 = value,
            None => self.entries.push((key, value)),
        }
    }

    /// Appends at the next free integer key; `None` where that key is taken (past
    /// `PHP_INT_MAX`), which PHP refuses.
    fn push(&mut self, value: T) -> Option<()> {
        let key = Key::Int(self.next.unwrap_or(0));
        if self.entries.iter().any(|(own, _)| *own == key) {
            return None;
        }
        self.insert(key, value);
        Some(())
    }
}

/// An array literal's entries in order, each key as PHP coerces it (a later duplicate replaces
/// an earlier one's element in place) with its element's node: what `...[...]` unpacks. `None`
/// where a key is not known, or the literal spreads or takes a reference.
pub(crate) fn array_entries<'r, D: Doc>(
    node: &Node<'r, D>,
    context: &mut Context<'_>,
) -> Option<Vec<(Key, Node<'r, D>)>> {
    if node.kind() != "array_creation_expression" {
        return None;
    }
    let mut builder = Builder::new();
    for element in named(node) {
        let parts: Vec<_> = named(&element).collect();
        match parts.as_slice() {
            [value]
                if element.kind() == "array_element_initializer"
                    && !matches!(value.kind().as_ref(), "variadic_unpacking" | "by_ref") =>
            {
                builder.push(value.clone())?;
            }
            [key, value] if element.kind() == "array_element_initializer" => {
                let key = key_of(&known(decode(key, context))?)?;
                builder.insert(key, value.clone());
            }
            _ => return None,
        }
    }
    Some(builder.entries)
}

fn array<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Option<Value> {
    let mut builder = Builder::new();
    // Where a key or a spread is not known, the array's length is not; where only an element is
    // not, its length still is.
    let mut unknown_key = false;
    let mut unknown_value = false;
    let mut has_element = false;
    for element in named(node) {
        if element.kind() != "array_element_initializer" {
            return None;
        }
        let parts: Vec<_> = named(&element).collect();
        match parts.as_slice() {
            [spread] if spread.kind() == "variadic_unpacking" => {
                match known(decode(&only_child(spread)?, context)) {
                    Some(Value::PhpArray(items)) => {
                        for (key, value) in items {
                            match key {
                                Key::Int(_) => builder.push(value)?,
                                key => builder.insert(key, value),
                            }
                        }
                    }
                    Some(_) => return None,
                    None => unknown_key = true,
                }
            }
            [by_ref] if by_ref.kind() == "by_ref" => return None,
            [value] => {
                has_element = true;
                let value = decode(value, context);
                unknown_value |= !value.is_constant();
                builder.push(value)?;
            }
            [key, value] => {
                has_element = true;
                let key = known(decode(key, context));
                let value = decode(value, context);
                unknown_value |= !value.is_constant();
                match key {
                    Some(key) => builder.insert(key_of(&key)?, value),
                    None => unknown_key = true,
                }
            }
            _ => return None,
        }
    }
    Some(if unknown_key {
        Value::Container {
            kind: ContainerKind::PhpArray,
            len: None,
            nonempty: (has_element || !builder.entries.is_empty()).then_some(true),
        }
    } else if unknown_value {
        Value::Container {
            kind: ContainerKind::PhpArray,
            len: Some(builder.entries.len()),
            nonempty: Some(!builder.entries.is_empty()),
        }
    } else {
        Value::PhpArray(builder.entries)
    })
}

/// `a[k]` over a known array (a missing key is `null`, with a warning) or string (one byte; a
/// negative offset counts from the end; past the end is `""`, with a warning).
fn subscript<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Option<Value> {
    let parts: Vec<_> = named(node).collect();
    let [object, index] = parts.as_slice() else {
        return None;
    };
    let text = node.text();
    if !text.trim_end().ends_with(']') {
        return Some(Value::Unknown);
    }
    let (Some(object), Some(index)) = (
        known(decode(object, context)),
        known(decode(index, context)),
    ) else {
        return Some(Value::Unknown);
    };
    Some(match object {
        Value::PhpArray(items) => {
            let key = key_of(&index)?;
            items
                .into_iter()
                .find(|(own, _)| *own == key)
                .map_or(Value::None, |(_, value)| value)
        }
        Value::PhpStr(bytes) => {
            let offset = match index {
                Value::Int(offset) => offset.to_i64()?,
                Value::PhpStr(text) => canonical_int(&text)?,
                _ => return Some(Value::Unknown),
            };
            let position = if offset < 0 {
                i64::try_from(bytes.len()).ok()? + offset
            } else {
                offset
            };
            match usize::try_from(position).ok().and_then(|at| bytes.get(at)) {
                Some(byte) => Value::PhpStr(vec![*byte]),
                None => Value::PhpStr(Vec::new()),
            }
        }
        _ => Value::Unknown,
    })
}

#[cfg(test)]
pub(crate) mod tests;
