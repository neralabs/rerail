//! SQL literal, identifier, and equality semantics shared by the generic SQL dialect.
//!
//! PostgreSQL's scanner (`scan.l`, `scansup.c`, notably `downcase_identifier` and
//! `truncate_identifier`) and SQLite's tokenizer are deliberately conservative here:
//! a value is returned only when their readings agree.

use crate::code_rule_dsl::tree::Tree;
use super::{Value, int_float_order};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::borrow::Cow;
use std::cmp::Ordering;

fn digits_ok(s: &str, radix: u32) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b[0] != b'_'
        && b[b.len() - 1] != b'_'
        && !s.contains("__")
        && s.chars().all(|c| c == '_' || c.is_digit(radix))
}

fn integer(text: &str, radix: u32) -> Option<Value> {
    let clean = text.replace('_', "");
    Some(Value::Int(BigInt::parse_bytes(clean.as_bytes(), radix)?))
}

fn fits_i64(v: Value) -> Value {
    match v {
        Value::Int(n) if n.to_i64().is_some() => Value::Int(n),
        _ => Value::Unknown,
    }
}

fn decimal_form(s: &str) -> Option<(bool, String, i32)> {
    let mut s = s;
    let neg = if let Some(x) = s.strip_prefix('-') {
        s = x;
        true
    } else if let Some(x) = s.strip_prefix('+') {
        s = x;
        false
    } else {
        false
    };
    let (mant, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], s[i + 1..].parse::<i32>().ok()?),
        None => (s, 0),
    };
    if mant.is_empty() || mant.matches('.').count() > 1 {
        return None;
    }
    let (a, b) = mant.split_once('.').unwrap_or((mant, ""));
    if a.is_empty() && b.is_empty()
        || (!a.is_empty() && !digits_ok(a, 10))
        || (!b.is_empty() && !digits_ok(b, 10))
    {
        return None;
    }
    let mut digs = format!("{}{}", a.replace('_', ""), b.replace('_', ""));
    // An exponent near `i32`'s bounds puts the scale outside them: PostgreSQL's `numeric_in`
    // refuses such a literal, so no value is read.
    let fraction = i32::try_from(b.replace('_', "").len()).ok()?;
    let mut scale = exp.checked_sub(fraction)?;
    let trimmed = digs.trim_start_matches('0').to_owned();
    digs = if trimmed.is_empty() {
        "0".into()
    } else {
        trimmed
    };
    while digs.len() > 1 && digs.ends_with('0') {
        digs.pop();
        scale = scale.checked_add(1)?;
    }
    Some((neg, digs, scale))
}

fn float_literal(s: &str) -> Option<Value> {
    let (neg, digits, scale) = decimal_form(s)?;
    let f = s.replace('_', "").parse::<f64>().ok()?;
    if !f.is_finite() {
        return None;
    }
    let mut repr = format!("{f:?}");
    if repr == "-0.0" {
        repr = "0.0".into();
    }
    let (_, rd, rs) = decimal_form(&repr)?;
    if neg != (f.is_sign_negative() && f != 0.0) || digits != rd || scale != rs {
        return None;
    }
    Some(Value::Float(f))
}

fn quoted_body(s: &str, quote: char) -> Option<(String, usize)> {
    scan_quoted(s, quote, false)
}

/// The body of the `quote`-delimited text at the start of `s` (doubled quotes collapsed)
/// and the bytes used; with `backslashes`, a backslash keeps the next character from ending it.
fn scan_quoted(s: &str, quote: char, backslashes: bool) -> Option<(String, usize)> {
    if !s.starts_with(quote) {
        return None;
    }
    let mut iter = s[quote.len_utf8()..].char_indices();
    let mut out = String::new();
    while let Some((offset, c)) = iter.next() {
        if c == quote {
            if let Some((_next_offset, next)) = iter.clone().next() {
                if next == quote {
                    out.push(quote);
                    iter.next();
                    continue;
                }
                return Some((out, quote.len_utf8() + offset + quote.len_utf8()));
            }
            return Some((out, quote.len_utf8() + offset + quote.len_utf8()));
        }
        if backslashes && c == '\\' {
            out.push(c);
            out.push(iter.next()?.1);
            continue;
        }
        out.push(c);
    }
    None
}

/// An `E''` string's value. Octal and hexadecimal escapes are bytes (PostgreSQL's `scan.l` keeps
/// an octal escape's low byte); the whole is a value only as valid UTF-8 without a NUL, which a
/// UTF-8 database requires.
fn e_string(s: &str) -> Option<String> {
    let (raw, used) = scan_quoted(s, '\'', true)?;
    if used != s.len() {
        return None;
    }
    let mut out: Vec<u8> = Vec::with_capacity(raw.len());
    let push = |out: &mut Vec<u8>, c: char| {
        out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
    };
    let b: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] != '\\' {
            push(&mut out, b[i]);
            i += 1;
            continue;
        }
        i += 1;
        if i == b.len() {
            return None;
        }
        let c = b[i];
        i += 1;
        let mapped = match c {
            'b' => '\u{8}',
            'f' => '\u{c}',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '\\' => '\\',
            '\'' => '\'',
            _ => '\0',
        };
        if mapped != '\0' {
            push(&mut out, mapped);
            continue;
        }
        if ('0'..='7').contains(&c) {
            let mut v = c as u32 - '0' as u32;
            for _ in 0..2 {
                if i < b.len() && ('0'..='7').contains(&b[i]) {
                    v = v * 8 + (b[i] as u32 - '0' as u32);
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(v as u8);
        } else if c == 'x' {
            let n = (1..=2)
                .rev()
                .find(|n| i + n <= b.len() && b[i..i + n].iter().all(char::is_ascii_hexdigit))
                .unwrap_or(0);
            if n == 0 {
                return None;
            }
            let digits: String = b[i..i + n].iter().collect();
            out.push(u8::from_str_radix(&digits, 16).ok()?);
            i += n;
        } else if c == 'u' || c == 'U' {
            let n = if c == 'u' { 4 } else { 8 };
            if i + n > b.len() || !b[i..i + n].iter().all(char::is_ascii_hexdigit) {
                return None;
            }
            let digits: String = b[i..i + n].iter().collect();
            push(
                &mut out,
                char::from_u32(u32::from_str_radix(&digits, 16).ok()?)?,
            );
            i += n;
        } else {
            push(&mut out, c);
        }
    }
    if out.contains(&0) {
        return None;
    }
    String::from_utf8(out).ok()
}

fn plain_string(s: &str) -> Option<String> {
    let (first, mut i) = quoted_body(s, '\'')?;
    let mut out = first;
    while i < s.len() {
        let rest = &s[i..];
        let mut p = 0;
        let mut newline = false;
        loop {
            let r = &rest[p..];
            if r.starts_with("--") {
                let n = r.find('\n')?;
                p += n + 1;
                newline = true;
                continue;
            }
            if r.starts_with("/*") {
                let n = r.find("*/")? + 2;
                newline |= r[..n].contains('\n');
                p += n;
                continue;
            }
            let Some(c) = r.chars().next() else { break };
            if c.is_whitespace() {
                newline |= c == '\n';
                p += c.len_utf8();
                continue;
            }
            break;
        }
        if !newline {
            return None;
        }
        i += p;
        let (part, used) = quoted_body(&s[i..], '\'')?;
        out.push_str(&part);
        i += used;
    }
    Some(out)
}

/// Whether `s` opens a Unicode-escaped string (`quote` `'`) or identifier (`"`): `U&` or `u&`.
fn unicode_prefixed(s: &str, quote: char) -> bool {
    s.strip_prefix(['U', 'u'])
        .and_then(|rest| rest.strip_prefix('&'))
        .is_some_and(|rest| rest.starts_with(quote))
}

fn unicode_string(s: &str, ident: bool) -> Option<String> {
    let q = if ident { '\"' } else { '\'' };
    if !unicode_prefixed(s, q) {
        return None;
    }
    let (raw, used) = quoted_body(&s[2..], q)?;
    let mut esc = '\\';
    let tail = &s[2 + used..];
    if let Some(t) = tail
        .strip_prefix(" UESCAPE ")
        .or_else(|| tail.strip_prefix(" UESCAPE"))
    {
        let (x, n) = quoted_body(t.trim_start(), '\'')?;
        if n != t.trim_start().len() || x.chars().count() != 1 {
            return None;
        }
        esc = x.chars().next()?;
    } else if !tail.is_empty() {
        return None;
    }
    let mut out = String::new();
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != esc {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        i += 1;
        if i == chars.len() {
            return None;
        }
        if chars[i] == esc {
            out.push(esc);
            i += 1;
            continue;
        }
        let n = if chars[i] == '+' {
            i += 1;
            6
        } else {
            4
        };
        if i + n > chars.len() {
            return None;
        }
        let h: String = chars[i..i + n].iter().collect();
        if !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        out.push(char::from_u32(u32::from_str_radix(&h, 16).ok()?)?);
        i += n;
    }
    Some(out)
}

pub fn literal(text: &str) -> Value {
    let upper = text.to_ascii_uppercase();
    if upper == "NULL" {
        return Value::None;
    }
    if upper == "TRUE" {
        return Value::Bool(true);
    }
    if upper == "FALSE" {
        return Value::Bool(false);
    }
    if text.starts_with("$$") || (text.starts_with('$') && text[1..].find('$').is_some()) {
        if let Some(p) = text[1..].find('$') {
            let tag = &text[..p + 2];
            return match text.strip_prefix(tag).and_then(|x| x.strip_suffix(tag)) {
                Some(x) => Value::Str(x.into()),
                None => Value::Unknown,
            };
        }
        return Value::Unknown;
    }
    if text.starts_with("N'") || text.starts_with("n'") {
        return plain_string(&text[1..])
            .map(Value::Str)
            .unwrap_or(Value::Unknown);
    }
    if text.starts_with("E'") || text.starts_with("e'") {
        return e_string(&text[1..])
            .map(Value::Str)
            .unwrap_or(Value::Unknown);
    }
    if unicode_prefixed(text, '\'') {
        return unicode_string(text, false)
            .map(Value::Str)
            .unwrap_or(Value::Unknown);
    }
    if text.starts_with("'") {
        return plain_string(text).map(Value::Str).unwrap_or(Value::Unknown);
    }
    if text.starts_with("X'")
        || text.starts_with("x'")
        || text.starts_with("B'")
        || text.starts_with("b'")
    {
        return Value::Unknown;
    }
    let mut n = text.trim_start();
    let sign = if n.starts_with('-') || n.starts_with('+') {
        let x = n.as_bytes()[0] as char;
        n = n[1..].trim_start();
        Some(x)
    } else {
        None
    };
    let signed = |v: Value| match sign {
        Some('-') => match v {
            Value::Int(x) => fits_i64(Value::Int(-x)),
            Value::Float(x) => Value::Float(-x),
            _ => v,
        },
        Some('+') | None => match v {
            Value::Int(_) => fits_i64(v),
            _ => v,
        },
        _ => Value::Unknown,
    };
    if let Some((prefix, radix)) = [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ]
    .iter()
    .find(|(p, _)| n.starts_with(p))
    {
        let d = &n[prefix.len()..];
        if digits_ok(d, *radix) {
            return signed(integer(d, *radix).unwrap_or(Value::Unknown));
        }
        return Value::Unknown;
    }
    if !n.contains('.') && !n.contains('e') && !n.contains('E') && digits_ok(n, 10) {
        return signed(integer(n, 10).unwrap_or(Value::Unknown));
    }
    // `n` is the unsigned magnitude: the sign, read above, applies once.
    if n.starts_with(['-', '+']) {
        return Value::Unknown;
    }
    float_literal(n).map(signed).unwrap_or(Value::Unknown)
}

pub fn equals(left: &Value, right: &Value) -> Option<bool> {
    if matches!(left, Value::None)
        || matches!(right, Value::None)
        || !left.is_known()
        || !right.is_known()
    {
        return None;
    }
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => Some(a == b),
        (Value::Int(a), Value::Float(b)) => int_float_order(a, *b).map(|x| x == Ordering::Equal),
        (Value::Float(a), Value::Int(b)) => int_float_order(b, *a).map(|x| x == Ordering::Equal),
        (Value::Float(a), Value::Float(b)) => Some(a == b),
        (Value::Bool(a), Value::Bool(b)) => Some(a == b),
        (Value::Str(a), Value::Str(b)) => {
            if a == b {
                Some(true)
            } else if a.is_ascii()
                && b.is_ascii()
                && a.trim_end_matches(' ')
                    .eq_ignore_ascii_case(b.trim_end_matches(' '))
            {
                Some(false)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The order of two known values where PostgreSQL and SQLite agree: numbers by value, booleans
/// `false < true`. Strings order by a collation the file does not name, NULL against nothing, and
/// values of different types not at all (PostgreSQL refuses them): unknown.
pub(crate) fn order(left: &Value, right: &Value) -> Option<Option<Ordering>> {
    match (left, right) {
        (Value::Int(left), Value::Int(right)) => Some(Some(left.cmp(right))),
        (Value::Int(left), Value::Float(right)) => Some(int_float_order(left, *right)),
        (Value::Float(left), Value::Int(right)) => {
            Some(int_float_order(right, *left).map(Ordering::reverse))
        }
        (Value::Float(left), Value::Float(right)) => Some(left.partial_cmp(right)),
        (Value::Bool(left), Value::Bool(right)) => Some(Some(left.cmp(right))),
        _ => None,
    }
}

/// Whether a value is true where it is a condition: a boolean; NULL and anything else are not
/// known (NULL is neither, PostgreSQL takes no number as a condition, SQLite takes any).
pub(crate) fn truthy(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        _ => None,
    }
}

/// A value cast to text, where PostgreSQL and SQLite agree: an integer's digits, a string itself.
/// A boolean is `true` or `1`, a number with a fraction is formatted apart: unknown.
pub(crate) fn to_str(value: &Value) -> Option<String> {
    match value {
        Value::Int(value) => Some(value.to_string()),
        Value::Str(value) => Some(value.clone()),
        _ => None,
    }
}

pub fn name(text: &str) -> Option<String> {
    let mut out = if unicode_prefixed(text, '"') {
        unicode_string(text, true)?
    } else if text.starts_with('"') {
        let (x, n) = quoted_body(text, '"')?;
        if n != text.len() {
            return None;
        }
        x
    } else if text.starts_with('`') || text.starts_with('[') {
        return None;
    } else {
        text.chars().map(|c| c.to_ascii_lowercase()).collect()
    };
    if out.is_empty() {
        return None;
    }
    while out.len() > 63 {
        out.pop();
    }
    Some(out)
}

/// An identifier's text as PostgreSQL names it (`name`), else as written: a MySQL or SQL Server
/// quoted name, which PostgreSQL does not read. A name already folded is borrowed.
pub fn identifier(text: Cow<'_, str>) -> Cow<'_, str> {
    let folded = text.len() <= 63
        && !text.starts_with(['"', '`', '['])
        && !unicode_prefixed(&text, '"')
        && !text.bytes().any(|byte| byte.is_ascii_uppercase());
    if folded {
        return text;
    }
    match name(&text) {
        Some(name) => Cow::Owned(name),
        None => text,
    }
}

pub fn decode<D: ast_grep_core::Doc>(node: &ast_grep_core::Node<'_, D>) -> Value {
    match node.kind().as_ref() {
        "literal" => literal(&node.text()),
        "parenthesized_expression" | "term" => {
            let mut c = node.named();
            match (c.next(), c.next()) {
                (Some(x), None) => decode(&x),
                _ => Value::Unknown,
            }
        }
        _ => Value::Unknown,
    }
}

pub fn node_name<D: ast_grep_core::Doc>(
    node: &ast_grep_core::Node<'_, D>,
) -> Option<String> {
    (node.kind() == "identifier")
        .then(|| name(&node.text()))
        .flatten()
}
// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)
