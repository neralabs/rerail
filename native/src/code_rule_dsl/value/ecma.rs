//! TypeScript values (tree-sitter-typescript 0.23): literals as ECMAScript reads them, strings as
//! UTF-16 code units, and operators folded, in the sense of eslint-utils' `getStaticValue`.

#[rustfmt::skip]
#[path = "ecma/builtins.generated.rs"]
pub(crate) mod builtins;
mod calls;
mod destructure;
#[path = "ecma/entities.generated.rs"]
mod entities;
pub(crate) mod ops;

use super::{ContainerKind, Context, Lang, MAX_ELEMENTS, MAX_STRING_BYTES, Value};
use ast_grep_core::{Doc, Node};
pub(crate) use destructure::destructured;

/// Decodes `node` to its value; `Unknown` for anything that is not a literal this knows.
pub(crate) fn decode<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    if !context.enter() {
        return Value::Unknown;
    }
    let value = decode_node(node, context);
    context.leave();
    if context.exhausted() {
        return Value::Unknown;
    }
    value
}

fn decode_node<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    match node.kind().as_ref() {
        "number" => number(&node.text()),
        "string" => string_node(node).map_or(Value::Unknown, Value::Utf16),
        "template_string" => template(node, context),
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::None,
        // TypeScript's wrappers leave the value alone, as `getStaticValue` reads them.
        "parenthesized_expression"
        | "as_expression"
        | "satisfies_expression"
        | "non_null_expression"
        | "type_assertion"
        | "instantiation_expression" => match operand(node) {
            Some(inner) => decode(&inner, context),
            None => Value::Unknown,
        },
        "array" => array(node, context),
        "regex" => regexp(node),
        "object" => object(node),
        // Builtin globals whose values are primitives, where no file-local binding shadows them.
        // With `follow`, other names are their bindings' values (`getStaticValue`'s constants).
        "undefined" | "identifier" => {
            let name = node.text();
            let value = match name.as_ref() {
                "undefined" => Value::Undefined,
                "NaN" => Value::Float(f64::NAN),
                "Infinity" => Value::Float(f64::INFINITY),
                _ => return follow(node, context),
            };
            if context.is_global(node.range(), &name) {
                value
            } else {
                follow(node, context)
            }
        }
        "unary_expression" => {
            let (Some(operator), Some(argument)) = (node.field("operator"), node.field("argument"))
            else {
                return Value::Unknown;
            };
            let operator = operator.text();
            // `void x` is `undefined` whatever `x` is; `delete` is not folded.
            if operator == "void" {
                return Value::Undefined;
            }
            ops::unary(&operator, &decode(&argument, context))
        }
        "binary_expression" => binary(node, context),
        "ternary_expression" => {
            let (Some(test), Some(consequence), Some(alternative)) = (
                node.field("condition"),
                node.field("consequence"),
                node.field("alternative"),
            ) else {
                return Value::Unknown;
            };
            match decode(&test, context).truthy(Lang::Js) {
                Some(true) => decode(&consequence, context),
                Some(false) => decode(&alternative, context),
                None => Value::Unknown,
            }
        }
        // The last expression's value.
        "sequence_expression" => match node
            .children()
            .filter(|child| child.is_named() && child.kind() != "comment")
            .last()
        {
            Some(last) => decode(&last, context),
            None => Value::Unknown,
        },
        // `x = v` is `v`.
        "assignment_expression" => match node.field("right") {
            Some(right) => decode(&right, context),
            None => Value::Unknown,
        },
        // Builtins' constants, getters and pure functions (`calls.rs`).
        "member_expression" | "subscript_expression" | "call_expression" => {
            calls::chained(node, context).value
        }
        _ => Value::Unknown,
    }
}

/// Expressions: the subtypes of the grammars' `expression` and `primary_expression` supertypes
/// (tree-sitter-typescript 0.23.2 `node-types.json`, TSX's JSX elements included), sequences, and
/// shorthand properties. Only an expression has a value; on any other node (a statement, a type,
/// a property name, punctuation) every value predicate fails.
pub(crate) const EXPRESSION_KINDS: &[&str] = &[
    "array",
    "arrow_function",
    "as_expression",
    "assignment_expression",
    "augmented_assignment_expression",
    "await_expression",
    "binary_expression",
    "call_expression",
    "class",
    "false",
    "function_expression",
    "generator_function",
    "identifier",
    "instantiation_expression",
    "internal_module",
    "jsx_element",
    "jsx_self_closing_element",
    "member_expression",
    "meta_property",
    "new_expression",
    "non_null_expression",
    "null",
    "number",
    "object",
    "parenthesized_expression",
    "regex",
    "satisfies_expression",
    "sequence_expression",
    "shorthand_property_identifier",
    "string",
    "subscript_expression",
    "super",
    "template_string",
    "ternary_expression",
    "this",
    "true",
    "type_assertion",
    "unary_expression",
    "undefined",
    "update_expression",
    "yield_expression",
];

/// Kinds whose value `decode` can know; any other expression is `Unknown`.
pub(crate) const VALUE_KINDS: &[&str] = &[
    "number",
    "string",
    "template_string",
    "true",
    "false",
    "null",
    "parenthesized_expression",
    "as_expression",
    "satisfies_expression",
    "non_null_expression",
    "type_assertion",
    "instantiation_expression",
    "array",
    "object",
    "regex",
    "undefined",
    "identifier",
    "unary_expression",
    "binary_expression",
    "ternary_expression",
    "sequence_expression",
    "assignment_expression",
    "member_expression",
    "subscript_expression",
    "call_expression",
];

/// A name's value through its binding (`value.follow`). An array, object or RegExp whose
/// properties a reference mutates has none, as `getStaticValue` reads it (`hasMutationInProperty`);
/// nor has a destructured name, which it does not read either.
fn follow<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    let Some(followed) = context.follow(node.range(), &node.text()) else {
        return Value::Unknown;
    };
    if followed.destructured {
        return Value::Unknown;
    }
    let Some(target) = followed.value.find_near(node.root().root(), node) else {
        return Value::Unknown;
    };
    match decode(&target, context) {
        Value::Array(_)
        | Value::RegExp(_)
        | Value::Container {
            kind: ContainerKind::Array | ContainerKind::Object,
            ..
        } if followed.mutated => Value::Unknown,
        value => value,
    }
}

/// Arithmetic, comparison and bitwise operators; `&&`, `||` and `??` give the operand that decides.
fn binary<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    let (Some(left), Some(operator), Some(right)) = (
        node.field("left"),
        node.field("operator"),
        node.field("right"),
    ) else {
        return Value::Unknown;
    };
    let operator = operator.text();
    let left = decode(&left, context);
    if !left.is_known() {
        return Value::Unknown;
    }
    let decided = match operator.as_ref() {
        "||" => left.truthy(Lang::Js),
        "&&" => left.truthy(Lang::Js).map(|truthy| !truthy),
        "??" => Some(!matches!(left, Value::None | Value::Undefined)),
        // `in` and `instanceof` are not folded.
        "in" | "instanceof" => return Value::Unknown,
        _ => return ops::binary(&operator, &left, &decode(&right, context)),
    };
    match decided {
        Some(true) => left,
        Some(false) => decode(&right, context),
        None => Value::Unknown,
    }
}

/// The expression a wrapper holds: its one named child that is not a type or a comment.
pub(super) fn operand<'r, D: Doc>(node: &Node<'r, D>) -> Option<Node<'r, D>> {
    let kind = node.kind();
    if kind == "type_assertion" {
        // `<T>x`: type arguments first, then the expression.
        return node
            .children()
            .filter(|child| child.is_named() && child.kind() != "comment")
            .last();
    }
    let mut named = node
        .children()
        .filter(|child| child.is_named() && child.kind() != "comment");
    let first = named.next()?;
    if kind == "parenthesized_expression" && named.next().is_some() {
        return None;
    }
    Some(first)
}

/// A numeric literal: decimal with `_` separators, `0x`/`0o`/`0b`, legacy octal (`010`) and legacy
/// decimal (`08`), and BigInts (`n`).
pub(crate) fn number(text: &str) -> Value {
    if !separators_valid(text) {
        return Value::Unknown;
    }
    if let Some(body) = text.strip_suffix('n') {
        let digits = body.replace('_', "");
        let lower = digits.to_ascii_lowercase();
        let (radix, digits) = radix_body(&lower);
        if radix == 10 && digits.len() > 1 && digits.starts_with('0') {
            return Value::Unknown;
        }
        return match i128::from_str_radix(digits, radix) {
            Ok(value) if !digits.is_empty() => Value::BigInt(value),
            _ => Value::Unknown,
        };
    }
    let lower = text.to_ascii_lowercase();
    let (radix, body) = radix_body(&lower);
    if radix != 10 {
        let digits = body.replace('_', "");
        if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
            return Value::Unknown;
        }
        // Past 128 bits the rounding would need a wider integer.
        return match u128::from_str_radix(&digits, radix) {
            Ok(value) => Value::Float(value as f64),
            Err(_) => Value::Unknown,
        };
    }
    if lower.len() > 1 && lower.starts_with('0') && lower.as_bytes()[1].is_ascii_digit() {
        // Legacy octal (`010`) when every digit is octal; legacy decimal (`08`, `09.5`) otherwise.
        if lower.contains('_') {
            return Value::Unknown;
        }
        if lower.bytes().all(|byte| (b'0'..=b'7').contains(&byte)) {
            return match u128::from_str_radix(&lower, 8) {
                Ok(value) => Value::Float(value as f64),
                Err(_) => Value::Unknown,
            };
        }
    }
    let digits = lower.replace('_', "");
    if !digits
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'e' | b'+' | b'-'))
    {
        return Value::Unknown;
    }
    digits.parse::<f64>().map_or(Value::Unknown, Value::Float)
}

/// A numeric separator stands between two digits, never after a radix prefix or next to `.`, an
/// exponent or another separator.
fn separators_valid(text: &str) -> bool {
    let bytes = text.as_bytes();
    let prefixed = bytes.len() > 2
        && bytes[0] == b'0'
        && matches!(bytes[1], b'x' | b'X' | b'o' | b'O' | b'b' | b'B');
    bytes.iter().enumerate().all(|(index, byte)| {
        *byte != b'_'
            || (index > 0
                && index + 1 < bytes.len()
                && bytes[index - 1].is_ascii_hexdigit()
                && bytes[index + 1].is_ascii_hexdigit()
                && !(prefixed && index == 2)
                && (prefixed || !matches!(bytes[index - 1], b'e' | b'E'))
                && (prefixed || !matches!(bytes[index + 1], b'e' | b'E')))
    })
}

fn radix_body(lower: &str) -> (u32, &str) {
    if let Some(body) = lower.strip_prefix("0x") {
        (16, body)
    } else if let Some(body) = lower.strip_prefix("0o") {
        (8, body)
    } else if let Some(body) = lower.strip_prefix("0b") {
        (2, body)
    } else {
        (10, lower)
    }
}

/// A string node's code units: a JSX attribute's value as JSX reads it, any other as a literal.
pub(crate) fn string_node<D: Doc>(node: &Node<'_, D>) -> Option<Vec<u16>> {
    let text = node.text();
    if node
        .parent()
        .is_some_and(|parent| parent.kind() == "jsx_attribute")
    {
        jsx_attribute_string(&text)
    } else {
        string(&text)
    }
}

/// A `'...'` or `"..."` literal's code units.
pub(crate) fn string(text: &str) -> Option<Vec<u16>> {
    let body = text
        .strip_prefix(['\'', '"'])
        .and_then(|rest| rest.strip_suffix(&text[..1]))?;
    let units = escapes(body, false)?;
    (units.len() * 2 <= MAX_STRING_BYTES).then_some(units)
}

/// A JSX attribute string's code units, as typescript-estree reads its `Literal.value`
/// (`unescapeStringLiteralText`): the text between the quotes as written, without escapes, where
/// `&#NNN;` and `&#xHH;` (lower-case `x`) up to U+10FFFF and the XHTML 1.0 entity names
/// (`entities.rs`) decode, and anything else, `&#X41;` and `&colon;` included, stays. A name on
/// `Object.prototype` (`&toString;`) reads a function from typescript-estree's table: `None`.
pub(crate) fn jsx_attribute_string(text: &str) -> Option<Vec<u16>> {
    let body = text
        .strip_prefix(['\'', '"'])
        .and_then(|rest| rest.strip_suffix(&text[..1]))?;
    let bytes = body.as_bytes();
    let mut out = Vec::with_capacity(body.len());
    let mut index = 0;
    while index < bytes.len() {
        let next = body[index..].find('&').map_or(bytes.len(), |at| index + at);
        out.extend(body[index..next].encode_utf16());
        if next == bytes.len() {
            break;
        }
        match character_reference(&body[next..])? {
            Some((units, length)) => {
                out.extend(units);
                index = next + length;
            }
            None => {
                out.push(u16::from(b'&'));
                index = next + 1;
            }
        }
        if out.len() * 2 > MAX_STRING_BYTES {
            return None;
        }
    }
    (out.len() * 2 <= MAX_STRING_BYTES).then_some(out)
}

/// The character reference `&...;` at the start of `text`, as typescript-estree's
/// `/&(?:#\d+|#x[\da-fA-F]+|[0-9a-zA-Z]+);/` finds it: its code units and length, `Some(None)`
/// where none starts here, and `None` where its value is not known.
fn character_reference(text: &str) -> Option<Option<(Vec<u16>, usize)>> {
    const PROTOTYPE: &[&str] = &[
        "constructor",
        "hasOwnProperty",
        "isPrototypeOf",
        "propertyIsEnumerable",
        "toLocaleString",
        "toString",
        "valueOf",
    ];
    let bytes = text.as_bytes();
    let run = |from: usize, test: fn(&u8) -> bool| {
        from + bytes[from..].iter().take_while(|byte| test(byte)).count()
    };
    let terminated = |from: usize, end: usize| end > from && bytes.get(end) == Some(&b';');
    let raw = |end: usize| Some(Some((text[..=end].encode_utf16().collect(), end + 1)));
    let numeric = |digits: &str, radix: u32, end: usize| {
        let digits = digits.trim_start_matches('0');
        let code = if digits.len() > 8 {
            None
        } else {
            u32::from_str_radix(if digits.is_empty() { "0" } else { digits }, radix).ok()
        };
        match code.filter(|code| *code <= 0x10FFFF) {
            Some(code) => {
                let mut units = Vec::with_capacity(2);
                push_code_point(&mut units, code)?;
                Some(Some((units, end + 1)))
            }
            // Past U+10FFFF `String.fromCodePoint` throws, and the reference stays.
            None => raw(end),
        }
    };
    if bytes.get(1) == Some(&b'#') {
        let end = run(2, u8::is_ascii_digit);
        if terminated(2, end) {
            return numeric(&text[2..end], 10, end);
        }
        if bytes.get(2) == Some(&b'x') {
            let end = run(3, u8::is_ascii_hexdigit);
            if terminated(3, end) {
                return numeric(&text[3..end], 16, end);
            }
        }
        return Some(None);
    }
    let end = run(1, u8::is_ascii_alphanumeric);
    if !terminated(1, end) {
        return Some(None);
    }
    let name = &text[1..end];
    if PROTOTYPE.contains(&name) {
        return None;
    }
    match entities::ENTITIES.binary_search_by(|(entry, _)| (*entry).cmp(name)) {
        Ok(found) => Some(Some((vec![entities::ENTITIES[found].1], end + 1))),
        Err(_) => raw(end),
    }
}

/// A template literal: its cooked chunks joined by its substitutions' strings (ToString of known
/// primitives). An escape a template cannot cook (legacy octal) makes it unknown.
fn template<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    let text = node.text();
    let start = node.range().start;
    if !text.starts_with('`') || !text.ends_with('`') || text.len() < 2 {
        return Value::Unknown;
    }
    let mut units = Vec::new();
    let mut chunk_start = 1;
    for substitution in node
        .children()
        .filter(|child| child.kind() == "template_substitution")
    {
        let range = substitution.range();
        let Some(chunk) = text.get(chunk_start..range.start - start) else {
            return Value::Unknown;
        };
        let Some(chunk) = cooked(chunk) else {
            return Value::Unknown;
        };
        units.extend(chunk);
        let Some(expression) = substitution
            .children()
            .find(|child| child.is_named() && child.kind() != "comment")
        else {
            return Value::Unknown;
        };
        let Some(string) = ops::to_string(&decode(&expression, context)) else {
            return Value::Unknown;
        };
        units.extend(string);
        if units.len() * 2 > MAX_STRING_BYTES {
            return Value::Unknown;
        }
        chunk_start = range.end - start;
    }
    let Some(chunk) = text.get(chunk_start..text.len() - 1).and_then(cooked) else {
        return Value::Unknown;
    };
    units.extend(chunk);
    if units.len() * 2 > MAX_STRING_BYTES {
        return Value::Unknown;
    }
    Value::Utf16(units)
}

/// A template chunk's cooked code units: line terminators normalized to LF, escapes decoded.
pub(super) fn cooked(chunk: &str) -> Option<Vec<u16>> {
    let normalized = chunk.replace("\r\n", "\n").replace('\r', "\n");
    let units = escapes(&normalized, true)?;
    (units.len() * 2 <= MAX_STRING_BYTES).then_some(units)
}

/// ECMAScript escape sequences, as UTF-16 code units. In a template, legacy octal and `\8`/`\9`
/// have no cooked value (`None`).
fn escapes(body: &str, template: bool) -> Option<Vec<u16>> {
    let mut out = Vec::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    let mut buffer = [0u16; 2];
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.extend_from_slice(c.encode_utf16(&mut buffer));
            continue;
        }
        let escape = chars.next()?;
        let unit = match escape {
            'b' => 8,
            'f' => 12,
            'n' => 10,
            'r' => 13,
            't' => 9,
            'v' => 11,
            // Line continuations.
            '\n' | '\u{2028}' | '\u{2029}' => continue,
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                continue;
            }
            'x' => {
                let digits = chars.by_ref().take(2).collect::<String>();
                if digits.len() != 2 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
                    return None;
                }
                u16::from_str_radix(&digits, 16).ok()?
            }
            'u' => {
                let code = if chars.peek() == Some(&'{') {
                    chars.next();
                    let digits = chars.by_ref().take_while(|c| *c != '}').collect::<String>();
                    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
                        return None;
                    }
                    u32::from_str_radix(&digits, 16).ok()?
                } else {
                    let digits = chars.by_ref().take(4).collect::<String>();
                    if digits.len() != 4 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
                        return None;
                    }
                    u32::from_str_radix(&digits, 16).ok()?
                };
                push_code_point(&mut out, code)?;
                continue;
            }
            '0' if !chars.peek().is_some_and(char::is_ascii_digit) => 0,
            '0'..='7' => {
                if template {
                    return None;
                }
                // LegacyOctalEscapeSequence: up to three digits from 0-3, two from 4-7.
                let mut code = escape.to_digit(8)?;
                let limit = if code <= 3 { 2 } else { 1 };
                for _ in 0..limit {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(digit) => {
                            code = code * 8 + digit;
                            chars.next();
                        }
                        None => break,
                    }
                }
                code as u16
            }
            '8' | '9' if template => return None,
            // Identity escapes, `\8` and `\9` included.
            other => {
                out.extend_from_slice(other.encode_utf16(&mut buffer));
                continue;
            }
        };
        out.push(unit);
    }
    Some(out)
}

/// A code point as UTF-16: surrogates stand alone, as JavaScript strings allow.
fn push_code_point(out: &mut Vec<u16>, code: u32) -> Option<()> {
    if code > 0x10FFFF {
        return None;
    }
    match char::from_u32(code) {
        Some(c) => {
            let mut buffer = [0u16; 2];
            out.extend_from_slice(c.encode_utf16(&mut buffer));
        }
        // A surrogate code point is one code unit.
        None => out.push(code as u16),
    }
    Some(())
}

/// An array literal: its values when all are known, holes as `undefined` and spreads of known
/// arrays and strings (by code point) inlined; otherwise its shape.
fn array<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    let mut values = Vec::new();
    let mut unknown = false;
    let mut spreads = 0;
    let mut elements = 0;
    // A comma after a comma (or after `[`) is a hole.
    let mut after_element = false;
    for child in node.children() {
        match child.kind().as_ref() {
            "[" | "]" | "comment" => {}
            "," => {
                if !after_element {
                    values.push(Value::Undefined);
                    elements += 1;
                }
                after_element = false;
            }
            "spread_element" => {
                after_element = true;
                let spread = child
                    .children()
                    .find(|inner| inner.is_named() && inner.kind() != "comment")
                    .map(|inner| decode(&inner, context));
                let items = match spread {
                    Some(Value::Array(items)) => items,
                    Some(Value::Utf16(units)) => char::decode_utf16(units.iter().copied())
                        .map(|c| match c {
                            Ok(c) => {
                                let mut buffer = [0u16; 2];
                                Value::Utf16(c.encode_utf16(&mut buffer).to_vec())
                            }
                            Err(lone) => Value::Utf16(vec![lone.unpaired_surrogate()]),
                        })
                        .collect(),
                    _ => {
                        spreads += 1;
                        unknown = true;
                        continue;
                    }
                };
                elements += items.len();
                values.extend(items);
                if values.len() > MAX_ELEMENTS {
                    return Value::Unknown;
                }
            }
            _ => {
                elements += 1;
                after_element = true;
                match decode(&child, context) {
                    Value::Unknown => unknown = true,
                    value => values.push(value),
                }
            }
        }
    }
    if !unknown {
        return Value::Array(values);
    }
    Value::Container {
        kind: ContainerKind::Array,
        len: (spreads == 0).then_some(elements),
        nonempty: if elements > 0 {
            Some(true)
        } else if spreads == 0 {
            Some(false)
        } else {
            None
        },
    }
}

/// A regex literal's RegExp object, holding `String(re)`: `/`, its source as written (a literal
/// needs no escaping), `/`, then its flags in `RegExp.prototype.flags`' order. Flags that repeat or
/// that ECMAScript does not define (an early error) make it unknown.
fn regexp<D: Doc>(node: &Node<'_, D>) -> Value {
    const FLAGS: &str = "dgimsuvy";
    let text = node.text();
    let Some(close) = text.rfind('/').filter(|close| *close > 0) else {
        return Value::Unknown;
    };
    let (body, flags) = (&text[..close], &text[close + 1..]);
    let mut seen = [false; FLAGS.len()];
    for flag in flags.chars() {
        match FLAGS.find(flag) {
            Some(index) if !seen[index] => seen[index] = true,
            _ => return Value::Unknown,
        }
    }
    // `u` and `v` together are an early error too.
    if seen[5] && seen[6] {
        return Value::Unknown;
    }
    let ordered = FLAGS
        .chars()
        .zip(seen)
        .filter_map(|(flag, set)| set.then_some(flag));
    let string = format!("{body}/{}", ordered.collect::<String>());
    if string.len() * 2 > MAX_STRING_BYTES {
        return Value::Unknown;
    }
    Value::RegExp(string.encode_utf16().collect())
}

/// An object literal: only that it is an object, which is truthy even when empty.
fn object<D: Doc>(node: &Node<'_, D>) -> Value {
    let members = node
        .children()
        .filter(|child| child.is_named() && child.kind() != "comment")
        .collect::<Vec<_>>();
    let spreads = members
        .iter()
        .filter(|member| member.kind() == "spread_element")
        .count();
    Value::Container {
        kind: ContainerKind::Object,
        len: None,
        nonempty: if spreads < members.len() {
            Some(true)
        } else if members.is_empty() {
            Some(false)
        } else {
            None
        },
    }
}

#[cfg(test)]
mod tests;
