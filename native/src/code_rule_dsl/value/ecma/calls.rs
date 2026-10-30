//! Member reads and calls `getStaticValue` folds: a builtin global's function or constant
//! (`String(1)`, `Math.max(1, 2)`, `Math.PI`) where no file-local binding shadows the global, or
//! reached through a followed binding (`const f = String; f(1)`, `const M = Math; M.max(1, 2)`), and a
//! known string's, number's, array's or RegExp's method or property (`'POW'.toLowerCase()`,
//! `'a'.length`, `/a/g.flags`). A call folds only through a function on eslint-utils' allowlists
//! (`builtins.rs`, generated from its source) that is implemented here; anything else is unknown.

use crate::code_rule_dsl::language::Family;

use super::builtins::{BUILTIN_NAMES, CALL_ALLOWED, CALL_PASS_THROUGH, CONSTANTS, GETTER_ALLOWED};
use super::ops::{self, to_number, to_primitive, to_string};
use super::{decode, operand};
use crate::code_rule_dsl::value::key::{
    identifier_name, number_to_string, string_to_bigint, to_int32, to_uint32,
};
use crate::code_rule_dsl::value::{
    BigInt, ContainerKind, Context, MAX_ELEMENTS, MAX_STRING_BYTES, Value,
};
use ast_grep_core::{Doc, Node};
use num_traits::{Signed, ToPrimitive, Zero};

/// A member read's or call's value, and whether an optional chain short-circuited on the way
/// (`a?.b.c` with `a` null is `undefined`).
pub(super) struct Chained {
    pub(super) value: Value,
    short: bool,
}

impl Chained {
    fn unknown() -> Self {
        Self {
            value: Value::Unknown,
            short: false,
        }
    }

    fn value(value: Value) -> Self {
        Self {
            value,
            short: false,
        }
    }
}

/// What a member's object is: a builtin global itself (a namespace or a function, whose members
/// are read from the tables), or a value.
enum Object {
    Global(&'static str),
    Value(Value),
}

/// Pairs of work (needle length times haystack length) a string search may do; past it the result
/// is unknown.
const MAX_SEARCH_WORK: usize = 1 << 22;

/// A `member_expression` or `subscript_expression`'s value, or a `call_expression`'s.
pub(super) fn chained<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Chained {
    match node.kind().as_ref() {
        "member_expression" | "subscript_expression" => member(node, context),
        "call_expression" => call(node, context),
        _ => Chained::value(decode(node, context)),
    }
}

/// Whether the node has an optional chain of its own (`a?.b`, `a?.[0]`, `f?.()`).
fn optional<D: Doc>(node: &Node<'_, D>) -> bool {
    node.children()
        .any(|child| matches!(child.kind().as_ref(), "optional_chain" | "?."))
}

/// The expression inside any parentheses.
fn unparenthesized<'r, D: Doc>(node: &Node<'r, D>) -> Option<Node<'r, D>> {
    let mut inner = node.clone();
    while inner.kind() == "parenthesized_expression" {
        inner = operand(&inner)?;
    }
    Some(inner)
}

/// The builtin an identifier names: the unshadowed global itself, or, under `value.follow`, what
/// its binding's value names, through aliases (`const f = String`, `const m = Math.max`), as
/// `getStaticValue` reads a variable's function value. `Global.key` for a builtin's member. A
/// destructured name has no value there. Hops spent on a chain that names no builtin are given
/// back, since the caller then decodes the same chain as a value.
fn builtin_function<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Option<String> {
    let hops = context.hops;
    let found = follow_builtin(node, context);
    if found.is_none() {
        context.hops = hops;
    }
    found
}

fn follow_builtin<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Option<String> {
    let mut inner = unparenthesized(node)?;
    let root = node.root();
    loop {
        match inner.kind().as_ref() {
            "identifier" => {
                let name = inner.text();
                if let Some(global) = BUILTIN_NAMES
                    .iter()
                    .find(|global| **global == name.as_ref())
                    && context.is_global(inner.range(), global)
                {
                    return Some((*global).to_owned());
                }
                let followed = context.follow(inner.range(), &name)?;
                if followed.destructured {
                    return None;
                }
                let target = followed.value.find(root.root())?;
                inner = unparenthesized(&target)?;
            }
            "member_expression" if !optional(&inner) => {
                if inner
                    .field("property")
                    .is_none_or(|property| property.kind() != "property_identifier")
                {
                    return None;
                }
                let object = inner.field("object")?;
                let global = follow_builtin(&object, context)?;
                if global.contains('.')
                    || matches!(global.as_str(), "undefined" | "NaN" | "Infinity")
                {
                    return None;
                }
                let key = property(&inner, context)?;
                return Some(format!("{global}.{}", String::from_utf16_lossy(&key)));
            }
            _ => return None,
        }
    }
}

/// A member's object: a chained expression keeps its short circuit, parentheses aside.
fn object<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Option<(Object, bool)> {
    let inner = unparenthesized(node)?;
    if inner.kind() == "identifier"
        && let Some(name) = builtin_function(&inner, context)
        && let Some(global) = BUILTIN_NAMES.iter().find(|global| **global == name)
        && !matches!(*global, "undefined" | "NaN" | "Infinity")
    {
        return Some((Object::Global(global), false));
    }
    if !context.enter() {
        return None;
    }
    let chained = chained(&inner, context);
    context.leave();
    if context.exhausted() {
        return None;
    }
    // A chain in parentheses is a whole ChainExpression: its short circuit does not carry on.
    let short = chained.short && inner.range() == node.range();
    Some((Object::Value(chained.value), short))
}

/// The property key a member reads: a name, or a computed key's ToPropertyKey.
fn property<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Option<Vec<u16>> {
    if node.kind() == "subscript_expression" {
        let index = node.field("index")?;
        let key = decode(&index, context);
        return to_string(&key);
    }
    let property = node.field("property")?;
    match property.kind().as_ref() {
        "property_identifier" => identifier_name(&property.text()),
        _ => None,
    }
}

fn member<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Chained {
    let Some(object_node) = node.field("object") else {
        return Chained::unknown();
    };
    let Some((object, short)) = object(&object_node, context) else {
        return Chained::unknown();
    };
    if let Object::Value(Value::None | Value::Undefined) = &object
        && (short || optional(node))
    {
        return Chained {
            value: Value::Undefined,
            short: true,
        };
    }
    let Some(key) = property(node, context) else {
        return Chained::unknown();
    };
    let key = String::from_utf16_lossy(&key);
    let value = match object {
        Object::Global(global) => {
            let path = format!("{global}.{key}");
            CONSTANTS
                .iter()
                .find(|(name, _)| *name == path)
                .map_or(Value::Unknown, |(_, value)| Value::Float(*value))
        }
        Object::Value(value) => read(&value, &key),
    };
    Chained::value(value)
}

/// A canonical array index (`"0"`, `"12"`; not `"01"` or `"1.0"`).
fn index(key: &str) -> Option<usize> {
    if key.is_empty() || (key.len() > 1 && key.starts_with('0')) {
        return None;
    }
    let index = key.parse::<u32>().ok().filter(|index| *index < u32::MAX)?;
    usize::try_from(index).ok()
}

/// `value[key]` where `getStaticValue` reads it: a string's or an array's `length` and elements, a
/// RegExp's allowed getters and `lastIndex`. Anything else, a method included, is unknown.
fn read(value: &Value, key: &str) -> Value {
    match value {
        Value::Utf16(units) => {
            if key == "length" {
                return Value::Float(units.len() as f64);
            }
            match index(key) {
                Some(at) => units
                    .get(at)
                    .map_or(Value::Undefined, |unit| Value::Utf16(vec![*unit])),
                None => Value::Unknown,
            }
        }
        Value::Array(items) => {
            if key == "length" {
                return Value::Float(items.len() as f64);
            }
            match index(key) {
                Some(at) => items.get(at).cloned().unwrap_or(Value::Undefined),
                None => Value::Unknown,
            }
        }
        Value::RegExp(units) => {
            if key == "lastIndex" {
                return Value::Float(0.0);
            }
            if !GETTER_ALLOWED.contains(&format!("RegExp.prototype.{key}").as_str()) {
                return Value::Unknown;
            }
            let text = String::from_utf16_lossy(units);
            let Some(close) = text.rfind('/') else {
                return Value::Unknown;
            };
            let (source, flags) = (&text[1..close], &text[close + 1..]);
            let flag = |flag: char| Value::Bool(flags.contains(flag));
            match key {
                "source" => Value::Utf16(source.encode_utf16().collect()),
                "flags" => Value::Utf16(flags.encode_utf16().collect()),
                "hasIndices" => flag('d'),
                "global" => flag('g'),
                "ignoreCase" => flag('i'),
                "multiline" => flag('m'),
                "dotAll" => flag('s'),
                "unicode" => flag('u'),
                "unicodeSets" => flag('v'),
                "sticky" => flag('y'),
                _ => Value::Unknown,
            }
        }
        _ => Value::Unknown,
    }
}

/// The builtin a callee names: `String`, `Math.max`, `String.prototype.slice` with its receiver.
struct Callee {
    name: String,
    this: Option<Value>,
}

fn call<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Chained {
    let (Some(function), Some(arguments)) = (node.field("function"), node.field("arguments"))
    else {
        return Chained::unknown();
    };
    let Some(inner) = unparenthesized(&function) else {
        return Chained::unknown();
    };
    // A builtin global, or a name bound to a builtin function (`const f = String; f(1)`).
    let builtin = (inner.kind() == "identifier")
        .then(|| builtin_function(&inner, context))
        .flatten();
    let callee = match inner.kind().as_ref() {
        "identifier" if builtin.is_some() => Callee {
            name: builtin.unwrap_or_default(),
            this: None,
        },
        "member_expression" | "subscript_expression" => {
            if inner
                .field("property")
                .is_some_and(|property| property.kind() == "private_property_identifier")
            {
                return Chained::unknown();
            }
            let Some(object_node) = inner.field("object") else {
                return Chained::unknown();
            };
            let Some((object, short)) = object(&object_node, context) else {
                return Chained::unknown();
            };
            if let Object::Value(Value::None | Value::Undefined) = &object
                && (short || optional(&inner) || optional(node))
            {
                return Chained {
                    value: Value::Undefined,
                    short: true,
                };
            }
            let Some(key) = property(&inner, context) else {
                return Chained::unknown();
            };
            let key = String::from_utf16_lossy(&key);
            match object {
                Object::Global(global) => Callee {
                    name: format!("{global}.{key}"),
                    this: None,
                },
                Object::Value(value) => {
                    let owner = match &value {
                        Value::Utf16(_) => "String",
                        Value::Float(_) => "Number",
                        Value::Array(_) => "Array",
                        _ => return Chained::unknown(),
                    };
                    Callee {
                        name: format!("{owner}.prototype.{key}"),
                        this: Some(value),
                    }
                }
            }
        }
        // Only `f?.()` on a nullish `f` folds through any other callee.
        _ => {
            if !optional(node) {
                return Chained::unknown();
            }
            let Some((Object::Value(callee), _)) = object(&function, context) else {
                return Chained::unknown();
            };
            return match callee {
                Value::None | Value::Undefined => Chained {
                    value: Value::Undefined,
                    short: true,
                },
                _ => Chained::unknown(),
            };
        }
    };
    if arguments.kind() == "template_string" {
        // A tagged template folds only through `String.raw`.
        return Chained::value(if callee.name == "String.raw" && callee.this.is_none() {
            raw(&arguments, context)
        } else {
            Value::Unknown
        });
    }
    let Some(values) = argument_values(&arguments, context) else {
        return Chained::unknown();
    };
    if CALL_PASS_THROUGH.contains(&callee.name.as_str()) {
        return Chained::value(values.into_iter().next().unwrap_or(Value::Undefined));
    }
    if !CALL_ALLOWED.contains(&callee.name.as_str()) || !IMPLEMENTED.contains(&callee.name.as_str())
    {
        return Chained::unknown();
    }
    Chained::value(apply(&callee.name, callee.this.as_ref(), &values))
}

/// A call's argument values, spreads of known arrays and strings inlined; `None` where one is not
/// known.
fn argument_values<D: Doc>(
    arguments: &Node<'_, D>,
    context: &mut Context<'_>,
) -> Option<Vec<Value>> {
    let mut values = Vec::new();
    for child in arguments.children() {
        if !child.is_named() || child.kind() == "comment" {
            continue;
        }
        if child.kind() == "spread_element" {
            let inner = child
                .children()
                .find(|inner| inner.is_named() && inner.kind() != "comment")?;
            match decode(&inner, context) {
                Value::Array(items) => values.extend(items),
                Value::Utf16(units) => {
                    values.extend(char::decode_utf16(units.iter().copied()).map(|c| match c {
                        Ok(c) => {
                            let mut buffer = [0u16; 2];
                            Value::Utf16(c.encode_utf16(&mut buffer).to_vec())
                        }
                        Err(lone) => Value::Utf16(vec![lone.unpaired_surrogate()]),
                    }))
                }
                _ => return None,
            }
        } else {
            let value = decode(&child, context);
            if !value.is_known() || matches!(value, Value::Container { .. }) {
                return None;
            }
            values.push(value);
        }
        if values.len() > MAX_ELEMENTS {
            return None;
        }
    }
    Some(values)
}

/// `String.raw` over a template: its raw chunks (line terminators read as LF) joined by its
/// substitutions' strings.
fn raw<D: Doc>(template: &Node<'_, D>, context: &mut Context<'_>) -> Value {
    let text = template.text();
    let start = template.range().start;
    if !text.starts_with('`') || !text.ends_with('`') || text.len() < 2 {
        return Value::Unknown;
    }
    let chunk = |text: &str| -> Vec<u16> {
        text.replace("\r\n", "\n")
            .replace('\r', "\n")
            .encode_utf16()
            .collect()
    };
    let mut units = Vec::new();
    let mut chunk_start = 1;
    for substitution in template
        .children()
        .filter(|child| child.kind() == "template_substitution")
    {
        let range = substitution.range();
        let Some(before) = text.get(chunk_start..range.start - start) else {
            return Value::Unknown;
        };
        units.extend(chunk(before));
        let Some(expression) = substitution
            .children()
            .find(|child| child.is_named() && child.kind() != "comment")
        else {
            return Value::Unknown;
        };
        let Some(string) = to_string(&decode(&expression, context)) else {
            return Value::Unknown;
        };
        units.extend(string);
        if units.len() * 2 > MAX_STRING_BYTES {
            return Value::Unknown;
        }
        chunk_start = range.end - start;
    }
    let Some(last) = text.get(chunk_start..text.len() - 1) else {
        return Value::Unknown;
    };
    units.extend(chunk(last));
    if units.len() * 2 > MAX_STRING_BYTES {
        return Value::Unknown;
    }
    Value::Utf16(units)
}

/// The functions implemented, each under every name the allowlists give it. A test checks that
/// each is on an allowlist; a name on no list is never called.
pub(super) const IMPLEMENTED: &[&str] = &[
    "Array.isArray",
    "Array.of",
    "Array.prototype.at",
    "Array.prototype.concat",
    "Array.prototype.includes",
    "Array.prototype.indexOf",
    "Array.prototype.join",
    "Array.prototype.lastIndexOf",
    "Array.prototype.slice",
    "Array.prototype.toString",
    "BigInt",
    "BigInt.prototype.constructor",
    "Boolean",
    "Boolean.prototype.constructor",
    "Math.abs",
    "Math.ceil",
    "Math.clz32",
    "Math.floor",
    "Math.fround",
    "Math.imul",
    "Math.max",
    "Math.min",
    "Math.pow",
    "Math.round",
    "Math.sign",
    "Math.sqrt",
    "Math.trunc",
    "Number",
    "Number.isFinite",
    "Number.isNaN",
    "Number.parseFloat",
    "Number.parseInt",
    "Number.prototype.constructor",
    "Number.prototype.toFixed",
    "Number.prototype.toPrecision",
    "Number.prototype.toString",
    "Object.freeze",
    "Object.is",
    "Object.preventExtensions",
    "Object.seal",
    "String",
    "String.fromCharCode",
    "String.fromCodePoint",
    "String.prototype.at",
    "String.prototype.charAt",
    "String.prototype.charCodeAt",
    "String.prototype.codePointAt",
    "String.prototype.concat",
    "String.prototype.constructor",
    "String.prototype.endsWith",
    "String.prototype.includes",
    "String.prototype.indexOf",
    "String.prototype.lastIndexOf",
    "String.prototype.normalize",
    "String.prototype.padEnd",
    "String.prototype.padStart",
    "String.prototype.slice",
    "String.prototype.startsWith",
    "String.prototype.substr",
    "String.prototype.substring",
    "String.prototype.toLowerCase",
    "String.prototype.toString",
    "String.prototype.toUpperCase",
    "String.prototype.trim",
    "String.prototype.trimEnd",
    "String.prototype.trimLeft",
    "String.prototype.trimRight",
    "String.prototype.trimStart",
    "String.raw",
    "isFinite",
    "isNaN",
    "parseFloat",
    "parseInt",
];

fn argument(values: &[Value], at: usize) -> &Value {
    values.get(at).unwrap_or(&Value::Undefined)
}

/// ToNumber, through ToPrimitive (a RegExp is its string); `None` for a BigInt (a TypeError), an
/// array or an object.
fn number_of(value: &Value) -> Option<f64> {
    to_number(&to_primitive(value)?)
}

/// ToIntegerOrInfinity: NaN is 0, infinities stay, anything else truncated (never `-0`).
fn integer(value: &Value) -> Option<f64> {
    let number = number_of(value)?;
    Some(if number.is_nan() {
        0.0
    } else if number.is_infinite() {
        number
    } else {
        number.trunc() + 0.0
    })
}

/// A relative index (`slice`, `at`) clamped into `0..=len`.
fn relative(value: &Value, len: usize, default: f64) -> Option<usize> {
    let at = if matches!(value, Value::Undefined) {
        default
    } else {
        integer(value)?
    };
    let len = len as f64;
    Some(if at < 0.0 {
        (len + at).max(0.0)
    } else {
        at.min(len)
    } as usize)
}

/// ToString of a primitive or a RegExp; an array or an object converts in a way not modelled.
fn string_of(value: &Value) -> Option<Vec<u16>> {
    to_string(value)
}

fn utf16(text: &str) -> Value {
    Value::Utf16(text.encode_utf16().collect())
}

/// Whether the value is an object (a SameValue against it depends on identity).
fn object_like(value: &Value) -> bool {
    matches!(
        value,
        Value::Array(_) | Value::RegExp(_) | Value::Container { .. }
    )
}

/// The first index at or after `from` where `needle` occurs in `haystack`.
fn find(haystack: &[u16], needle: &[u16], from: usize) -> Option<Option<usize>> {
    if needle.len().saturating_mul(haystack.len()) > MAX_SEARCH_WORK {
        return None;
    }
    if from > haystack.len() {
        return Some(None);
    }
    if needle.is_empty() {
        return Some(Some(from));
    }
    Some(
        haystack[from..]
            .windows(needle.len())
            .position(|window| window == needle)
            .map(|at| at + from),
    )
}

/// The last index at or before `until` where `needle` occurs in `haystack`.
fn find_last(haystack: &[u16], needle: &[u16], until: usize) -> Option<Option<usize>> {
    if needle.len().saturating_mul(haystack.len()) > MAX_SEARCH_WORK {
        return None;
    }
    if needle.len() > haystack.len() {
        return Some(None);
    }
    let last = until.min(haystack.len() - needle.len());
    Some(
        (0..=last)
            .rev()
            .find(|at| haystack[*at..*at + needle.len()] == *needle),
    )
}

/// A `String.prototype` search argument: a RegExp throws (`IsRegExp`).
fn search_string(value: &Value) -> Option<Vec<u16>> {
    if matches!(value, Value::RegExp(_)) {
        return None;
    }
    string_of(value)
}

/// Calls the builtin `name` (on an allowlist) with `this` and known arguments; `Unknown` where it
/// is not implemented or would throw.
fn apply(name: &str, this: Option<&Value>, values: &[Value]) -> Value {
    let first = argument(values, 0);
    let second = argument(values, 1);
    let known = |value: Option<Value>| value.unwrap_or(Value::Unknown);
    match (name, this) {
        ("String" | "String.prototype.constructor", _) => {
            if values.is_empty() {
                return utf16("");
            }
            known(string_of(first).map(Value::Utf16))
        }
        ("Number" | "Number.prototype.constructor", _) => {
            if values.is_empty() {
                return Value::Float(0.0);
            }
            known(to_primitive(first).and_then(|primitive| match primitive {
                Value::BigInt(value) => Some(Value::Float(value as f64)),
                other => to_number(&other).map(Value::Float),
            }))
        }
        ("Boolean" | "Boolean.prototype.constructor", _) => {
            known(first.truthy(Family::Ecma).map(Value::Bool))
        }
        ("BigInt" | "BigInt.prototype.constructor", _) => known(to_bigint(first)),
        ("parseInt" | "Number.parseInt", _) => known(parse_int(first, second)),
        ("parseFloat" | "Number.parseFloat", _) => {
            known(string_of(first).map(|units| Value::Float(parse_float(&units))))
        }
        ("isNaN", _) => known(number_of(first).map(|number| Value::Bool(number.is_nan()))),
        ("isFinite", _) => known(number_of(first).map(|number| Value::Bool(number.is_finite()))),
        ("Number.isNaN", _) => {
            Value::Bool(matches!(first, Value::Float(number) if number.is_nan()))
        }
        ("Number.isFinite", _) => {
            Value::Bool(matches!(first, Value::Float(number) if number.is_finite()))
        }
        ("Object.is", _) => {
            if object_like(first) || object_like(second) {
                return Value::Unknown;
            }
            known(first.equals(second, Family::Ecma).map(Value::Bool))
        }
        ("Array.isArray", _) => Value::Bool(matches!(
            first,
            Value::Array(_)
                | Value::Container {
                    kind: ContainerKind::Array,
                    ..
                }
        )),
        ("Array.of", _) => Value::Array(values.to_vec()),
        ("String.fromCharCode", _) => {
            let mut units = Vec::with_capacity(values.len());
            for value in values {
                let Some(number) = number_of(value) else {
                    return Value::Unknown;
                };
                units.push((to_uint32(number) & 0xFFFF) as u16);
            }
            Value::Utf16(units)
        }
        ("String.fromCodePoint", _) => {
            let mut units = Vec::with_capacity(values.len());
            for value in values {
                let Some(number) = number_of(value) else {
                    return Value::Unknown;
                };
                if number.fract() != 0.0 || !(0.0..=1_114_111.0).contains(&number) {
                    return Value::Unknown;
                }
                let code = number as u32;
                match char::from_u32(code) {
                    Some(c) => {
                        let mut buffer = [0u16; 2];
                        units.extend_from_slice(c.encode_utf16(&mut buffer));
                    }
                    None => units.push(code as u16),
                }
            }
            Value::Utf16(units)
        }
        (math, _) if math.starts_with("Math.") => known(apply_math(&math[5..], values)),
        (_, Some(Value::Utf16(units))) => known(apply_string(name, units, values)),
        (_, Some(Value::Float(number))) => known(apply_number(name, *number, values)),
        (_, Some(Value::Array(items))) => known(apply_array(name, items, values)),
        _ => Value::Unknown,
    }
}

/// ToBigInt after ToPrimitive, as `BigInt(v)` converts: an integral Number exactly; a boolean; a
/// string through StringToBigInt. Only BigInts of 128 bits are values.
fn to_bigint(value: &Value) -> Option<Value> {
    let fit = |value: BigInt| value.to_i128().map(Value::BigInt);
    match to_primitive(value)? {
        Value::Float(number) => {
            if !number.is_finite() || number.fract() != 0.0 {
                return None;
            }
            fit(num_traits::FromPrimitive::from_f64(number)?)
        }
        Value::Bool(value) => Some(Value::BigInt(i128::from(value))),
        Value::BigInt(value) => Some(Value::BigInt(value)),
        Value::Utf16(units) => fit(string_to_bigint(&units)?),
        _ => None,
    }
}

fn is_white_space(unit: u16) -> bool {
    char::from_u32(u32::from(unit)).is_some_and(|c| {
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
    })
}

/// `parseInt(string, radix)`, exactly: a value past 2^53 is read correctly rounded in radix 10 and
/// the powers of two, as V8 reads it, and is unknown in any other radix.
fn parse_int(string: &Value, radix: &Value) -> Option<Value> {
    let units = string_of(string)?;
    let radix = if matches!(radix, Value::Undefined) {
        0
    } else {
        to_int32(number_of(radix)?)
    };
    let mut rest = &units[units
        .iter()
        .take_while(|unit| is_white_space(**unit))
        .count()..];
    let mut sign = 1.0;
    if let Some(first) = rest.first() {
        if *first == u16::from(b'-') {
            sign = -1.0;
        }
        if *first == u16::from(b'-') || *first == u16::from(b'+') {
            rest = &rest[1..];
        }
    }
    let mut radix = radix;
    let mut strip = true;
    if radix != 0 {
        if !(2..=36).contains(&radix) {
            return Some(Value::Float(f64::NAN));
        }
        if radix != 16 {
            strip = false;
        }
    } else {
        radix = 10;
    }
    if strip
        && rest.len() >= 2
        && rest[0] == u16::from(b'0')
        && (rest[1] == u16::from(b'x') || rest[1] == u16::from(b'X'))
    {
        rest = &rest[2..];
        radix = 16;
    }
    let radix = radix as u32;
    let digits = rest
        .iter()
        .map_while(|unit| char::from_u32(u32::from(*unit)).and_then(|c| c.to_digit(radix)))
        .collect::<Vec<_>>();
    if digits.is_empty() {
        return Some(Value::Float(f64::NAN));
    }
    let mut value = BigInt::zero();
    for digit in &digits {
        value = value * radix + *digit;
        if value.bits() > 1100 {
            return None;
        }
    }
    let exact = value.bits() <= 53;
    if !exact && radix != 10 && !radix.is_power_of_two() {
        return None;
    }
    let magnitude = crate::code_rule_dsl::value::int_to_float(&value)?;
    Some(Value::Float(sign * magnitude))
}

/// `parseFloat(string)`: the longest prefix, after white space, that is a StrDecimalLiteral.
fn parse_float(units: &[u16]) -> f64 {
    let text = String::from_utf16_lossy(
        &units[units
            .iter()
            .take_while(|unit| is_white_space(**unit))
            .count()..],
    );
    let bytes = text.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        end = 1;
    }
    if text[end..].starts_with("Infinity") {
        let sign = if bytes.first() == Some(&b'-') {
            -1.0
        } else {
            1.0
        };
        return sign * f64::INFINITY;
    }
    let digits = |from: usize| {
        from + bytes[from..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count()
    };
    let integer_end = digits(end);
    let mut last = integer_end;
    let mut count = integer_end - end;
    if bytes.get(last) == Some(&b'.') {
        let fraction_end = digits(last + 1);
        count += fraction_end - last - 1;
        last = fraction_end;
    }
    if count == 0 {
        return f64::NAN;
    }
    if matches!(bytes.get(last), Some(b'e' | b'E')) {
        let mut exponent = last + 1;
        if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
            exponent += 1;
        }
        let exponent_end = digits(exponent);
        if exponent_end > exponent {
            last = exponent_end;
        }
    }
    text[..last].parse::<f64>().unwrap_or(f64::NAN)
}

/// `Math` functions whose results are exact in IEEE 754 (or, for `pow`, the `**` operator's).
fn apply_math(name: &str, values: &[Value]) -> Option<Value> {
    let numbers = values.iter().map(number_of).collect::<Option<Vec<_>>>()?;
    let first = numbers.first().copied().unwrap_or(f64::NAN);
    let second = numbers.get(1).copied().unwrap_or(f64::NAN);
    Some(Value::Float(match name {
        "abs" => first.abs(),
        "ceil" => first.ceil(),
        "floor" => first.floor(),
        "trunc" => first.trunc(),
        "sqrt" => first.sqrt(),
        "fround" => f64::from(first as f32),
        "sign" => {
            if first.is_nan() || first == 0.0 {
                first
            } else {
                first.signum()
            }
        }
        "round" => {
            if !first.is_finite() || first == 0.0 {
                first
            } else if first > 0.0 && first < 0.5 {
                0.0
            } else if (-0.5..0.0).contains(&first) {
                -0.0
            } else {
                let floor = first.floor();
                if first - floor >= 0.5 {
                    floor + 1.0
                } else {
                    floor
                }
            }
        }
        "clz32" => f64::from(to_uint32(first).leading_zeros()),
        "imul" => f64::from((to_uint32(first) as i32).wrapping_mul(to_uint32(second) as i32)),
        "pow" => {
            return Some(ops::binary(
                "**",
                &Value::Float(first),
                &Value::Float(second),
            ));
        }
        "max" | "min" => {
            let max = name == "max";
            let mut result = if max {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
            let mut nan = false;
            for number in numbers {
                if number.is_nan() {
                    nan = true;
                } else if number == result && number == 0.0 {
                    // +0 is larger than -0.
                    if max == number.is_sign_positive() {
                        result = number;
                    }
                } else if (max && number > result) || (!max && number < result) {
                    result = number;
                }
            }
            if nan { f64::NAN } else { result }
        }
        _ => return None,
    }))
}

fn apply_string(name: &str, units: &[u16], values: &[Value]) -> Option<Value> {
    let method = name.strip_prefix("String.prototype.")?;
    let first = argument(values, 0);
    let second = argument(values, 1);
    let len = units.len();
    let string = |units: &[u16]| Value::Utf16(units.to_vec());
    Some(match method {
        "toString" => string(units),
        "at" => {
            let at = integer(first)?;
            let at = if at < 0.0 { len as f64 + at } else { at };
            if at < 0.0 || at >= len as f64 {
                Value::Undefined
            } else {
                string(&units[at as usize..=at as usize])
            }
        }
        "charAt" | "charCodeAt" | "codePointAt" => {
            let at = integer(first)?;
            if at < 0.0 || at >= len as f64 {
                return Some(match method {
                    "charAt" => utf16(""),
                    "charCodeAt" => Value::Float(f64::NAN),
                    _ => Value::Undefined,
                });
            }
            let at = at as usize;
            match method {
                "charAt" => string(&units[at..=at]),
                "charCodeAt" => Value::Float(f64::from(units[at])),
                _ => {
                    let code = char::decode_utf16(units[at..].iter().copied())
                        .next()
                        .map_or(u32::from(units[at]), |c| {
                            c.map_or(u32::from(units[at]), u32::from)
                        });
                    Value::Float(f64::from(code))
                }
            }
        }
        "concat" => {
            let mut out = units.to_vec();
            for value in values {
                out.extend(string_of(value)?);
                if out.len() * 2 > MAX_STRING_BYTES {
                    return None;
                }
            }
            Value::Utf16(out)
        }
        "startsWith" | "endsWith" | "includes" => {
            let needle = search_string(first)?;
            match method {
                "startsWith" => {
                    let start = relative_clamp(second, len, 0.0)?;
                    Value::Bool(units[start..].starts_with(&needle))
                }
                "endsWith" => {
                    let end = relative_clamp(second, len, len as f64)?;
                    Value::Bool(units[..end].ends_with(&needle))
                }
                _ => {
                    let start = relative_clamp(second, len, 0.0)?;
                    Value::Bool(find(units, &needle, start)?.is_some())
                }
            }
        }
        "indexOf" => {
            let needle = string_of(first)?;
            let start = relative_clamp(second, len, 0.0)?;
            Value::Float(find(units, &needle, start)?.map_or(-1.0, |at| at as f64))
        }
        "lastIndexOf" => {
            let needle = string_of(first)?;
            let position = number_of(second)?;
            let position = if position.is_nan() {
                f64::INFINITY
            } else {
                position.trunc()
            };
            let until = position.clamp(0.0, len as f64) as usize;
            Value::Float(find_last(units, &needle, until)?.map_or(-1.0, |at| at as f64))
        }
        "padStart" | "padEnd" => {
            let target = integer(first)?.clamp(0.0, 9_007_199_254_740_991.0);
            if target <= len as f64 {
                return Some(string(units));
            }
            let fill = if matches!(second, Value::Undefined) {
                vec![u16::from(b' ')]
            } else {
                string_of(second)?
            };
            if fill.is_empty() {
                return Some(string(units));
            }
            if target * 2.0 > MAX_STRING_BYTES as f64 {
                return None;
            }
            let pad = fill
                .iter()
                .copied()
                .cycle()
                .take(target as usize - len)
                .collect::<Vec<_>>();
            Value::Utf16(if method == "padStart" {
                [pad, units.to_vec()].concat()
            } else {
                [units.to_vec(), pad].concat()
            })
        }
        "slice" => {
            let start = relative(first, len, 0.0)?;
            let end = relative(second, len, len as f64)?;
            string(if start < end { &units[start..end] } else { &[] })
        }
        "substring" => {
            let start = relative_clamp(first, len, 0.0)?;
            let end = relative_clamp(second, len, len as f64)?;
            string(&units[start.min(end)..start.max(end)])
        }
        "substr" => {
            let start = integer(first)?;
            let start = if start == f64::NEG_INFINITY {
                0.0
            } else if start < 0.0 {
                (len as f64 + start).max(0.0)
            } else {
                start.min(len as f64)
            };
            let length = if matches!(second, Value::Undefined) {
                len as f64
            } else {
                integer(second)?
            };
            let length = length.clamp(0.0, len as f64);
            let end = (start + length).min(len as f64);
            string(if start < end {
                &units[start as usize..end as usize]
            } else {
                &[]
            })
        }
        "toLowerCase" | "toUpperCase" => {
            let text = String::from_utf16(units).ok()?;
            let mapped = if method == "toLowerCase" {
                text.to_lowercase()
            } else {
                text.to_uppercase()
            };
            let mapped = mapped.encode_utf16().collect::<Vec<_>>();
            if mapped.len() * 2 > MAX_STRING_BYTES {
                return None;
            }
            Value::Utf16(mapped)
        }
        "trim" | "trimStart" | "trimLeft" | "trimEnd" | "trimRight" => {
            let start = if matches!(method, "trimEnd" | "trimRight") {
                0
            } else {
                units
                    .iter()
                    .take_while(|unit| is_white_space(**unit))
                    .count()
            };
            let end = if matches!(method, "trimStart" | "trimLeft") {
                len
            } else {
                len - units[start..]
                    .iter()
                    .rev()
                    .take_while(|unit| is_white_space(**unit))
                    .count()
            };
            string(&units[start..end])
        }
        "normalize" => {
            let form = if matches!(first, Value::Undefined) {
                "NFC".encode_utf16().collect()
            } else {
                string_of(first)?
            };
            let form = String::from_utf16(&form).ok()?;
            if !matches!(form.as_str(), "NFC" | "NFD" | "NFKC" | "NFKD") {
                return None;
            }
            // Every normalization form leaves ASCII alone; other text is not normalized here.
            if !units.iter().all(|unit| *unit < 0x80) {
                return None;
            }
            string(units)
        }
        _ => return None,
    })
}

/// A position clamped into `0..=len` (`substring`, `startsWith`, `indexOf`), or the default where
/// it is `undefined`.
fn relative_clamp(value: &Value, len: usize, default: f64) -> Option<usize> {
    let at = if matches!(value, Value::Undefined) {
        default
    } else {
        integer(value)?
    };
    Some(at.clamp(0.0, len as f64) as usize)
}

fn apply_number(name: &str, number: f64, values: &[Value]) -> Option<Value> {
    let first = argument(values, 0);
    match name.strip_prefix("Number.prototype.")? {
        "toString" => {
            let radix = if matches!(first, Value::Undefined) {
                10.0
            } else {
                integer(first)?
            };
            if !(2.0..=36.0).contains(&radix) {
                return None;
            }
            if radix == 10.0 || !number.is_finite() {
                return Some(utf16(&number_to_string(number)));
            }
            if number.fract() != 0.0 || number.abs() >= 9_007_199_254_740_992.0 {
                return None;
            }
            let digits = BigInt::from(number.abs() as i64).to_str_radix(radix as u32);
            let sign = if number < 0.0 { "-" } else { "" };
            Some(utf16(&format!("{sign}{digits}")))
        }
        "toFixed" => {
            let digits = integer(first)?;
            if !(0.0..=100.0).contains(&digits) {
                return None;
            }
            Some(utf16(&to_fixed(number, digits as u32)?))
        }
        "toPrecision" => {
            if matches!(first, Value::Undefined) {
                return Some(utf16(&number_to_string(number)));
            }
            let digits = integer(first)?;
            if !number.is_finite() {
                return Some(utf16(&number_to_string(number)));
            }
            if !(1.0..=100.0).contains(&digits) {
                return None;
            }
            Some(utf16(&to_precision(number, digits as u32)))
        }
        _ => None,
    }
}

/// A finite non-negative double as the exact rational `mantissa * 2^exponent`.
fn exact(magnitude: f64) -> (BigInt, i64) {
    let bits = magnitude.to_bits();
    let exponent = ((bits >> 52) & 0x7FF) as i64;
    let fraction = bits & ((1 << 52) - 1);
    if exponent == 0 {
        (BigInt::from(fraction), -1074)
    } else {
        (BigInt::from(fraction | (1 << 52)), exponent - 1075)
    }
}

/// The integer nearest `mantissa * 2^exponent * 10^scale`, the larger on a tie.
fn rounded(mantissa: &BigInt, exponent: i64, scale: i64) -> BigInt {
    let ten = BigInt::from(10u32);
    let mut numerator = mantissa.clone();
    let mut denominator = BigInt::from(1u8);
    if scale >= 0 {
        numerator *= ten.pow(scale as u32);
    } else {
        denominator *= ten.pow((-scale) as u32);
    }
    if exponent >= 0 {
        numerator <<= exponent as usize;
    } else {
        denominator <<= (-exponent) as usize;
    }
    let two = BigInt::from(2u8);
    (&numerator * &two + &denominator) / (&denominator * &two)
}

/// `Number.prototype.toPrecision(precision)` of a finite `number`, exactly (ECMA-262
/// §21.1.3.5): the `precision` significant digits `n` and exponent `e` for which
/// `n * 10^(e - precision + 1)` is nearest the number, the larger on a tie, laid out in fixed
/// notation for `-6 <= e < precision` and in exponential notation otherwise.
pub(crate) fn to_precision(number: f64, precision: u32) -> String {
    debug_assert!(number.is_finite() && (1..=100).contains(&precision));
    let sign = if number < 0.0 { "-" } else { "" };
    let magnitude = number.abs();
    let p = precision as i64;
    let (digits, e) = if magnitude == 0.0 {
        ("0".repeat(precision as usize), 0)
    } else {
        let (mantissa, exponent) = exact(magnitude);
        let low = BigInt::from(10u32).pow(precision - 1);
        let high = &low * BigInt::from(10u32);
        // The estimate is off by at most one either way; the exact rounding settles it.
        let mut e = magnitude.log10().floor() as i64;
        let n = loop {
            let n = rounded(&mantissa, exponent, p - 1 - e);
            if n >= high {
                e += 1;
            } else if n < low {
                e -= 1;
            } else {
                break n;
            }
        };
        (n.to_string(), e)
    };
    let text = if e < -6 || e >= p {
        let (first, rest) = digits.split_at(1);
        let point = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let exponent_sign = if e >= 0 { "+" } else { "-" };
        format!("{first}{point}e{exponent_sign}{}", e.abs())
    } else if e == p - 1 {
        digits
    } else if e >= 0 {
        let (whole, fraction) = digits.split_at(e as usize + 1);
        format!("{whole}.{fraction}")
    } else {
        format!("0.{}{digits}", "0".repeat((-(e + 1)) as usize))
    };
    format!("{sign}{text}")
}

/// `Number.prototype.toFixed`, exactly: the integer `n` nearest `x * 10^f`, the larger on a tie.
fn to_fixed(number: f64, digits: u32) -> Option<String> {
    if number.is_nan() {
        return Some("NaN".to_owned());
    }
    if number.abs() >= 1e21 || number.is_infinite() {
        return Some(number_to_string(number));
    }
    let sign = if number < 0.0 { "-" } else { "" };
    let magnitude = number.abs();
    let bits = magnitude.to_bits();
    let exponent = ((bits >> 52) & 0x7FF) as i64;
    let fraction = bits & ((1 << 52) - 1);
    let (mantissa, exponent) = if exponent == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), exponent - 1075)
    };
    let scale = BigInt::from(10u32).pow(digits);
    let mut numerator = BigInt::from(mantissa) * scale;
    let mut denominator = BigInt::from(1u8);
    if exponent >= 0 {
        numerator <<= exponent as usize;
    } else {
        denominator <<= (-exponent) as usize;
    }
    let two = BigInt::from(2u8);
    let rounded: BigInt = (&numerator * &two + &denominator) / (&denominator * &two);
    let mut text = rounded.abs().to_string();
    if digits > 0 {
        let width = digits as usize + 1;
        if text.len() < width {
            text = format!("{}{text}", "0".repeat(width - text.len()));
        }
        text.insert(text.len() - digits as usize, '.');
    }
    Some(format!("{sign}{text}"))
}

fn apply_array(name: &str, items: &[Value], values: &[Value]) -> Option<Value> {
    let first = argument(values, 0);
    let second = argument(values, 1);
    let len = items.len();
    Some(match name.strip_prefix("Array.prototype.")? {
        "at" => {
            let at = integer(first)?;
            let at = if at < 0.0 { len as f64 + at } else { at };
            if at < 0.0 || at >= len as f64 {
                Value::Undefined
            } else {
                items[at as usize].clone()
            }
        }
        "concat" => {
            let mut out = items.to_vec();
            for value in values {
                match value {
                    Value::Array(more) => out.extend(more.iter().cloned()),
                    Value::Container { .. } => return None,
                    other => out.push(other.clone()),
                }
                if out.len() > MAX_ELEMENTS {
                    return None;
                }
            }
            Value::Array(out)
        }
        "includes" | "indexOf" => {
            if object_like(first) {
                return None;
            }
            // A hole reads as `undefined`, which `indexOf` skips and `includes` does not; the
            // array here cannot tell them apart.
            if name.ends_with("indexOf") && matches!(first, Value::Undefined) {
                return None;
            }
            let from = integer(second)?;
            if from >= len as f64 {
                return Some(if name.ends_with("includes") {
                    Value::Bool(false)
                } else {
                    Value::Float(-1.0)
                });
            }
            let from = if from < 0.0 {
                (len as f64 + from).max(0.0)
            } else {
                from
            } as usize;
            let mut found = None;
            for (at, item) in items.iter().enumerate().skip(from) {
                let equal = match (item, first) {
                    (Value::Float(left), Value::Float(right)) => {
                        left == right
                            || (name.ends_with("includes") && left.is_nan() && right.is_nan())
                    }
                    _ => !object_like(item) && item.equals(first, Family::Ecma)?,
                };
                if equal {
                    found = Some(at);
                    break;
                }
            }
            if name.ends_with("includes") {
                Value::Bool(found.is_some())
            } else {
                Value::Float(found.map_or(-1.0, |at| at as f64))
            }
        }
        "lastIndexOf" => {
            if object_like(first) || matches!(first, Value::Undefined) {
                return None;
            }
            if len == 0 {
                return Some(Value::Float(-1.0));
            }
            let from = if values.len() > 1 {
                integer(second)?
            } else {
                len as f64 - 1.0
            };
            let from = if from < 0.0 {
                len as f64 + from
            } else {
                from.min(len as f64 - 1.0)
            };
            if from < 0.0 {
                return Some(Value::Float(-1.0));
            }
            let found = (0..=from as usize)
                .rev()
                .find(|at| match (&items[*at], first) {
                    (Value::Float(left), Value::Float(right)) => left == right,
                    (item, first) => {
                        !object_like(item) && item.equals(first, Family::Ecma) == Some(true)
                    }
                });
            Value::Float(found.map_or(-1.0, |at| at as f64))
        }
        "join" | "toString" => {
            let separator = if name.ends_with("toString") || matches!(first, Value::Undefined) {
                vec![u16::from(b',')]
            } else {
                string_of(first)?
            };
            Value::Utf16(join(items, &separator, 0)?)
        }
        "slice" => {
            let start = relative(first, len, 0.0)?;
            let end = relative(second, len, len as f64)?;
            Value::Array(if start < end {
                items[start..end].to_vec()
            } else {
                Vec::new()
            })
        }
        _ => return None,
    })
}

/// `Array.prototype.join`: `undefined` and `null` elements are empty, nested arrays join with
/// commas.
fn join(items: &[Value], separator: &[u16], depth: usize) -> Option<Vec<u16>> {
    if depth > 32 {
        return None;
    }
    let mut out = Vec::new();
    for (at, item) in items.iter().enumerate() {
        if at > 0 {
            out.extend_from_slice(separator);
        }
        match item {
            Value::Undefined | Value::None => {}
            Value::Array(inner) => out.extend(join(inner, &[u16::from(b',')], depth + 1)?),
            other => out.extend(string_of(other)?),
        }
        if out.len() * 2 > MAX_STRING_BYTES {
            return None;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Number.prototype.toPrecision` as Node.js 25 (V8) computes it, exactly: ties go to the
    /// larger magnitude, and the layout turns exponential below `e-6` and from `precision` digits.
    #[test]
    fn to_precision_is_ecmascripts() {
        for (number, precision, expected) in [
            (0.0, 1, "0"),
            (-0.0, 3, "0.00"),
            (1.0, 1, "1"),
            (123.456, 4, "123.5"),
            (0.000123, 2, "0.00012"),
            (1e21, 3, "1.00e+21"),
            (123456.0, 2, "1.2e+5"),
            (9223372036854775808.0, 19, "9223372036854775808"),
            (9223372036854775808.0, 20, "9223372036854775808.0"),
            (0.1, 20, "0.10000000000000000555"),
            (5e-324, 3, "4.94e-324"),
            (
                f64::MAX,
                100,
                "1.797693134862315708145274237317043567980705675258449965989174768031572607800285387605895586327668782e+308",
            ),
            (2.5, 1, "3"),
            (3.5, 1, "4"),
            (-1.25, 2, "-1.3"),
            (0.00001, 1, "0.00001"),
            (0.000001, 1, "0.000001"),
            (1e-7, 1, "1e-7"),
            (99.99, 3, "100"),
            (99.95, 3, "100"),
            (1.005, 3, "1.00"),
            (255.0, 2, "2.6e+2"),
            (1.0 / 3.0, 17, "0.33333333333333331"),
            (12345678901234567890.0, 21, "12345678901234567168.0"),
            (-987.654321, 10, "-987.6543210"),
        ] {
            assert_eq!(to_precision(number, precision), expected, "{number}");
        }
    }

    /// The evaluator's `toPrecision`: `String(x)` without an argument or for a non-finite `x`,
    /// and no value (a RangeError) outside 1 to 100.
    #[test]
    fn to_precision_calls_follow_the_specification() {
        let call = |number: f64, values: &[Value]| {
            apply_number("Number.prototype.toPrecision", number, values)
        };
        assert_eq!(call(0.5, &[]), Some(utf16("0.5")));
        assert_eq!(
            call(f64::INFINITY, &[Value::Float(0.0)]),
            Some(utf16("Infinity"))
        );
        assert_eq!(call(0.5, &[Value::Float(2.0)]), Some(utf16("0.50")));
        assert_eq!(call(0.5, &[Value::Float(0.0)]), None);
        assert_eq!(call(0.5, &[Value::Float(101.0)]), None);
    }
}
