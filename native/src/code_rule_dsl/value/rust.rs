//! Rust literal values as rustc evaluates them.

use ast_grep_core::{Doc, Node};
use num_bigint::BigInt;
use num_traits::{Num, ToPrimitive, Zero};

/// A Rust integer type (suffix or annotation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntTy {
    I8,
    I16,
    I32,
    I64,
    I128,
    Isize,
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
}
impl IntTy {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::I128 => "i128",
            Self::Isize => "isize",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::U128 => "u128",
            Self::Usize => "usize",
        }
    }
    fn limits(self) -> (BigInt, BigInt) {
        let bits = match self {
            Self::I8 | Self::U8 => 8,
            Self::I16 | Self::U16 => 16,
            Self::I32 | Self::U32 => 32,
            Self::I64 | Self::U64 => 64,
            Self::I128 | Self::U128 => 128,
            Self::Isize | Self::Usize => 64,
        };
        let one = BigInt::from(1u8);
        if self.name().starts_with('i') || self == Self::Isize {
            (
                -(one.clone() << (bits - 1usize)),
                (one << (bits - 1usize)) - 1,
            )
        } else {
            (BigInt::from(0), (one << bits) - 1)
        }
    }
}

/// A Rust float type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FloatTy {
    F32,
    F64,
}
impl FloatTy {
    #[allow(dead_code)]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }
}

/// A Rust literal value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustValue {
    Unknown,
    Bool(bool),
    Int { value: BigInt, ty: Option<IntTy> },
    Float { value: f64, ty: Option<FloatTy> },
    Char(char),
    Str(String),
    Bytes(Vec<u8>),
    CStr(Vec<u8>),
}

/// Decodes a tree-sitter-rust literal node.
pub(crate) fn literal<D: Doc>(node: &Node<'_, D>) -> RustValue {
    let text = node.text();
    match node.kind().as_ref() {
        "boolean_literal" => match text.as_ref() {
            "true" => RustValue::Bool(true),
            "false" => RustValue::Bool(false),
            _ => RustValue::Unknown,
        },
        "integer_literal" => integer(&text),
        "float_literal" => float(&text),
        // tree-sitter-rust reads `b'a'` as a `char_literal`, `b".."` and `c".."` as a
        // `string_literal` and `br".."` and `cr".."` as a `raw_string_literal`: the prefix tells them
        // apart.
        "char_literal" if text.starts_with('b') => quoted(&text, '\'', true)
            .and_then(|s| decode_escaped(s, EscapeMode::Byte))
            .and_then(|b| match b.as_slice() {
                [byte] => Some(RustValue::Int {
                    value: BigInt::from(*byte),
                    ty: Some(IntTy::U8),
                }),
                _ => None,
            })
            .unwrap_or(RustValue::Unknown),
        "char_literal" => quoted(&text, '\'', false)
            .and_then(|s| decode_escaped(s, EscapeMode::Char))
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|s| {
                let mut c = s.chars();
                Some(RustValue::Char(c.next()?)).filter(|_| c.next().is_none())
            })
            .unwrap_or(RustValue::Unknown),
        "string_literal" if text.starts_with('b') => quoted(&text, '"', true)
            .and_then(|s| decode_escaped(s, EscapeMode::ByteString))
            .map(RustValue::Bytes)
            .unwrap_or(RustValue::Unknown),
        "string_literal" if text.starts_with('c') => quoted(&text, '"', true)
            .and_then(|s| decode_escaped(s, EscapeMode::CStr))
            .filter(|b| !b.contains(&0))
            .map(RustValue::CStr)
            .unwrap_or(RustValue::Unknown),
        "string_literal" => quoted(&text, '"', false)
            .and_then(|s| decode_escaped(s, EscapeMode::String))
            .and_then(|b| String::from_utf8(b).ok())
            .map(RustValue::Str)
            .unwrap_or(RustValue::Unknown),
        "raw_string_literal" if text.starts_with("br") => raw_bytes(&text, b"br")
            .filter(|b| b.is_ascii())
            .map(RustValue::Bytes)
            .unwrap_or(RustValue::Unknown),
        "raw_string_literal" if text.starts_with("cr") => raw_bytes(&text, b"cr")
            .filter(|b| !b.contains(&0))
            .map(RustValue::CStr)
            .unwrap_or(RustValue::Unknown),
        "raw_string_literal" => raw(&text, b'r')
            .map(RustValue::Str)
            .unwrap_or(RustValue::Unknown),
        _ => RustValue::Unknown,
    }
}

fn integer(text: &str) -> RustValue {
    // A decimal literal with a float suffix is a float (`1f32`); after a base prefix `f32` is
    // hexadecimal digits (`0x10f64`).
    let based = ["0x", "0o", "0b"]
        .iter()
        .any(|prefix| text.starts_with(prefix));
    if !based && (text.ends_with("f32") || text.ends_with("f64")) {
        return float(text);
    }
    let (body, ty) = int_suffix(text);
    let clean: String = body.chars().filter(|c| *c != '_').collect();
    // Rust's base prefixes are lowercase only.
    let (radix, digits) = if let Some(x) = clean.strip_prefix("0x") {
        (16, x)
    } else if let Some(x) = clean.strip_prefix("0o") {
        (8, x)
    } else if let Some(x) = clean.strip_prefix("0b") {
        (2, x)
    } else {
        (10, clean.as_str())
    };
    let Some(value) = BigInt::from_str_radix(digits, radix).ok() else {
        return RustValue::Unknown;
    };
    let valid = ty.map_or(value.bits() <= 128, |t| {
        let (lo, hi) = t.limits();
        value >= lo && value <= hi
    });
    if valid {
        RustValue::Int { value, ty }
    } else {
        RustValue::Unknown
    }
}

fn float(text: &str) -> RustValue {
    let (body, ty) = float_suffix(text);
    let clean: String = body.chars().filter(|c| *c != '_').collect();
    let value = match ty {
        Some(FloatTy::F32) => clean.parse::<f32>().ok().map(f64::from),
        Some(FloatTy::F64) | None => clean.parse::<f64>().ok(),
    };
    value
        .filter(|value| value.is_finite())
        .map_or(RustValue::Unknown, |value| RustValue::Float { value, ty })
}

fn int_suffix(text: &str) -> (&str, Option<IntTy>) {
    for (suffix, ty) in [
        ("isize", IntTy::Isize),
        ("usize", IntTy::Usize),
        ("i128", IntTy::I128),
        ("u128", IntTy::U128),
        ("i64", IntTy::I64),
        ("u64", IntTy::U64),
        ("i32", IntTy::I32),
        ("u32", IntTy::U32),
        ("i16", IntTy::I16),
        ("u16", IntTy::U16),
        ("i8", IntTy::I8),
        ("u8", IntTy::U8),
    ] {
        if let Some(body) = text.strip_suffix(suffix) {
            if suffix.starts_with('i') || suffix.starts_with('u') || body.ends_with('_') {
                return (body, Some(ty));
            }
        }
    }
    (text, None)
}

fn float_suffix(text: &str) -> (&str, Option<FloatTy>) {
    if let Some(body) = text.strip_suffix("f32") {
        return (body, Some(FloatTy::F32));
    }
    if let Some(body) = text.strip_suffix("f64") {
        return (body, Some(FloatTy::F64));
    }
    (text, None)
}

fn quoted<'a>(text: &'a str, quote: char, prefix: bool) -> Option<&'a str> {
    let start = if prefix { 1 } else { 0 };
    text.get(start + 1..text.len().checked_sub(1)?).filter(|_| {
        text.as_bytes().get(start) == Some(&(quote as u8))
            && text.as_bytes().last() == Some(&(quote as u8))
    })
}
fn raw(text: &str, prefix: u8) -> Option<String> {
    raw_bytes(text, &[prefix]).and_then(|b| String::from_utf8(b).ok())
}
fn raw_bytes(text: &str, prefix: &[u8]) -> Option<Vec<u8>> {
    let t = text.as_bytes();
    if !t.starts_with(prefix) {
        return None;
    }
    let q = t[prefix.len()..].iter().position(|&x| x == b'"')? + prefix.len();
    let hashes = q - prefix.len();
    if !t[prefix.len()..q].iter().all(|&c| c == b'#') {
        return None;
    }
    let close = t.len().checked_sub(hashes + 1)?;
    if close <= q || t[close] != b'"' || !t[close + 1..].iter().all(|&c| c == b'#') {
        return None;
    }
    Some(t[q + 1..close].to_vec())
}

enum EscapeMode {
    Char,
    String,
    Byte,
    ByteString,
    CStr,
}
fn decode_escaped(s: &str, mode: EscapeMode) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    let mut o = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            if matches!(mode, EscapeMode::Byte | EscapeMode::ByteString) && bytes[i] >= 128 {
                return None;
            }
            o.push(bytes[i]);
            i += 1;
            continue;
        }
        i += 1;
        if i == bytes.len() {
            return None;
        }
        match bytes[i] {
            b'n' => o.push(b'\n'),
            b'r' => o.push(b'\r'),
            b't' => o.push(b'\t'),
            b'\\' => o.push(b'\\'),
            b'0' => o.push(0),
            b'\'' => o.push(b'\''),
            b'"' => o.push(b'"'),
            b'x' => {
                if i + 2 >= bytes.len() {
                    return None;
                }
                let v = hex(&bytes[i + 1..i + 3])?;
                if matches!(mode, EscapeMode::Char | EscapeMode::String) && v > 0x7f {
                    return None;
                }
                o.push(v);
                i += 2
            }
            b'u' => {
                if matches!(mode, EscapeMode::Byte | EscapeMode::ByteString) {
                    return None;
                }
                i += 1;
                if bytes.get(i) != Some(&b'{') {
                    return None;
                }
                i += 1;
                let st = i;
                while i < bytes.len() && bytes[i] != b'}' {
                    i += 1
                }
                if i == bytes.len() {
                    return None;
                }
                let h: String = bytes[st..i]
                    .iter()
                    .filter(|&&c| c != b'_')
                    .map(|&c| c as char)
                    .collect();
                if h.is_empty() || h.len() > 6 {
                    return None;
                }
                let v = u32::from_str_radix(&h, 16).ok()?;
                if v > 0x10ffff || (0xd800..=0xdfff).contains(&v) {
                    return None;
                }
                o.extend(char::from_u32(v)?.encode_utf8(&mut [0; 4]).as_bytes());
            }
            b'\n' => {
                i += 1;
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
                    i += 1
                }
                continue;
            }
            _ => return None,
        }
        i += 1;
    }
    if matches!(mode, EscapeMode::Char) && String::from_utf8(o.clone()).ok()?.chars().count() != 1 {
        return None;
    }
    Some(o)
}
fn hex(b: &[u8]) -> Option<u8> {
    Some(u8::from_str_radix(std::str::from_utf8(b).ok()?, 16).ok()?)
}

/// The value rustc gives the expression `node` (tree-sitter-rust), folding literals through
/// operators, casts, parentheses and constant `if`/block expressions; `RustValue::Unknown` for
/// anything else (names, paths, calls, macros) and for every expression rustc rejects.
pub(crate) fn evaluate<D: Doc>(node: &Node<'_, D>) -> RustValue {
    eval(node)
}

fn eval<D: Doc>(node: &Node<'_, D>) -> RustValue {
    match node.kind().as_ref() {
        "integer_literal" | "float_literal" | "boolean_literal" | "char_literal"
        | "string_literal" | "raw_string_literal" => literal(node),
        "parenthesized_expression" => node
            .children()
            .filter(|n| n.is_named())
            .next()
            .map_or(RustValue::Unknown, |n| eval(&n)),
        "unary_expression" => {
            let Some(op) = node.field("operator").or_else(|| node.children().next()) else {
                return RustValue::Unknown;
            };
            let Some(arg) = node
                .field("argument")
                .or_else(|| node.children().filter(|n| n.is_named()).next())
            else {
                return RustValue::Unknown;
            };
            let argument = eval(&arg);
            let argument = if matches!(argument, RustValue::Unknown) && op.text().as_ref() == "-" {
                min_literal(&arg).unwrap_or(RustValue::Unknown)
            } else {
                argument
            };
            unary(op.text().as_ref(), argument)
        }
        "binary_expression" => {
            let (Some(left), Some(op), Some(right)) = (
                node.field("left"),
                node.field("operator"),
                node.field("right"),
            ) else {
                return RustValue::Unknown;
            };
            binary(op.text().as_ref(), eval(&left), eval(&right))
        }
        "type_cast_expression" => {
            let mut named = node.children().filter(|n| n.is_named());
            let Some(value) = node.field("value").or_else(|| named.next()) else {
                return RustValue::Unknown;
            };
            let Some(ty) = node.field("type").or_else(|| named.next()) else {
                return RustValue::Unknown;
            };
            cast(eval(&value), ty.text().as_ref())
        }
        "if_expression" => {
            let (Some(condition), Some(consequence)) =
                (node.field("condition"), node.field("consequence"))
            else {
                return RustValue::Unknown;
            };
            match eval(&condition) {
                RustValue::Bool(true) => eval_block(&consequence),
                RustValue::Bool(false) => node
                    .field("alternative")
                    .map_or(RustValue::Unknown, |n| eval_block(&n)),
                _ => RustValue::Unknown,
            }
        }
        "block" => eval_block(node),
        _ => RustValue::Unknown,
    }
}

fn eval_block<D: Doc>(node: &Node<'_, D>) -> RustValue {
    node.children()
        .filter(|n| n.is_named())
        .last()
        .map_or(RustValue::Unknown, |n| eval(&n))
}

fn min_literal<D: Doc>(node: &Node<'_, D>) -> Option<RustValue> {
    let text = node.text();
    let text = text.trim_matches(|c| c == '(' || c == ')');
    let (body, ty) = int_suffix(text);
    let ty = ty?;
    if !ty.name().starts_with('i') {
        return None;
    }
    let value = BigInt::from_str_radix(body.trim_matches('_'), 10).ok()?;
    Some(RustValue::Int {
        value,
        ty: Some(ty),
    })
}

fn unary(op: &str, value: RustValue) -> RustValue {
    match (op, value) {
        ("!", RustValue::Bool(value)) => RustValue::Bool(!value),
        (
            "!",
            RustValue::Int {
                value,
                ty: Some(ty),
            },
        ) => RustValue::Int {
            value: wrap_int(!value, ty),
            ty: Some(ty),
        },
        // `!0` is -1 as an i32 and 255 as a u8: the context's type decides.
        ("!", RustValue::Int { ty: None, .. }) => RustValue::Unknown,
        (
            "-",
            RustValue::Int {
                value,
                ty: Some(ty),
            },
        ) if ty.name().starts_with('i') => {
            let value = -value;
            in_range(value, ty).map_or(RustValue::Unknown, |value| RustValue::Int {
                value,
                ty: Some(ty),
            })
        }
        ("-", RustValue::Int { value, ty: None }) => RustValue::Int {
            value: -value,
            ty: None,
        },
        ("-", RustValue::Float { value, ty }) => RustValue::Float { value: -value, ty },
        ("+", value @ RustValue::Int { .. }) | ("+", value @ RustValue::Float { .. }) => value,
        _ => RustValue::Unknown,
    }
}

fn binary(op: &str, left: RustValue, right: RustValue) -> RustValue {
    if matches!(op, "&&" | "||") {
        return match (left, right) {
            (RustValue::Bool(a), RustValue::Bool(b)) => {
                RustValue::Bool(if op == "&&" { a && b } else { a || b })
            }
            _ => RustValue::Unknown,
        };
    }
    if let (RustValue::Int { value: a, ty: ta }, RustValue::Int { value: b, ty: tb }) =
        (left.clone(), right.clone())
    {
        let ty = match (ta, tb) {
            (Some(a), Some(b)) if a != b => return RustValue::Unknown,
            (Some(a), _) | (_, Some(a)) => Some(a),
            _ => None,
        };
        if matches!(op, "==" | "!=" | "<" | "<=" | ">" | ">=") {
            return RustValue::Bool(compare_int(op, &a, &b));
        }
        let value = match op {
            "+" => a + b,
            "-" => a - b,
            "*" => a * b,
            "/" if !b.is_zero() => a / b,
            "%" if !b.is_zero() => a % b,
            "<<" | ">>" => {
                let Some(shift) = b.to_usize().filter(|s| *s < ty.map_or(128, int_bits)) else {
                    return RustValue::Unknown;
                };
                if op == "<<" { a << shift } else { a >> shift }
            }
            "&" => a & b,
            "|" => a | b,
            "^" => a ^ b,
            _ => return RustValue::Unknown,
        };
        return match ty {
            None => RustValue::Int { value, ty: None },
            Some(ty) => in_range(value, ty).map_or(RustValue::Unknown, |value| RustValue::Int {
                value,
                ty: Some(ty),
            }),
        };
    }
    if let (RustValue::Float { value: a, ty: ta }, RustValue::Float { value: b, ty: tb }) =
        (left.clone(), right.clone())
    {
        if ta.is_some() && tb.is_some() && ta != tb {
            return RustValue::Unknown;
        }
        let ty = ta.or(tb);
        let value = match op {
            "+" => a + b,
            "-" => a - b,
            "*" => a * b,
            "/" => a / b,
            "%" => a % b,
            "==" => return RustValue::Bool(a == b),
            "!=" => return RustValue::Bool(a != b),
            "<" => return RustValue::Bool(a < b),
            "<=" => return RustValue::Bool(a <= b),
            ">" => return RustValue::Bool(a > b),
            ">=" => return RustValue::Bool(a >= b),
            _ => return RustValue::Unknown,
        };
        return RustValue::Float {
            value: if ty == Some(FloatTy::F32) {
                f64::from(value as f32)
            } else {
                value
            },
            ty,
        };
    }
    if let (RustValue::Bool(a), RustValue::Bool(b)) = (left.clone(), right.clone()) {
        return match op {
            "&" => RustValue::Bool(a & b),
            "|" => RustValue::Bool(a | b),
            "^" => RustValue::Bool(a ^ b),
            "==" => RustValue::Bool(a == b),
            "!=" => RustValue::Bool(a != b),
            "<" => RustValue::Bool(a < b),
            "<=" => RustValue::Bool(a <= b),
            ">" => RustValue::Bool(a > b),
            ">=" => RustValue::Bool(a >= b),
            _ => RustValue::Unknown,
        };
    }
    if let (RustValue::Char(a), RustValue::Char(b)) = (left, right) {
        return match op {
            "==" => RustValue::Bool(a == b),
            "!=" => RustValue::Bool(a != b),
            "<" => RustValue::Bool(a < b),
            "<=" => RustValue::Bool(a <= b),
            ">" => RustValue::Bool(a > b),
            ">=" => RustValue::Bool(a >= b),
            _ => RustValue::Unknown,
        };
    }
    RustValue::Unknown
}

fn compare_int(op: &str, a: &BigInt, b: &BigInt) -> bool {
    match op {
        "==" => a == b,
        "!=" => a != b,
        "<" => a < b,
        "<=" => a <= b,
        ">" => a > b,
        ">=" => a >= b,
        _ => false,
    }
}

fn int_bits(ty: IntTy) -> usize {
    match ty {
        IntTy::I8 | IntTy::U8 => 8,
        IntTy::I16 | IntTy::U16 => 16,
        IntTy::I32 | IntTy::U32 => 32,
        IntTy::I64 | IntTy::U64 | IntTy::Isize | IntTy::Usize => 64,
        IntTy::I128 | IntTy::U128 => 128,
    }
}

fn in_range(value: BigInt, ty: IntTy) -> Option<BigInt> {
    let (lo, hi) = ty.limits();
    (value >= lo && value <= hi).then_some(value)
}

fn cast(value: RustValue, target: &str) -> RustValue {
    let target = target.trim();
    let int = [
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
    ]
    .iter()
    .find(|name| **name == target)
    .and_then(|name| int_suffix(name).1);
    if let Some(ty) = int {
        return match value {
            RustValue::Int { value, .. } => RustValue::Int {
                value: wrap_int(value, ty),
                ty: Some(ty),
            },
            RustValue::Float { value, .. } => {
                let (lo, hi) = ty.limits();
                let value = if value.is_nan() {
                    BigInt::from(0)
                } else if value <= lo.to_f64().unwrap_or(f64::MIN) {
                    lo
                } else if value >= hi.to_f64().unwrap_or(f64::MAX) {
                    hi
                } else {
                    BigInt::from(value.trunc() as i128)
                };
                RustValue::Int {
                    value,
                    ty: Some(ty),
                }
            }
            RustValue::Bool(value) => RustValue::Int {
                value: BigInt::from(value as u8),
                ty: Some(ty),
            },
            _ => RustValue::Unknown,
        };
    }
    if target == "char" {
        if let RustValue::Int { value, .. } = value {
            return value
                .to_u32()
                .and_then(char::from_u32)
                .map_or(RustValue::Unknown, RustValue::Char);
        }
    }
    if let Some(ty) = [("f32", FloatTy::F32), ("f64", FloatTy::F64)]
        .iter()
        .find(|(n, _)| *n == target)
        .map(|(_, t)| *t)
    {
        return match value {
            RustValue::Int { value, .. } => {
                value
                    .to_f64()
                    .map_or(RustValue::Unknown, |value| RustValue::Float {
                        value,
                        ty: Some(ty),
                    })
            }
            RustValue::Float { value, .. } => RustValue::Float {
                value: if ty == FloatTy::F32 {
                    f64::from(value as f32)
                } else {
                    value
                },
                ty: Some(ty),
            },
            _ => RustValue::Unknown,
        };
    }
    RustValue::Unknown
}

fn wrap_int(value: BigInt, ty: IntTy) -> BigInt {
    let bits = int_bits(ty);
    let modulus = BigInt::from(1u8) << bits;
    let mut value = ((value % &modulus) + &modulus) % &modulus;
    if ty.name().starts_with('i') && value >= (BigInt::from(1u8) << (bits - 1)) {
        value -= modulus;
    }
    value
}
#[cfg(test)]
mod tests;
